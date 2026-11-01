//! The editor's routes: an effect's source, a draft's preview, and the draft on the lights.
//!
//! These are the daemon half of the effect editor (#2). The editor itself is a later PR; what
//! lives here is everything it needs to open a file, render a draft, and try one on the lights
//! without saving anything.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::EffectSource;
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
