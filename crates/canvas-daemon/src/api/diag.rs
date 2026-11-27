//! The diagnostic routes: the readback and the sensor-failure switch.

use axum::extract::State;
use axum::Json;
use canvas_proto::api::{DiagBundle, ReadbackDevice, ReadbackResponse};
use canvas_proto::fans::TempSource;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// Where a bundle written through the API goes unless the caller names a folder.
fn default_bundle_dir() -> std::path::PathBuf {
    crate::paths::home().join("diagnostics")
}

/// `GET /api/v1/diag/bundle`: the file list and sizes before anything is saved (#575).
pub async fn bundle_list(State(_app): State<Arc<App>>) -> ApiResult<Json<DiagBundle>> {
    Ok(Json(canvas_diag::bundle::Bundle::new().list()))
}

/// The optional body of `POST /api/v1/diag/bundle`: an output folder under the user's own tree.
#[derive(Debug, Default, Deserialize)]
pub struct BundleWriteRequest {
    pub path: Option<String>,
}

/// `POST /api/v1/diag/bundle`: writes the scrubbed, aliased zip and answers with its path.
pub async fn bundle_write(
    State(app): State<Arc<App>>,
    body: Option<Json<BundleWriteRequest>>,
) -> ApiResult<Json<DiagBundle>> {
    let target = body
        .and_then(|Json(body)| body.path)
        .filter(|p| !p.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_bundle_dir);
    let scrubber = app.scrubber.lock().unwrap_or_else(|e| e.into_inner());
    let result = canvas_diag::bundle::Bundle::new()
        .write(&target, &scrubber)
        .map_err(|e| ApiFailure::internal(format!("the diagnostics bundle could not be written: {e}")))?;
    Ok(Json(result))
}

/// `GET /api/v1/diag/readback`.
///
/// The colours as the hardware side reports them (OpenRGB's REQUEST_CONTROLLER_DATA).
///
/// The readback runs on a task of its own (#308): a client that goes away in the middle of
/// it must not cancel a request OpenRGB already answered. A cancelled request breaks the
/// OpenRGB link (the reply may still come, and it would answer the next request), and
/// every light would pause for one reconnect.
pub async fn readback(State(app): State<Arc<App>>) -> ApiResult<Json<ReadbackResponse>> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    let for_the_work = Arc::clone(&app);
    tokio::spawn(async move {
        let _ = reply.send(read_the_colours(&for_the_work).await);
    });
    let devices = answer
        .await
        .map_err(|_| ApiFailure::internal("the readback task ended"))?;
    Ok(Json(ReadbackResponse { devices }))
}

/// The colours of every device the daemon knows, as the hardware side reports them.
///
/// A device that cannot be read is left out and the reason is logged: the answer is what
/// the hardware says, not a guess.
async fn read_the_colours(app: &Arc<App>) -> Vec<ReadbackDevice> {
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
    out
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
    let response = crate::pipe_client::send(
        &app,
        &canvas_proto::pipe::PipeRequest::FailSensor { source: source.clone(), failed: request.failed },
    )
    .await
    .map_err(|e| ApiFailure::service_unavailable(format!("the fan service is not reachable: {e}")))?;
    // Issue #225: the service's refusal is the answer, not a toggle that happened.
    crate::api::fans::expect_ok(response)?;

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
