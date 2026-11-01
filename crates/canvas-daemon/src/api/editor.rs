//! The editor's routes: an effect's source, a draft's preview, and the draft on the lights.
//!
//! These are the daemon half of the effect editor (#2). The editor itself is a later PR; what
//! lives here is everything it needs to open a file, render a draft, and try one on the lights
//! without saving anything.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{EffectPreviewRequest, EffectPreviewResponse, EffectSource};
use canvas_proto::props::Props;
use canvas_proto::traits::Effect as _;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// `GET /api/v1/effects/{id}/source`.
///
/// The HTML of an `html:` effect, user or stock, and the file name its Save would write.
/// A Rust built-in has no source: it is part of the app, so the route says so with a 404.
pub async fn effect_source(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<EffectSource>> {
    if let Some((source, file_name)) = app.user_effects.source_and_file_name(&id) {
        return Ok(Json(EffectSource { source: source.to_string(), file_name }));
    }
    // An id that is not an html: effect is a Rust built-in (or one the daemon does not know).
    match crate::api::effects::find_schema(&app.user_effects, &id) {
        Some(schema) if !schema.id.starts_with(crate::user_effects::USER_EFFECT_PREFIX) => {
            Err(ApiFailure::not_found(format!(
                "{} is a built-in effect written in Rust; it has no HTML source to edit",
                schema.id
            )))
        }
        _ => Err(ApiFailure::not_found(format!("no effect with the id or name {id}"))),
    }
}

/// `POST /api/v1/effects/preview`.
///
/// The draft's HTML and the editor's props, rendered headless: the schema, the runtime's
/// errors, the check warnings, a source hash, and the 64x40 24-frame loop. No device is
/// touched and nothing is written.
///
/// The parse and the render run on a blocking thread, so a draft that runs a script past its
/// deadline never holds the API: the runtime stops it at its load or frame budget and the
/// answer says what happened. A draft whose top-level script runs over is refused (422);
/// one whose frame callback runs over still answers with its loop and lists the error, so the
/// editor can keep the last good picture.
pub async fn preview_effect(
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
) -> ApiResult<Json<EffectPreviewResponse>> {
    let bytes = crate::api::limited_body(body, crate::api::effects::IMPORT_BODY_LIMIT)?;
    let request: EffectPreviewRequest = serde_json::from_slice(&bytes)
        .map_err(|e| ApiFailure::validation(format!("the body is not a preview request: {e}")))?;
    if request.html.len() > canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES {
        return Err(ApiFailure::validation(format!(
            "the draft is {} bytes; the limit is {}",
            request.html.len(),
            canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES
        )));
    }
    Ok(Json(preview(&request.html, &request.props).await?))
}

/// Renders a draft for [`preview_effect`], off the async workers.
async fn preview(html: &str, props: &Props) -> Result<EffectPreviewResponse, ApiFailure> {
    let source_hash = crate::user_effects::source_hash(html);
    let id = format!("{}{source_hash}", canvas_proto::consts::DRAFT_ID_PREFIX);
    let html = html.to_string();
    let props = props.clone();
    tokio::task::spawn_blocking(move || -> Result<EffectPreviewResponse, ApiFailure> {
        // The same build the lights use, so the preview is judged by the runtime that will
        // run it.
        let mut effect = canvas_html::HtmlEffect::from_source(&id, &html, &Props::new())
            .map_err(|e| ApiFailure::validation(crate::user_effects::load_error_sentence(&e.to_string())))?;
        let schema = effect.schema();
        let thumbnail = crate::thumbnail::render(&mut effect, &props).map_err(|e| {
            ApiFailure::service_unavailable(format!("the preview was given up: {}", e.reason()))
        })?;
        let mut warnings: Vec<String> = effect
            .warnings()
            .iter()
            .map(|warning| format!("the file was read with a part ignored: {warning}"))
            .collect();
        if !crate::thumbnail::body_moves(&thumbnail) {
            warnings.push(crate::thumbnail::NO_MOVEMENT.to_string());
        }
        if crate::thumbnail::body_is_flat(&thumbnail) {
            warnings.push(crate::thumbnail::FLAT_COLOUR.to_string());
        }
        Ok(EffectPreviewResponse {
            schema,
            source_hash,
            errors: effect.errors().to_vec(),
            warnings,
            thumbnail: preview_base64(&thumbnail),
        })
    })
    .await
    .map_err(|e| ApiFailure::internal(format!("the preview task stopped: {e}")))?
}

/// The loop's bytes as base64, for the JSON answer.
fn preview_base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
