//! The lights routes (Scenes plan, Task 6): `GET /api/v1/lights` and
//! `PATCH /api/v1/lights/{id}`.
//!
//! A light is the spine's name for a run of LEDs the owner treats as one thing; the hardware map
//! is `lights.json` ([`canvas_proto::scene::LightsFile`]). The list answers every light the file
//! holds with what the editor shows beside it: the LEDs it has now and whether its device is
//! here. A change to a light is saved at once, like every write in the scene model.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{LightPatch, LightView, LightsResponse};
use canvas_proto::device::{DeviceInfo, ZoneInfo};
use canvas_proto::scene::{Light, GROUP_NAME_MAX};
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// The biggest light patch the route reads. A patch carries a name of at most [`GROUP_NAME_MAX`]
/// characters, an arrangement and a flag, so this is far more than it needs.
const PATCH_BODY_LIMIT: usize = 16 * 1024;

/// `GET /api/v1/lights`.
///
/// Every light of `lights.json`, in file order, with the LED count it has now and whether its
/// device is connected. A light whose device is absent is listed (it keeps its place in the
/// scenes) with no LEDs.
pub async fn list_lights(State(app): State<Arc<App>>) -> Json<LightsResponse> {
    let devices = app.devices.read().await.clone();
    let lights = app.store.read().await.lights.lights.clone();
    Json(LightsResponse { lights: lights.iter().map(|light| view(light, &devices)).collect() })
}

/// What the editor shows beside a light.
fn view(light: &Light, devices: &[DeviceInfo]) -> LightView {
    let device = devices.iter().find(|device| device.id == light.device_id);
    let zone = device.and_then(|device| {
        device.zones.iter().find(|zone| zone.index == light.zone_index)
    });
    LightView {
        light: light.clone(),
        led_count: zone.map(|zone| led_count(light, zone)).unwrap_or(0),
        connected: device.is_some_and(|device| device.connected),
        label: label(light, device, zone),
    }
}

/// The LEDs the light has now: its run clamped to the zone, or the zone's whole count.
fn led_count(light: &Light, zone: &ZoneInfo) -> u32 {
    match light.leds {
        None => zone.led_count,
        Some(slice) => {
            let start = slice.start.min(zone.led_count);
            slice.count.min(zone.led_count - start)
        }
    }
}

/// The owner's name, else the device and zone name. A device that is here but has no such zone
/// still names the light, by the device's own name. With no device to name, the light's id, which
/// is the only thing that still identifies it.
fn label(light: &Light, device: Option<&DeviceInfo>, zone: Option<&ZoneInfo>) -> String {
    if let Some(name) = light.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_string();
    }
    match (device, zone) {
        (Some(device), Some(zone)) => format!("{} {}", device.name, zone.name),
        (Some(device), None) => device.name.clone(),
        _ => light.id.clone(),
    }
}

/// `PATCH /api/v1/lights/{id}`.
///
/// `name` left out is unchanged, `name: null` clears it, a string renames the light (trimmed; an
/// empty or over-long one is a 422). `arrangement` and `reversed` are set when given. The change
/// is saved before it is answered, so a refused save changes nothing and answers 503. A patch
/// that changes nothing is answered without writing the file.
pub async fn patch_light(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
) -> ApiResult<Json<LightView>> {
    // The body is read as bytes, so a body that is not a patch is answered with the API's own
    // error shape (an unknown arrangement, say), not axum's plain text.
    let bytes = crate::api::limited_body(body, PATCH_BODY_LIMIT)?;
    let patch: LightPatch = serde_json::from_slice(&bytes)
        .map_err(|e| ApiFailure::validation(format!("the body is not a light patch: {e}")))?;

    let devices = app.devices.read().await.clone();
    // A rename changes nothing the engine plays; an arrangement or a direction does.
    let replan = patch.arrangement.is_some() || patch.reversed.is_some();
    let (light, saved) = {
        let mut store = app.store.write().await;
        let Some(index) = store.lights.lights.iter().position(|light| light.id == id) else {
            return Err(ApiFailure::not_found(format!("no light with the id {id}")));
        };
        let name = match patch.name {
            None => None,
            Some(None) => Some(None),
            Some(Some(raw)) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    return Err(ApiFailure::validation(
                        "a light's name cannot be empty; clear it with null instead",
                    ));
                }
                if trimmed.chars().count() > GROUP_NAME_MAX {
                    return Err(ApiFailure::validation(format!(
                        "a light's name is at most {GROUP_NAME_MAX} characters"
                    )));
                }
                Some(Some(trimmed.to_string()))
            }
        };
        // Kept so a failed save leaves memory as it was: never ahead of the disk and the engine.
        let before = store.lights.lights[index].clone();
        {
            let light = &mut store.lights.lights[index];
            if let Some(name) = name {
                light.name = name;
            }
            if let Some(arrangement) = patch.arrangement {
                light.arrangement = arrangement;
            }
            if let Some(reversed) = patch.reversed {
                light.reversed = reversed;
            }
        }
        // A patch that changes nothing leaves the file alone: the light on disk is still right.
        let saved = store.lights.lights[index] != before;
        if saved {
            if let Err(e) = store.save_lights() {
                store.lights.lights[index] = before;
                return Err(ApiFailure::service_unavailable(format!(
                    "the light was not changed: {e}"
                )));
            }
        }
        (store.lights.lights[index].clone(), saved)
    };

    // The engine follows: a new arrangement or direction is a new plan for the light's group.
    if replan && saved {
        crate::scenes::apply_active(&app, false).await;
    } else {
        app.scene.notify();
    }

    if saved {
        tracing::info!(light = %light.id, "a light was changed");
    }
    Ok(Json(view(&light, &devices)))
}
