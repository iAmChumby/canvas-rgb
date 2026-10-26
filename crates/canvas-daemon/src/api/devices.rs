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
/// Lights one white LED at a time so Luke can count to the physical end of a header.
pub async fn identify(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(request): Json<IdentifyRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let device_id = DeviceId(id.clone());
    if app.device(&device_id).await.is_none() {
        return Err(ApiFailure::not_found(format!("no device with the id {id}")));
    }
    {
        let mut adapter = app.adapter.lock().await;
        adapter
            .identify(&device_id, request.zone, request.led)
            .await
            .map_err(|e| ApiFailure::internal(format!("identify failed: {e}")))?;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
