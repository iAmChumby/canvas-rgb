//! The lights routes (Scenes plan, Task 6): `GET /api/v1/lights`,
//! `PATCH /api/v1/lights/{id}` and `DELETE /api/v1/lights/{id}` (#414).
//!
//! A light is the spine's name for a run of LEDs the owner treats as one thing; the hardware map
//! is `lights.json` ([`canvas_proto::scene::LightsFile`]). The list answers every light the file
//! holds with what the editor shows beside it: the LEDs it has now and whether its device is
//! here. A change to a light is saved at once, like every write in the scene model.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{EmptiedGroup, LightDeleted, LightPatch, LightView, LightsResponse};
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
    let leds = zone.map(|zone| led_count(light, zone)).unwrap_or(0);
    // A zone that shrank below the owner's first LED drops it: there is no notch to show and the
    // light plays from LED 0. The stored value is left alone (a resize back restores it); the
    // engine's own helper applies the same rule at play time (#468).
    let mut shown = light.clone();
    if shown.first_led.is_some_and(|first| first >= leds) {
        shown.first_led = None;
    }
    LightView {
        light: shown,
        led_count: leds,
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
/// empty or over-long one is a 422). `arrangement` and `reversed` are set when given. `first_led`
/// is set when given: `0` clears it (LED 0 starts the run, as before #468) unless the light runs
/// back, where a stored 0 keeps LED 0 first and a missing one means the old flip; a value at or past
/// the light's current LED count is a 422. The change is saved before it is answered, so a refused
/// save changes nothing and answers 503. A patch that changes nothing is answered without writing
/// the file.
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
    // A rename changes nothing the engine plays; an arrangement, a direction or a first LED does.
    let replan = patch.arrangement.is_some() || patch.reversed.is_some() || patch.first_led.is_some();
    let (index, light, saved) = {
        let store = app.store.read().await;
        let Some(index) = store.lights.lights.iter().position(|light| light.id == id) else {
            return Err(ApiFailure::not_found(format!("no light with the id {id}")));
        };
        // A first LED is a physical LED of this light, so it must fit the count the device has
        // now. 0 resets it to None. Validate before any field is changed so a refused patch
        // leaves memory and lights.json as they were (#468).
        // A first LED of 0 clears it, unless the light runs back: a reversed light with no first
        // LED plays the old full flip (position 0 on its last LED), so LED 0 as the first LED of a
        // reversed run has to be stored (#468).
        let reversed_after = patch.reversed.unwrap_or(store.lights.lights[index].reversed);
        let first_led = match patch.first_led {
            None => None,
            Some(0) if !reversed_after => Some(None),
            Some(first) => {
                let current = &store.lights.lights[index];
                let count = devices
                    .iter()
                    .find(|device| device.id == current.device_id)
                    .and_then(|device| device.zones.iter().find(|zone| zone.index == current.zone_index))
                    .map(|zone| led_count(current, zone))
                    .unwrap_or(0);
                if first >= count {
                    return Err(ApiFailure::validation(format!(
                        "a light's first LED must be below its {count} LEDs"
                    )));
                }
                Some(Some(first))
            }
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
        let before = store.lights.lights[index].clone();
        let mut changed = before.clone();
        if let Some(name) = name {
            changed.name = name;
        }
        if let Some(arrangement) = patch.arrangement {
            changed.arrangement = arrangement;
        }
        if let Some(reversed) = patch.reversed {
            changed.reversed = reversed;
        }
        if let Some(first_led) = first_led {
            changed.first_led = first_led;
        }
        // A patch that changes nothing leaves the file alone: the light on disk is still right.
        let saved = changed != before;
        (index, changed, saved)
    };
    if saved {
        let persisted = light.clone();
        crate::store_io::store_save_rolled(
            &app,
            |store| {
                let old = std::mem::replace(&mut store.lights.lights[index], persisted);
                Ok(((), old))
            },
            |store| store.save_lights(),
            |old, store, e, _| {
                // A failed save leaves memory as it was: never ahead of the disk and the engine.
                store.lights.lights[index] = old;
                ApiFailure::service_unavailable(format!("the light was not changed: {e}"))
            },
        )
        .await?;
    }

    // The engine follows: a new arrangement or direction is a new plan for the light's group.
    if replan && saved {
        // The scene order, then the draft gate: the order every route takes.
        let lock = app.scene.lock().await;
        let _gate = app.draft.gate().lock().await;
        // A takeover (a draft, the onboarding greeting) holds the lights: the scene is not put
        // over it. The change is saved, and it lands when the takeover ends, because its restore
        // applies the active scene.
        if !app.scene.is_taken_over() {
            crate::scenes::apply_with(&app, &lock, false).await;
        }
    } else {
        app.scene.notify();
    }

    if saved {
        tracing::info!(light = %light.id, "a light was changed");
    }
    Ok(Json(view(&light, &devices)))
}

/// `DELETE /api/v1/lights/{id}` (#414).
///
/// Removes a light the daemon never drives — a leftover of an old `lights.json` a device's owner
/// cannot otherwise remove. A light on a driven device is a 409: those go through chain setup.
/// Before anything is written a snapshot of `lights.json` and `scenes.json` is taken; a failure
/// there answers 503 and changes nothing. Then the light leaves `lights.json` and every group, the
/// scenes repair drops a group it left empty, both files are saved, and the answer names every
/// group that went with it. A failed save restores the in-memory files and answers 503. As with a
/// patch that changes the arrangement, the scene is replanned and published so the editor sees it.
pub async fn delete_light(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<LightDeleted>> {
    // The scene order, then the draft gate, then the store: the order every route that writes a
    // scene takes (api/scenes.rs, api/chains.rs). The scene order is held across the whole write
    // below, so a scene PUT that validated against this light cannot write it back after it is
    // gone (#497). This is the same lock the apply at the end uses, so it is taken once here.
    let lock = app.scene.lock().await;
    let _gate = app.draft.gate().lock().await;

    let (removed, emptied, lights, scenes) = {
        let store = app.store.read().await;
        let Some(index) = store.lights.lights.iter().position(|light| light.id == id) else {
            return Err(ApiFailure::not_found(format!("no light with the id {id}")));
        };
        // A light on a device the daemon drives is removed through chain setup, never here.
        if crate::is_driven(&store.lights.lights[index].device_id) {
            return Err(ApiFailure::conflict(format!(
                "the light {id} is on a device the daemon drives; remove it through chain setup"
            )));
        }
        // The snapshot is taken BEFORE any write: the folder holds the owner's files exactly as
        // they were, so this removal can be undone by hand. Without it, nothing is removed.
        if let Err(e) = crate::storage::snapshot(&store.home, "before-removing-a-light") {
            return Err(ApiFailure::service_unavailable(format!(
                "the light was not removed: a snapshot could not be taken: {e}"
            )));
        }
        let mut lights = store.lights.clone();
        let mut scenes = store.scenes.clone();
        let removed = lights.lights[index].clone();
        lights.lights.remove(index);
        // The light leaves every group. A group it leaves empty is named in the answer; the repair
        // below drops it, as it drops any other empty group.
        let mut emptied: Vec<EmptiedGroup> = Vec::new();
        for scene in &mut scenes.scenes {
            let mut changed = false;
            for group in &mut scene.groups {
                if group.members.iter().any(|member| member == &id) {
                    group.members.retain(|member| member != &id);
                    changed = true;
                    if group.members.is_empty() {
                        emptied.push(EmptiedGroup {
                            scene_id: scene.id.clone(),
                            group_id: group.id.clone(),
                            name: group.name.clone(),
                        });
                    }
                }
            }
            // Every server-side scene write moves the rev, so a PUT still holding the old one is
            // refused as stale (409) instead of writing over this change (#497).
            if changed {
                scene.rev += 1;
            }
        }
        let _changes = crate::scenes::repair(&mut scenes, &lights);
        (removed, emptied, lights, scenes)
    };
    let next_lights = lights.clone();
    let next_scenes = scenes.clone();
    crate::store_io::store_save_rolled(
        &app,
        |store| {
            // Kept so a failed save leaves memory as it was: never ahead of the disk and the engine.
            let before_lights = std::mem::replace(&mut store.lights, next_lights);
            let before_scenes = std::mem::replace(&mut store.scenes, next_scenes);
            Ok(((), (before_lights, before_scenes)))
        },
        |store| {
            store.save_lights()?;
            store.save_scenes()
        },
        |(before_lights, before_scenes), store, e, _| {
            store.lights = before_lights;
            store.scenes = before_scenes;
            // Best effort: put the lights file back too, so a half-written pair is not left behind.
            crate::logging::warn_if_err(
                "lights.json could not be put back after scenes.json failed to save",
                store.save_lights(),
            );
            ApiFailure::service_unavailable(format!("the light was not removed: {e}"))
        },
    )
    .await?;

    // The scene changed, so it is replanned and published the way a patch that changes the
    // arrangement is: the editor sees the light gone and its groups repaired. The scene order is
    // already held from before the store write; it is reused here, not taken again (#497).
    if !app.scene.is_taken_over() {
        crate::scenes::apply_with(&app, &lock, false).await;
    }
    app.scene.notify();

    tracing::info!(light = %removed.id, "a light was removed");
    Ok(Json(LightDeleted { ok: true, id: removed.id, emptied_groups: emptied }))
}
