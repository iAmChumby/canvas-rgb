//! The diagnostic routes: the readback and the sensor-failure switch.

use axum::extract::State;
use axum::Json;
use canvas_proto::api::{ReadbackDevice, ReadbackResponse};
use canvas_proto::fans::TempSource;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// `GET /api/v1/diag/readback`.
///
/// The colours as the hardware side reports them (OpenRGB's REQUEST_CONTROLLER_DATA).
pub async fn readback(State(app): State<Arc<App>>) -> ApiResult<Json<ReadbackResponse>> {
    let devices = app.devices.read().await.clone();
    let mut out = Vec::new();
    for device in devices {
        let colors = {
            let mut adapter = app.adapter.lock().await;
            adapter.read_back(&device.id).await
        };
        let colors = match colors {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(device = %device.id, error = %e, "the readback failed");
                continue;
            }
        };
        out.push(ReadbackDevice {
            device_id: device.id.0.clone(),
            name: device.name.clone(),
            colors,
        });
    }
    Ok(Json(ReadbackResponse { devices: out }))
}

/// The body of `POST /api/v1/diag/fail-sensor`.
#[derive(Debug, Clone, Deserialize)]
pub struct FailSensorRequest {
    /// `cpu`, `gpu` or `board:<name>`.
    pub source: String,
    /// True to fail the sensor, false to clear the failure.
    #[serde(default)]
    pub failed: bool,
}

/// Parses the CLI's sensor argument into a [`TempSource`].
pub fn parse_source(source: &str) -> Option<TempSource> {
    let source = source.trim();
    match source.to_ascii_lowercase().as_str() {
        "cpu" => Some(TempSource::Cpu),
        "gpu" => Some(TempSource::Gpu),
        other => other
            .strip_prefix("board:")
            .map(|name| TempSource::Board(name.trim().to_string())),
    }
}

/// `POST /api/v1/diag/fail-sensor`.
///
/// Tells the service to treat a temperature source as failed (the safety test).
pub async fn fail_sensor(
    State(app): State<Arc<App>>,
    Json(request): Json<FailSensorRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let source = parse_source(&request.source)
        .ok_or_else(|| ApiFailure::validation(format!("unknown sensor source {}", request.source)))?;
    crate::pipe_client::send(
        &app,
        &canvas_proto::pipe::PipeRequest::FailSensor { source: source.clone(), failed: request.failed },
    )
    .await
    .map_err(|e| ApiFailure::service_unavailable(format!("the fan service is not reachable: {e}")))?;

    tracing::info!(?source, failed = request.failed, "the sensor failure was toggled");
    Ok(Json(serde_json::json!({ "ok": true, "source": source, "failed": request.failed })))
}

/// The query the readback accepts (`?device=` to pick one).
#[derive(Debug, Deserialize)]
pub struct ReadbackQuery {
    pub device: Option<String>,
}

/// A helper for the CLI: the query string for the readback.
pub fn readback_query(device: Option<&str>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(device) = device {
        map.insert("device".to_string(), device.to_string());
    }
    map
}
