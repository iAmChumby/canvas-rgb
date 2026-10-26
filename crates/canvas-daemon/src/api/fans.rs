//! The fan routes. The daemon validates; the service does the work.
//!
//! Every write is validated with `canvas_proto::fans::validate_mode` or
//! `validate_profile` FIRST, so a value below the floor is refused with 422
//! `validation` before the daemon says anything to the service.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::FanProfileName;
use canvas_proto::fans::{validate_mode, validate_profile, FanMode, FanProfile, FansStatus};
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

    crate::pipe_client::send(&app, &canvas_proto::pipe::PipeRequest::SetHeaderMode {
        header: header.clone(),
        mode: mode.clone(),
    })
    .await
    .map_err(|e| ApiFailure::service_unavailable(format!("the fan service is not reachable: {e}")))?;

    tracing::info!(header = %header, mode = ?mode, "the fan header mode changed");
    Ok(Json(serde_json::json!({ "ok": true, "header": header, "mode": mode })))
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
    crate::pipe_client::send(&app, &canvas_proto::pipe::PipeRequest::SetProfile {
        name: request.name.clone(),
    })
    .await
    .map_err(|e| ApiFailure::service_unavailable(format!("the fan service is not reachable: {e}")))?;

    // Keep the daemon's copy of the fans file in step, so an export while the service is
    // down still shows the right profile.
    {
        let home = app.store.read().await.home.clone();
        let mut guard = app.fans_last.write().await;
        if let Some(fans) = guard.as_mut() {
            fans.active_profile = request.name.clone();
            let _ = crate::storage::save_fans_last(&home, fans);
        }
    }

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
    let response = crate::pipe_client::send(
        &app,
        &canvas_proto::pipe::PipeRequest::DeleteProfile { name: name.clone() },
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

    // Keep the daemon's copy of the fans file in step, as `put_profile` does.
    {
        let home = app.store.read().await.home.clone();
        let mut guard = app.fans_last.write().await;
        if let Some(fans) = guard.as_mut() {
            fans.profiles.retain(|p| p.name != name);
            let _ = crate::storage::save_fans_last(&home, fans);
        }
    }

    tracing::info!(profile = %name, "the fan profile was deleted");
    Ok(Json(serde_json::json!({ "ok": true, "name": name })))
}

/// Maps a service error code to the API's status: `conflict` 409, `not_found` 404,
/// `service_unavailable` 503. Any other code is an internal error.
fn service_failure(code: &str, message: String) -> ApiFailure {
    use canvas_proto::api::error_codes;
    match code {
        error_codes::CONFLICT => ApiFailure::new(
            axum::http::StatusCode::CONFLICT,
            error_codes::CONFLICT,
            message,
        ),
        error_codes::NOT_FOUND => ApiFailure::not_found(message),
        error_codes::SERVICE_UNAVAILABLE => ApiFailure::service_unavailable(message),
        _ => ApiFailure::internal(message),
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

    crate::pipe_client::send(&app, &canvas_proto::pipe::PipeRequest::PutProfile {
        profile: profile.clone(),
    })
    .await
    .map_err(|e| ApiFailure::service_unavailable(format!("the fan service is not reachable: {e}")))?;

    // Mirror the profiles into the daemon's copy of the fans file and push the whole
    // config, so the service and the daemon agree.
    let (home, fans) = {
        let home = app.store.read().await.home.clone();
        let mut guard = app.fans_last.write().await;
        let mut fans = guard.clone().unwrap_or_default();
        fans.floor_percent = floor;
        match fans.profiles.iter_mut().find(|p| p.name == profile.name) {
            Some(existing) => *existing = profile.clone(),
            None => fans.profiles.push(profile.clone()),
        }
        let _ = crate::storage::save_fans_last(&home, &fans);
        *guard = Some(fans.clone());
        (home, fans)
    };
    let _ = home;
    crate::pipe_client::send(&app, &canvas_proto::pipe::PipeRequest::PutFanConfig { config: fans })
        .await
        .map_err(|e| {
            ApiFailure::service_unavailable(format!("the fan service is not reachable: {e}"))
        })?;

    tracing::info!(profile = %profile.name, "the fan profile was saved");
    Ok(Json(profile))
}
