//! The numeric display routes. A config is validated FIRST, saved to `displays.json`, and then the
//! display task starts writing it. A DELETE removes the config, stops the writes and closes the
//! device. A display with no saved config is never opened (see `crate::displays`).

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{DisplayStatus, SetDisplayValue};
use canvas_proto::device::DeviceId;
use canvas_proto::display::{DisplayConfig, DisplayInfo, DisplaySource};
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::displays::{save, validate, DisplaysFile};
use crate::state::App;

/// What a configured display that the enumeration has not found reports.
fn not_found_info(config: &DisplayConfig) -> DisplayInfo {
    DisplayInfo {
        id: config.device.clone(),
        name: "display not found".into(),
        digits: 0,
        max_decimals: config.decimals,
        modes: Vec::new(),
        connected: false,
    }
}

/// `GET /api/v1/displays`. Every saved config, with what the display shows now.
pub async fn list_displays(State(app): State<Arc<App>>) -> Json<Vec<DisplayStatus>> {
    let configs = app.store.read().await.displays.displays.clone();
    let runtime = app.displays.runtime.read().await;
    let list = configs
        .into_iter()
        .map(|config| {
            let info = match runtime.found.iter().find(|f| f.id == config.device) {
                Some(found) => DisplayInfo { connected: runtime.connected, ..found.clone() },
                None => not_found_info(&config),
            };
            // A blank is reported as `shown: None`, not as a value with no number.
            let shown = runtime.shown.get(&config.device).filter(|v| v.value.is_some()).copied();
            DisplayStatus { info, config, shown }
        })
        .collect();
    Json(list)
}

/// `PUT /api/v1/displays/{id}`. The body is a [`DisplayConfig`] for that device.
///
/// 422 `validation` for a config that does not match the path, a decimal count other than 0 or 1,
/// or a fan RPM with decimals. Saving is atomic; the task starts writing at its next pass.
pub async fn put_display(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(config): Json<DisplayConfig>,
) -> ApiResult<Json<DisplayConfig>> {
    validate(&id, &config).map_err(ApiFailure::validation)?;
    let changed = {
        let mut store = app.store.write().await;
        let mut next: DisplaysFile = store.displays.clone();
        let changed = match next.displays.iter_mut().find(|c| c.device == config.device) {
            Some(existing) if *existing == config => false,
            Some(existing) => {
                *existing = config.clone();
                true
            }
            None => {
                next.displays.push(config.clone());
                true
            }
        };
        if changed {
            save(&store.home, &next)
                .map_err(|e| ApiFailure::internal(format!("could not save the display config: {e}")))?;
            store.displays = next;
        }
        changed
    };
    // Only a real change wakes the task and drops its retry pacing. A repeated PUT of the same
    // config (a client that PUTs on a timer) must not reset a backoff in progress.
    if changed {
        app.displays.changed();
    }
    tracing::info!(device = %id, source = ?config.source, "the display config was saved");
    Ok(Json(config))
}

/// `DELETE /api/v1/displays/{id}`. Removes the saved config, stops writing and closes the device so
/// the vendor app can take it back. 404 `not_found` when the display has no saved config.
pub async fn delete_display(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let device = DeviceId(id.clone());
    {
        let mut store = app.store.write().await;
        let position = store
            .displays
            .displays
            .iter()
            .position(|c| c.device == device)
            .ok_or_else(|| ApiFailure::not_found(format!("no saved config for display {id}")))?;
        let mut next = store.displays.clone();
        next.displays.remove(position);
        save(&store.home, &next)
            .map_err(|e| ApiFailure::internal(format!("could not save the display config: {e}")))?;
        store.displays = next;
    }
    app.displays.changed();

    // The task holds the adapter for each enumerate, connect and write. Taking the lock here waits
    // out any write in flight; the config is already gone, so no later pass writes this display.
    // The runtime is cleaned up only after that wait, so a write that was in flight cannot put
    // `shown` back afterwards.
    let still_saved = app.store.read().await.displays.displays.iter().any(|c| c.device == device);
    let mut adapter = app.displays.adapter.lock().await;
    if !still_saved {
        if let Err(e) = adapter.disconnect().await {
            tracing::warn!(device = %id, error = %e, "the display did not close cleanly");
        }
    }
    {
        let mut runtime = app.displays.runtime.write().await;
        runtime.shown.remove(&device);
        runtime.api.remove(&device);
        runtime.refusing.remove(&device);
        if !still_saved {
            runtime.connected = false;
        }
    }
    drop(adapter);
    tracing::info!(device = %id, "the display config was removed");
    Ok(Json(serde_json::json!({ "ok": true, "device_id": id })))
}

/// `PUT /api/v1/displays/{id}/value`. The number a display with `DisplaySource::Api` shows.
///
/// 404 `not_found` when neither a saved config nor an enumerated display has that id. 422
/// `validation` when the display's saved source is not `api`, or the value is not a finite number.
/// `value: null` means "no value": the display keeps what it shows, because no blank packet is
/// sent until blanking is proven on hardware (see `crate::displays`).
pub async fn put_display_value(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<SetDisplayValue>,
) -> ApiResult<Json<SetDisplayValue>> {
    if body.value.is_some_and(|v| !v.is_finite()) {
        return Err(ApiFailure::validation("the value must be a finite number"));
    }
    let device = DeviceId(id.clone());
    let saved_source = app
        .store
        .read()
        .await
        .displays
        .displays
        .iter()
        .find(|c| c.device == device)
        .map(|c| c.source.clone());
    let mut runtime = app.displays.runtime.write().await;
    match saved_source {
        Some(DisplaySource::Api) => {}
        Some(_) => {
            return Err(ApiFailure::validation(format!(
                "display {id} is not set to the api source, so a value does not show"
            )))
        }
        None if runtime.found.iter().any(|f| f.id == device) => {}
        None => return Err(ApiFailure::not_found(format!("no display with the id {id}"))),
    }
    runtime.api.insert(device, body.value);
    Ok(Json(body))
}
