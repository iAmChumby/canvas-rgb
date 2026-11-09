//! `GET /api/v1/devices`, `PATCH /api/v1/devices/{id}/zones/{zone}` and
//! `POST /api/v1/devices/{id}/identify`, and the white balance (#50):
//! `GET /api/v1/white-balance`, `PUT/DELETE /api/v1/devices/{id}/white-balance`.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{DevicesResponse, IdentifyRequest, PatchZone, WhiteBalances};
use canvas_proto::color::WhiteBalance;
use canvas_proto::device::DeviceId;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// `GET /api/v1/devices`.
pub async fn list_devices(State(app): State<Arc<App>>) -> Json<DevicesResponse> {
    let devices = app.devices.read().await.clone();
    Json(DevicesResponse { devices })
}

/// `PATCH /api/v1/devices/{id}/zones/{zone}`.
///
/// Saves the new LED count to `zones.json` (keyed by the device fingerprint) and resizes
/// the zone in OpenRGB. The size is re-applied on every OpenRGB connect.
pub async fn patch_zone(
    State(app): State<Arc<App>>,
    Path((id, zone)): Path<(String, u32)>,
    Json(patch): Json<PatchZone>,
) -> ApiResult<Json<serde_json::Value>> {
    let device_id = DeviceId(id.clone());
    let device = app
        .device(&device_id)
        .await
        .ok_or_else(|| ApiFailure::not_found(format!("no device with the id {id}")))?;

    let Some(zone_info) = device.zones.iter().find(|z| z.index == zone) else {
        return Err(ApiFailure::not_found(format!("{id} has no zone {zone}")));
    };
    if !zone_info.resizable {
        return Err(ApiFailure::validation(format!(
            "{id} zone {zone} cannot be resized"
        )));
    }
    if patch.led_count < zone_info.min_leds || patch.led_count > zone_info.max_leds {
        return Err(ApiFailure::validation(format!(
            "{id} zone {zone} takes {}..={} LEDs, got {}",
            zone_info.min_leds, zone_info.max_leds, patch.led_count
        )));
    }

    // Resize in OpenRGB first: if the hardware refuses, the saved size must not change.
    {
        let mut adapter = app.adapter.lock().await;
        adapter
            .set_zone_size(&device_id, zone, patch.led_count)
            .await
            .map_err(|e| ApiFailure::internal(format!("could not resize the zone: {e}")))?;
    }

    // Save it.
    {
        let home = app.store.read().await.home.clone();
        let mut zones = app.zones.write().await;
        zones.entry(id.clone()).or_default().insert(zone, patch.led_count);
        let file = home.join(canvas_proto::config::ZONES_FILE);
        let snapshot = canvas_proto::config::ZonesFile {
            schema_version: canvas_proto::config::ZONES_SCHEMA,
            sizes: zones.clone(),
        };
        crate::storage::write_json(&file, &snapshot)
            .map_err(|e| ApiFailure::internal(format!("could not save the zone sizes: {e}")))?;
    }

    // Re-discover so the device list carries the new LED count, then re-set the layout.
    let devices = {
        let mut adapter = app.adapter.lock().await;
        adapter
            .discover()
            .await
            .map_err(|e| ApiFailure::internal(format!("could not re-discover the devices: {e}")))?
    };
    {
        let mut list = app.devices.write().await;
        *list = devices.clone();
    }
    if let Some(layout) = app.active_layout().await {
        app.set_layout(&layout, &devices);
        app.stacks.publish_areas(&layout, &devices);
    }

    tracing::info!(device = %id, zone, leds = patch.led_count, "the zone size changed");
    Ok(Json(serde_json::json!({ "ok": true, "device_id": id, "zone": zone, "led_count": patch.led_count })))
}

/// `POST /api/v1/devices/{id}/identify`.
///
/// An engine override (see [`crate::identify`]): while it is active every device is black
/// except `count` LEDs from `led` in `zone` of this device, at full brightness, even with
/// output paused. `stop: true` ends it; so does silence for [`crate::identify::WATCHDOG`].
pub async fn identify(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(request): Json<IdentifyRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::transient::touch(&app);
    tracing::info!(
        device = %id, zone = request.zone, led = request.led, count = request.count.unwrap_or(1),
        stop = request.stop, "identify request"
    );
    let device_id = DeviceId(id.clone());
    let Some(device) = app.device(&device_id).await else {
        return Err(ApiFailure::not_found(format!("no device with the id {id}")));
    };
    if request.stop {
        if app.identify.stop() {
            crate::identify::restore(&app).await;
        }
        return Ok(Json(serde_json::json!({ "ok": true })));
    }
    let Some(zone) = device.zones.iter().find(|z| z.index == request.zone) else {
        return Err(ApiFailure::validation(format!("{id} has no zone {}", request.zone)));
    };
    if request.led >= zone.led_count {
        return Err(ApiFailure::validation(format!(
            "LED {} is past the end of zone {} ({} LEDs)",
            request.led, request.zone, zone.led_count
        )));
    }
    let start = app.identify.start(
        device_id,
        request.zone,
        request.led,
        request.count.unwrap_or(1),
        std::time::Instant::now(),
    );
    // Send the first frames now; the ticker repeats them and runs the watchdog.
    let devices = app.devices.read().await.clone();
    if let crate::identify::Tick::Frames(frames) = app.identify.tick(std::time::Instant::now(), &devices) {
        // The push comes first: the line may only say the frame was sent if the writer took it.
        let pushed = app.identify.push(frames);
        crate::identify::log_sent(&device, &pushed);
    }
    if start {
        crate::identify::spawn_ticker(app.clone());
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/v1/onboarding/session`.
///
/// While a session is active every device is black except identify targets, even with output
/// paused. It ends on `active: false` or 5 s after the last `active: true`.
pub async fn onboarding_session(
    State(app): State<Arc<App>>,
    Json(request): Json<canvas_proto::api::OnboardingSession>,
) -> ApiResult<Json<serde_json::Value>> {
    if request.active {
        let (was_new, start) = app.identify.start_session(std::time::Instant::now());
        if was_new {
            tracing::info!("onboarding session started: every light is black except identify targets");
        }
        let devices = app.devices.read().await.clone();
        if let crate::identify::Tick::Frames(frames) = app.identify.tick(std::time::Instant::now(), &devices) {
            app.identify.push(frames);
        }
        if start {
            crate::identify::spawn_ticker(app.clone());
        }
    } else if app.identify.end_session() {
        tracing::info!("onboarding session ended by the page");
        crate::identify::restore(&app).await;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------- white balance (#50)

/// The most devices that may carry a white balance: the config never grows past this.
pub const WHITE_BALANCE_MAX_DEVICES: usize = 128;
/// The longest device fingerprint a white balance is kept for.
pub const WHITE_BALANCE_MAX_ID_LEN: usize = 256;

/// The config's white balances as the engine takes them.
pub fn white_balance_map(
    config: &canvas_proto::config::ConfigFile,
) -> std::collections::HashMap<DeviceId, WhiteBalance> {
    config.white_balance.iter().map(|(id, wb)| (id.clone(), *wb)).collect()
}

async fn white_balances(app: &Arc<App>) -> WhiteBalances {
    WhiteBalances { devices: app.store.read().await.config.white_balance.clone() }
}

/// `GET /api/v1/white-balance`.
pub async fn get_white_balance(State(app): State<Arc<App>>) -> Json<WhiteBalances> {
    Json(white_balances(&app).await)
}

/// `PUT /api/v1/devices/{id}/white-balance`.
///
/// Sets one device's gains, saves them and applies them to the next frame. The device need
/// not be connected. A neutral balance is stored as no entry.
pub async fn put_white_balance(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(balance): Json<WhiteBalance>,
) -> ApiResult<Json<WhiteBalances>> {
    if !balance.is_valid() {
        return Err(ApiFailure::validation(
            "every white-balance gain must be a number from 0 to 1",
        ));
    }
    set_white_balance(&app, id, balance).await
}

/// `DELETE /api/v1/devices/{id}/white-balance`: back to neutral.
pub async fn delete_white_balance(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<WhiteBalances>> {
    set_white_balance(&app, id, WhiteBalance::NEUTRAL).await
}

async fn set_white_balance(
    app: &Arc<App>,
    id: String,
    balance: WhiteBalance,
) -> ApiResult<Json<WhiteBalances>> {
    if id.is_empty() || id.len() > WHITE_BALANCE_MAX_ID_LEN {
        return Err(ApiFailure::validation(format!(
            "a device id is 1..={WHITE_BALANCE_MAX_ID_LEN} bytes"
        )));
    }
    let device_id = DeviceId(id);
    let balances = {
        let mut store = app.store.write().await;
        let map = &mut store.config.white_balance;
        if balance == WhiteBalance::NEUTRAL {
            map.remove(&device_id);
        } else {
            if !map.contains_key(&device_id) && map.len() >= WHITE_BALANCE_MAX_DEVICES {
                return Err(ApiFailure::validation(format!(
                    "at most {WHITE_BALANCE_MAX_DEVICES} devices can carry a white balance"
                )));
            }
            map.insert(device_id.clone(), balance);
        }
        store
            .save_config()
            .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))?;
        white_balance_map(&store.config)
    };
    app.engine.set_white_balance(balances);
    tracing::info!(
        device = %device_id, red = balance.red, green = balance.green, blue = balance.blue,
        "white balance set"
    );
    Ok(Json(white_balances(app).await))
}
