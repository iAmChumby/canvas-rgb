//! `PUT`/`DELETE /api/v1/stacks/preview`: the builder's live preview (D95.4.4).
//!
//! The preview runs on its own engine, with no device sink, so nothing here can touch a light.
//! The heavy part (building a stack, which loads its HTML layers) runs on a blocking thread;
//! the route shares [`crate::api::editor::PREVIEW_CONCURRENT`] with `POST /effects/preview`, so
//! at most two preview builds run at once and the rest answer 503 with `Retry-After: 1`.

use axum::extract::State;
use axum::Json;
use canvas_proto::api::StackPreviewRequest;
use std::sync::Arc;

use crate::api::{effects, ApiFailure, ApiResult};
use crate::preview::PreviewError;
use crate::state::App;

/// `PUT /api/v1/stacks/preview`.
pub async fn put_stack_preview(
    State(app): State<Arc<App>>,
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
) -> ApiResult<Json<canvas_proto::api::StackPreviewResponse>> {
    let bytes = crate::api::limited_body(body, effects::IMPORT_BODY_LIMIT)?;
    let request: StackPreviewRequest = serde_json::from_slice(&bytes)
        .map_err(|e| ApiFailure::validation(format!("the body is not a preview request: {e}")))?;
    if !crate::epoch::valid_epoch(&request.epoch) {
        return Err(ApiFailure::validation(format!(
            "the epoch {:?} is not 16 hexadecimal characters, as a page makes it",
            request.epoch
        )));
    }
    let answer = crate::preview::put(&app, &request.stack, &request.epoch, request.seq)
        .await
        .map_err(|error| match error {
            PreviewError::Busy => ApiFailure::service_unavailable(format!(
                "the daemon is already rendering {} previews; try again shortly",
                crate::api::editor::PREVIEW_CONCURRENT
            ))
            .with_retry_after(1),
            PreviewError::Conflict(message) => ApiFailure::conflict(message),
            PreviewError::Validation(message) => ApiFailure::validation(message),
            PreviewError::Internal(message) => ApiFailure::internal(message),
        })?;
    Ok(Json(answer))
}

/// `DELETE /api/v1/stacks/preview`: drops the preview and clears the sequence watermark.
pub async fn delete_stack_preview(State(app): State<Arc<App>>) -> Json<serde_json::Value> {
    let had = crate::preview::clear(&app).await;
    Json(serde_json::json!({ "ok": true, "running": had }))
}
