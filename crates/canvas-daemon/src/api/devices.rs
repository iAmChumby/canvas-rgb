//! `GET /api/v1/devices`, `PATCH /api/v1/devices/{id}/zones/{zone}` and
//! `POST /api/v1/devices/{id}/identify`.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{DevicesResponse, IdentifyRequest, PatchZone};
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
        app.engine.set_layout(layout, devices);
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
        app.identify.push(frames);
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
