//! `PUT`/`DELETE /api/v1/stacks/preview`: the builder's live preview (D95.4.4).
//!
//! The preview runs on its own engine, with no device sink, so nothing here can touch a light.
//! The heavy part (building a stack, which loads its HTML layers) runs on a blocking thread;
//! the route shares [`crate::api::editor::PREVIEW_CONCURRENT`] with `POST /effects/preview`, so
//! at most two preview builds run at once and the rest answer 503 with `Retry-After: 1`.

use axum::extract::{Query, State};
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

/// The query `DELETE /stacks/preview` takes: the page's session epoch, when it sends one.
///
/// The route has no request type in `canvas-proto` (its answer is plain JSON), so the query stays
/// here: a caller that sends no epoch keeps the old behaviour, and one that does is answered
/// `applied: false` when it does not own the preview.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DeleteStackPreviewQuery {
    #[serde(default)]
    pub epoch: Option<String>,
}

/// `DELETE /api/v1/stacks/preview`: drops the preview and clears the sequence watermark.
///
/// A page that sends its epoch and does not own the preview drops nothing: `applied` is `false`
/// and the page that owns it keeps previewing (D95.4.4, item 4). `running` still says whether a
/// preview was dropped here, so an earlier caller's answer means what it always did.
pub async fn delete_stack_preview(
    State(app): State<Arc<App>>,
    Query(query): Query<DeleteStackPreviewQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    if let Some(epoch) = &query.epoch {
        if !crate::epoch::valid_epoch(epoch) {
            return Err(ApiFailure::validation(format!(
                "the epoch {epoch:?} is not 16 hexadecimal characters, as a page makes it"
            )));
        }
    }
    let outcome = crate::preview::clear(&app, query.epoch.as_deref()).await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "running": outcome.running,
        "applied": outcome.applied,
    })))
}
