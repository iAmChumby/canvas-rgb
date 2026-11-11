//! The fan routes. The daemon validates; the service does the work.
//!
//! Every write is validated with `canvas_proto::fans::validate_mode` or
//! `validate_profile` FIRST, so a value below the floor is refused with 422
//! `validation` before the daemon says anything to the service.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{FanProfileName, PutFanHot};
use canvas_proto::config::FansFile;
use canvas_proto::fans::{
    hot_limit_status, validate_hot_limits, validate_mode, validate_profile, FanMode, FanProfile, FansStatus,
    HotKind,
};
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// The floor the daemon validates against. The service may report its own.
async fn configured_floor(app: &Arc<App>) -> u8 {
    app.fans
        .read()
        .await
        .as_ref()
        .map(|s| s.floor_percent)
        .unwrap_or(canvas_proto::fans::DEFAULT_FLOOR_PERCENT)
}

/// `GET /api/v1/fans`.
///
/// 503 `service_unavailable` when the service is down.
pub async fn get_fans(State(app): State<Arc<App>>) -> ApiResult<Json<FansStatus>> {
    let status = app
        .fans
        .read()
        .await
        .clone()
        .ok_or_else(|| {
            ApiFailure::service_unavailable("the fan service is not reachable")
        })?;
    Ok(Json(status))
}

/// `PATCH /api/v1/fans/headers/{h}`.
///
/// The body is a [`FanMode`]. The value is validated first: a percent below the floor
/// is refused with 422 and never reaches the service.
pub async fn patch_header(
    State(app): State<Arc<App>>,
    Path(header): Path<String>,
    Json(mode): Json<FanMode>,
) -> ApiResult<Json<serde_json::Value>> {
    let floor = configured_floor(&app).await;
    validate_mode(&header, &mode, floor)
        .map_err(|e| ApiFailure::validation(e.to_string()))?;

    // Issue #295 review: the same lock the whole-config pushes hold. A push reads the service's
    // config, then replaces it, so a mode set inside that window would be overwritten by the
    // push's older copy. Held across the send only: nothing here reads the daemon's own copy.
    // The send is bounded (#225), so a hung service never holds the lock for good.
    {
        let _fans = app.fans_last.write().await;
        let response = send_under_fans_lock(&app, &canvas_proto::pipe::PipeRequest::SetHeaderMode {
            header: header.clone(),
            mode: mode.clone(),
        })
        .await?;
        // Issue #225: the service's refusal is the answer (a pipe `Error` maps through
        // `service_failure`); only an `Ok` means the mode changed.
        expect_ok(response)?;
    }

    tracing::info!(header = %header, mode = ?mode, "the fan header mode changed");
    Ok(Json(serde_json::json!({ "ok": true, "header": header, "mode": mode })))
}

/// `PUT /api/v1/fans/headers/{h}/availability` (issue #146).
///
/// The owner's word on whether a header is on this board: `false` is "Not on my board",
/// `true` is "Show anyway" and `null` takes the word off, so the board table decides again.
/// The service knows the headers, so it refuses an unknown one; that refusal is a 422 here,
/// mapped through the same `service_failure` the other fan routes use.
///
/// The answer is the new fan status: the word changes which headers the page shows, and the
/// daemon's cached status is refreshed so the next read says the same. When the service
/// took the word but the follow-up FanStatus fails, the daemon does not claim failure: it
/// answers 200 with the cached status patched for that header, or 200 `{"ok":true}` when
/// nothing is cached (issue #146 review S2).
pub async fn put_header_availability(
    State(app): State<Arc<App>>,
    Path(header): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // Validate the header id in the daemon, as the other fans write route does: a
    // non-empty, bounded id, and a known one when the cached status has headers.
    if header.is_empty() || header.len() > 64 {
        return Err(ApiFailure::validation(format!("unknown header {header}")));
    }
    {
        let cached = app.fans.read().await.clone();
        if let Some(status) = cached.as_ref() {
            if !status.headers.is_empty() && !status.headers.iter().any(|h| h.id == header) {
                return Err(ApiFailure::validation(format!("unknown header {header}")));
            }
        }
    }
    // The body is strict: exactly `available`, a bool or null. A missing field, an
    // unknown field, or any other shape is a 422 before anything reaches the service.
    let available = match body {
        serde_json::Value::Object(ref map) if map.len() == 1 => match map.get("available") {
            Some(serde_json::Value::Bool(v)) => Some(*v),
            Some(serde_json::Value::Null) => None,
            _ => return Err(ApiFailure::validation("the body must be {\"available\": true, false, or null}".to_string())),
        },
        _ => return Err(ApiFailure::validation("the body must be {\"available\": true, false, or null}".to_string())),
    };
    let response = crate::pipe_client::send(
        &app,
        &canvas_proto::pipe::PipeRequest::SetHeaderAvailability {
            header: header.clone(),
            available,
        },
    )
    .await
    .map_err(|e| ApiFailure::service_unavailable(format!("the fan service is not reachable: {e}")))?;

    match response {
        canvas_proto::pipe::PipeResponse::Ok => {}
        canvas_proto::pipe::PipeResponse::Error { code, message } => {
            return Err(service_failure(&code, message));
        }
        _ => {
            return Err(ApiFailure::internal("the fan service gave an unexpected answer"));
        }
    }

    // The word changed what the service reports, so ask it again and keep that copy.
    match crate::pipe_client::send(&app, &canvas_proto::pipe::PipeRequest::FanStatus).await {
        Ok(canvas_proto::pipe::PipeResponse::FanStatus { status }) => {
            *app.fans.write().await = Some(status.clone());
            tracing::info!(header = %header, available = ?available, "the header availability changed");
            Ok(Json(serde_json::to_value(&status).unwrap_or(serde_json::json!({ "ok": true }))))
        }
        _ => {
            // The word is already taken and saved in the service: do not answer 503.
            tracing::warn!(header = %header, available = ?available, "the fan status could not be read after the header availability changed; answering with the cached status patched for this header");
            let cached = app.fans.read().await.clone();
            match cached {
                Some(mut status) => {
                    if let Some(h) = status.headers.iter_mut().find(|h| h.id == header) {
                        if let Some(v) = available {
                            h.available = v;
                        }
                        h.override_available = available;
                    }
                    *app.fans.write().await = Some(status.clone());
                    Ok(Json(serde_json::to_value(&status).unwrap_or(serde_json::json!({ "ok": true }))))
                }
                None => Ok(Json(serde_json::json!({ "ok": true }))),
            }
        }
    }
}

/// `GET /api/v1/fans/profile`.
pub async fn get_profile(State(app): State<Arc<App>>) -> ApiResult<Json<FanProfileName>> {
    let status = app
        .fans
        .read()
        .await
        .as_ref()
        .map(|s| s.active_profile.clone())
        .ok_or_else(|| ApiFailure::service_unavailable("the fan service is not reachable"))?;
    Ok(Json(FanProfileName { name: status }))
}

/// `PATCH /api/v1/fans/profile`.
///
/// Switches to another profile. This is what `agentmode on` and `off` use.
pub async fn patch_profile(
    State(app): State<Arc<App>>,
    Json(request): Json<FanProfileName>,
) -> ApiResult<Json<FanProfileName>> {
    // Issue #295 review: the profile lives in the service's config, and this is the lock the
    // whole-config pushes hold across their read-modify-push. Held from before the send, so a
    // push that started first cannot put the old profile back over this one. Bounded (#225).
    let home = app.store.read().await.home.clone();
    let mut guard = app.fans_last.write().await;
    let response = send_under_fans_lock(&app, &canvas_proto::pipe::PipeRequest::SetProfile {
        name: request.name.clone(),
    })
    .await?;
    // Issue #225: a refusal (`not_found` for a profile the service does not have) is the
    // answer, and the daemon's copy changes only after the service said `Ok`.
    expect_ok(response)?;

    // Keep the daemon's copy of the fans file in step, so an export while the service is
    // down still shows the right profile.
    if let Some(fans) = guard.as_mut() {
        fans.active_profile = request.name.clone();
        crate::logging::warn_if_err("could not save the daemon's copy of the fans file", crate::storage::save_fans_last(&home, fans));
    }
    drop(guard);

    tracing::info!(profile = %request.name, "the fan profile changed");
    Ok(Json(request))
}

/// `DELETE /api/v1/fans/profiles/{name}`.
///
/// The service refuses the active profile and the last remaining one with `conflict`, which
/// is 409 here. An unknown name is 404 and a service that is down is 503.
pub async fn delete_profile(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    // The fans lock is held from before the send (#295 review): a whole-config push that read its
    // base before this delete would otherwise put the deleted profile back. Bounded (#225).
    {
        let home = app.store.read().await.home.clone();
        let mut guard = app.fans_last.write().await;
        let response = send_under_fans_lock(
            &app,
            &canvas_proto::pipe::PipeRequest::DeleteProfile { name: name.clone() },
        )
        .await?;
        expect_ok(response)?;
        // Keep the daemon's copy of the fans file in step, as `put_profile` does.
        if let Some(fans) = guard.as_mut() {
            fans.profiles.retain(|p| p.name != name);
            crate::logging::warn_if_err("could not save the daemon's copy of the fans file", crate::storage::save_fans_last(&home, fans));
        }
    }

    tracing::info!(profile = %name, "the fan profile was deleted");
    Ok(Json(serde_json::json!({ "ok": true, "name": name })))
}

/// Maps a service error code to the API's status: `conflict` 409, `not_found` 404,
/// `service_unavailable` 503, `validation` 422. Any other code is an internal error.
pub(crate) fn service_failure(code: &str, message: String) -> ApiFailure {
    use canvas_proto::api::error_codes;
    match code {
        error_codes::CONFLICT => ApiFailure::new(
            axum::http::StatusCode::CONFLICT,
            error_codes::CONFLICT,
            message,
        ),
        error_codes::NOT_FOUND => ApiFailure::not_found(message),
        error_codes::VALIDATION => ApiFailure::validation(message),
        error_codes::SERVICE_UNAVAILABLE => ApiFailure::service_unavailable(message),
        _ => ApiFailure::internal(message),
    }
}

/// The config a whole-config push (`PutFanConfig`) starts from (issue #295).
///
/// The push REPLACES the service's config, and the service keeps header modes in its own profile
/// (`SetHeaderMode` never reaches the daemon's copy). So the base is the config the SERVICE holds,
/// asked for over the pipe (`GetFanConfig`): a hot-limit PUT then carries the modes, profiles,
/// boot colour and floor the service has. The daemon never reads `%ProgramData%\canvas-rgb\fans.toml`
/// itself (issue #295 review): that tree's owner and reparse-point checks belong to the service, so
/// a file read there could be a planted one pushed straight to the SYSTEM service.
///
/// A service that does not answer, or answers with something else, refuses the write with 503 and
/// nothing is sent: a default config pushed over the service's own would put every header back on
/// BIOS. The answer's own `floor_percent` is the service's floor, and both push routes keep it: the
/// daemon's cached status floor can be older, and taking it would lower a floor the service raised.
async fn base_fans_config(app: &Arc<App>) -> Result<FansFile, ApiFailure> {
    let answer = tokio::time::timeout(
        app.fans_pipe_limit,
        crate::pipe_client::send(app, &canvas_proto::pipe::PipeRequest::GetFanConfig),
    )
    .await;
    let response = match answer {
        Ok(Ok(response)) => response,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "the service did not answer for its fans config; nothing was sent");
            return Err(ApiFailure::service_unavailable(
                "the service's fan config could not be read, so nothing was changed",
            ));
        }
        Err(_) => {
            tracing::warn!(limit = ?app.fans_pipe_limit, "the service did not answer for its fans config in time; nothing was sent");
            return Err(ApiFailure::service_unavailable(
                "the service's fan config could not be read, so nothing was changed",
            ));
        }
    };
    let config = match response {
        canvas_proto::pipe::PipeResponse::FanConfig { config } => config,
        other => {
            tracing::warn!(answer = ?other, "the service gave no fans config; nothing was sent");
            return Err(ApiFailure::service_unavailable(
                "the service's fan config could not be read, so nothing was changed",
            ));
        }
    };
    if config.schema_version != canvas_proto::config::FANS_SCHEMA {
        tracing::warn!(schema_version = config.schema_version, "the service's fan config is a version this app does not know; nothing was sent");
        return Err(ApiFailure::service_unavailable(
            "the service's fan config is a version this app does not know, so nothing was changed",
        ));
    }
    Ok(config)
}

/// Sends one request to the service while the caller holds the `fans_last` lock, and gives up
/// after `App::fans_pipe_limit` with 503 (issue #225). The pipe client has no timeout of its
/// own, so without this bound a hung service would hold the lock and block config export and
/// every other fans writer. Dropping the future on timeout ends the exchange; the caller's
/// early return then releases the lock.
async fn send_under_fans_lock(
    app: &Arc<App>,
    request: &canvas_proto::pipe::PipeRequest,
) -> Result<canvas_proto::pipe::PipeResponse, ApiFailure> {
    match tokio::time::timeout(app.fans_pipe_limit, crate::pipe_client::send(app, request)).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(e)) => Err(ApiFailure::service_unavailable(format!("the fan service is not reachable: {e}"))),
        Err(_) => {
            tracing::warn!(limit = ?app.fans_pipe_limit, "the fan service did not answer in time; the fans lock is released");
            Err(ApiFailure::service_unavailable("the fan service did not answer in time"))
        }
    }
}

/// Turns the service's answer to a write into `Ok(())` or the matching failure: a pipe
/// `Error` goes through [`service_failure`], and anything other than `Ok` is an internal
/// error. Every fan route that sends a request and needs no payload back reads its answer
/// through this (issue #225).
pub(crate) fn expect_ok(response: canvas_proto::pipe::PipeResponse) -> Result<(), ApiFailure> {
    match response {
        canvas_proto::pipe::PipeResponse::Ok => Ok(()),
        canvas_proto::pipe::PipeResponse::Error { code, message } => Err(service_failure(&code, message)),
        _ => Err(ApiFailure::internal("the fan service gave an unexpected answer")),
    }

}

/// `PUT /api/v1/fans/profiles/{name}`.
///
/// Creates or replaces a profile. The profile is validated first.
pub async fn put_profile(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
    Json(profile): Json<FanProfile>,
) -> ApiResult<Json<FanProfile>> {
    if profile.name != name {
        return Err(ApiFailure::validation(format!(
            "the profile name in the body ({}) does not match the path ({name})",
            profile.name
        )));
    }
    let floor = configured_floor(&app).await;
    validate_profile(&profile, floor).map_err(|e| ApiFailure::validation(e.to_string()))?;

    // One fans-config lock (`fans_last`) is held across the whole read-modify-send-send-save:
    // the base, `PutProfile`, the whole-config push that mirrors it, then the daemon's copy. A
    // concurrent hot-limit PUT, profile PUT, profile delete or header/profile PATCH waits, so no
    // push lands between two sends and no two writes lose each other's change (#225, #295). Lock
    // order: the store's home first, then `fans_last` (as the other fans routes do).
    let home = app.store.read().await.home.clone();
    let mut guard = app.fans_last.write().await;
    // The base comes BEFORE the first send (issue #295 review): when the service cannot give its
    // config the write is refused with 503, instead of saving the profile and then answering that
    // nothing was changed. Its `floor_percent` is the service's own, so it is kept as is.
    let mut next = base_fans_config(&app).await?;

    let put_response = send_under_fans_lock(&app, &canvas_proto::pipe::PipeRequest::PutProfile {
        profile: profile.clone(),
    })
    .await?;
    expect_ok(put_response)?;

    // Mirror the profile into the base and push the whole config, so the service and the daemon
    // agree.
    match next.profiles.iter_mut().find(|p| p.name == profile.name) {
        Some(existing) => *existing = profile.clone(),
        None => next.profiles.push(profile.clone()),
    }
    // Send first: the daemon's copy is updated below only after the service said Ok.
    let response = send_under_fans_lock(&app, &canvas_proto::pipe::PipeRequest::PutFanConfig { config: next.clone() }).await?;
    expect_ok(response)?;
    crate::logging::warn_if_err("could not save the daemon's copy of the fans file", crate::storage::save_fans_last(&home, &next));
    *guard = Some(next);

    tracing::info!(profile = %profile.name, "the fan profile was saved");
    Ok(Json(profile))
}

/// `PUT /api/v1/fans/hot` (issue #130).
///
/// The user's "don't exceed" temperatures, one per kind of source. Validated first
/// against the rated maxima in the last status the service reported, so a value past
/// the ceiling gap is refused with 422 before anything reaches the service. The
/// service validates again on receipt.
pub async fn put_fan_hot(
    State(app): State<Arc<App>>,
    Json(limits): Json<PutFanHot>,
) -> ApiResult<Json<FansStatus>> {
    let status = app.fans.read().await.clone();
    let rated_of = |s: &FansStatus, kind: HotKind| {
        s.hot.iter().find(|h| h.kind == kind).and_then(|h| h.rated_max_c)
    };
    let (rated_cpu, rated_gpu) = match &status {
        Some(s) => (rated_of(s, HotKind::Cpu), rated_of(s, HotKind::Gpu)),
        None => (None, None),
    };
    validate_hot_limits(&limits, rated_cpu, rated_gpu)
        .map_err(|e| ApiFailure::validation(e.to_string()))?;

    // Mirror the limits into the daemon's copy of the fans file and push the whole
    // config, so the service and the daemon agree (as `put_profile` does). One
    // fans-config lock (`fans_last`) is held across the whole read-modify-send-save,
    // so two PUTs cannot lose each other's change. Send first: the daemon's copy is
    // saved only after the service said Ok; a refusal (a pipe `Error`, like
    // `delete_profile` maps) saves nothing.
    {
        let home = app.store.read().await.home.clone();
        let mut guard = app.fans_last.write().await;
        // The base is the config the service holds (`GetFanConfig`), and its `floor_percent` is the
        // service's own floor: the push keeps it rather than lowering it to the daemon's cached copy.
        let mut next = base_fans_config(&app).await?;
        next.hot_limits = limits;
        let response = send_under_fans_lock(&app, &canvas_proto::pipe::PipeRequest::PutFanConfig { config: next.clone() }).await?;
        expect_ok(response)?;
        crate::logging::warn_if_err("could not save the daemon's copy of the fans file", crate::storage::save_fans_last(&home, &next));
        *guard = Some(next.clone());
    }

    tracing::info!(?limits, "the fan hot limits changed");

    // Answer the new status: the last one the service reported, with its hot rows
    // recomputed for the limits just saved. With no status cached, ask the service.
    let mut new_status = match status {
        Some(status) => status,
        None => match crate::pipe_client::send(&app, &canvas_proto::pipe::PipeRequest::FanStatus).await {
            Ok(canvas_proto::pipe::PipeResponse::FanStatus { status }) => status,
            _ => return Err(ApiFailure::service_unavailable("the fan service is not reachable")),
        },
    };
    new_status.hot = [HotKind::Cpu, HotKind::Gpu, HotKind::Board]
        .iter()
        .map(|kind| {
            let rated = match kind {
                HotKind::Cpu => rated_cpu,
                HotKind::Gpu => rated_gpu,
                HotKind::Board => None,
            };
            hot_limit_status(*kind, &limits, rated)
        })
        .collect();
    Ok(Json(new_status))
}
