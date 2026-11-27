//! The pattern routes: list, read, save and delete a saved pattern (0.16.0).
//!
//! A pattern is written whole and read whole, as a stack is: the editor holds the draft and
//! saves once, so a stroke never lands as a half-saved pattern. The file is written atomically
//! through [`crate::storage::write_json`], like every other saved file.

use axum::extract::{Path, Query, State};
use axum::Json;
use canvas_proto::api::{
    DeletePatternQuery, DeletePatternResponse, PatternsResponse,
};
use canvas_proto::pattern::{Pattern, PATTERN_ID_PREFIX};
use std::sync::Arc;

use crate::api::{effects, ApiFailure, ApiResult};
use crate::state::App;
use crate::patterns::PatternsError;

/// `GET /api/v1/patterns`.
pub async fn list_patterns(State(app): State<Arc<App>>) -> Json<PatternsResponse> {
    Json(PatternsResponse { patterns: app.patterns.all() })
}

/// `GET /api/v1/patterns/{id}`: the whole pattern, by `pattern:<name>` or by name.
pub async fn get_pattern(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<Pattern>> {
    let pattern = app
        .patterns
        .get(&id)
        .ok_or_else(|| ApiFailure::not_found(format!("no pattern with the id or name {id}")))?;
    Ok(Json(pattern))
}

/// `PUT /api/v1/patterns/{id}`.
///
/// The whole document in, the saved pattern out. The path id is `pattern:<name>` or the bare
/// name; a body whose id differs from the path is a 422, because a save never renames. 409 when
/// the name is another pattern's, ignoring case; 422 for a pattern that breaks a rule, with the
/// sentence. A PUT of a new name creates it: 404 is not used.
pub async fn put_pattern(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
) -> ApiResult<Json<Pattern>> {
    // The body is read as bytes, so a body that is not a pattern is answered with the API's own
    // error shape and its reason, not axum's plain text.
    let bytes = crate::api::limited_body(body, effects::IMPORT_BODY_LIMIT)?;
    let pattern: Pattern = serde_json::from_slice(&bytes)
        .map_err(|e| ApiFailure::validation(format!("the body is not a pattern: {e}")))?;

    let canonical = if id.starts_with(PATTERN_ID_PREFIX) {
        id.clone()
    } else {
        format!("{PATTERN_ID_PREFIX}{id}")
    };
    if pattern.id != canonical {
        return Err(ApiFailure::validation(format!(
            "the pattern's id in the body ({}) does not match the path ({canonical})",
            pattern.id
        )));
    }

    // The file is written on a blocking thread: the write syncs to disk (#241).
    let patterns = Arc::clone(&app.patterns);
    let saved = tokio::task::spawn_blocking(move || patterns.put(pattern))
        .await
        .map_err(|e| ApiFailure::internal(format!("the pattern save did not finish: {e}")))?
        .map_err(|error| match error {
        PatternsError::Invalid(message) => ApiFailure::validation(message),
        PatternsError::Conflict(message) => ApiFailure::conflict(message),
        PatternsError::Io(message) => {
            ApiFailure::internal(format!("could not save the pattern: {message}"))
        }
    })?;
    tracing::info!(pattern = %saved.id, "a pattern was saved");
    // A saved pattern is live at once: every group of the active scene that plays it takes the new
    // colours, and a group that held the old document takes the new one. The store is written, so
    // the change never depends on a later scene save.
    crate::scenes::pattern_changed(&app, &saved.id).await;
    Ok(Json(saved))
}

/// `DELETE /api/v1/patterns/{id}`.
///
/// A pattern a group of **any** scene plays is only deleted with `?force=true`: that group would
/// go dark. The refusal names the scenes, so the owner knows where the pattern is used. A forced
/// delete removes the pattern and its favourite, and the group that played it goes dark with a
/// note, as a deleted stack's does.
///
/// The whole delete runs on a task of its own, as a chain commit does: it is two writes
/// (`effects.json`, then the pattern file) and a client that goes away between them must not leave
/// the favourite gone and the pattern standing.
pub async fn delete_pattern(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(query): Query<DeletePatternQuery>,
) -> ApiResult<Json<DeletePatternResponse>> {
    tokio::spawn(async move { delete_the_pattern(app, id, query).await })
        .await
        .unwrap_or_else(|e| {
            Err(ApiFailure::internal(format!("the pattern delete did not finish: {e}")))
        })
}

/// The delete itself, on the detached task [`delete_pattern`] starts.
async fn delete_the_pattern(
    app: Arc<App>,
    id: String,
    query: DeletePatternQuery,
) -> ApiResult<Json<DeletePatternResponse>> {
    let pattern = app
        .patterns
        .get(&id)
        .ok_or_else(|| ApiFailure::not_found(format!("no pattern with the id or name {id}")))?;

    // The delete replaces what is on the lights: the draft gate is held across it, so a draft is
    // not put over by accident.
    let _gate = app.draft.gate().lock().await;

    // Every group of every scene: a pattern in a scene that is not active is still in use.
    let named_by = crate::scenes::scenes_playing(&app, &pattern.id).await;
    if !named_by.is_empty() && !query.force {
        let names: Vec<String> = named_by.iter().map(|(_, name)| name.clone()).collect();
        // The editor shows this sentence as it is, so it is in the owner's words: the pattern's
        // name, the scenes, and what a forced delete does.
        let name = pattern.id.strip_prefix(canvas_proto::pattern::PATTERN_ID_PREFIX).unwrap_or(&pattern.id);
        return Err(ApiFailure::conflict(format!(
            "{name} plays in {}. Delete it anyway and the groups that play it go dark.",
            names.join(", ")
        )));
    }

    // Its favourite is keyed by the id and would be unreachable with it gone. It goes first: a
    // failed write of effects.json puts it back and leaves the pattern alone.
    let pid = pattern.id.clone();
    let favorite_removed = crate::store_io::store_save_rolled(
        &app,
        |store| {
            let effects_before = store.effects.clone();
            let before = store.effects.favorites.len();
            store.effects.favorites.retain(|favorite| favorite != &pid);
            let favorite_removed = store.effects.favorites.len() != before;
            Ok((favorite_removed, effects_before))
        },
        |store| store.save_effects(),
        |before, store, e, _| {
            store.effects = before;
            ApiFailure::internal(format!("the pattern was left as it was: {e}"))
        },
    )
    .await?;

    // The pattern file is written on a blocking thread: the write syncs to disk.
    let patterns = Arc::clone(&app.patterns);
    let target = pattern.id.clone();
    let deleted = tokio::task::spawn_blocking(move || patterns.delete(&target))
        .await
        .map_err(|e| ApiFailure::internal(format!("the pattern delete did not finish: {e}")))?;
    let removed = match deleted {
        Ok(Some(removed)) => removed,
        Ok(None) => {
            restore_favorite(&app, &pid, favorite_removed).await;
            return Err(ApiFailure::not_found(format!(
                "no pattern with the id or name {id}"
            )));
        }
        Err(e) => {
            // The pattern file was not written: put its favourite back too.
            restore_favorite(&app, &pid, favorite_removed).await;
            return Err(ApiFailure::internal(format!(
                "the pattern could not be deleted, and nothing was removed: {e}"
            )));
        }
    };

    // A draft of this pattern holds the lights: it ends here, before the pattern is gone, or it
    // would put a pattern that cannot be built any more back on the lights.
    if app.draft.snapshot().is_some_and(|slot| drafts_pattern(&slot.id, &removed.id)) {
        let ended = crate::draft::stop_locked(&app, None).await;
        if let Some(ended) = ended.ended {
            if let Some(note) = ended.note {
                tracing::warn!(%note, "the draft ended when the pattern it showed was deleted");
            }
        }
    }

    // A pattern a group plays ends with that group dark and a note, the ending a deleted stack
    // gets: the scene goes on again, the group cannot build its effect any more, and the others
    // run. A draft that holds the lights keeps them: the scene waits for it.
    let mut note = None;
    if !named_by.is_empty() {
        if !app.draft.is_active() {
            crate::scenes::apply_active_ungated(&app, false).await;
        }
        note = Some(crate::api::scrub_errors::scrub_error_text(
            &app.scrubber.lock().unwrap_or_else(|e| e.into_inner()),
            &format!(
                "{} was deleted while a group played it, so that group is dark",
                removed.id
            ),
        ));
        tracing::warn!(pattern = %removed.id, "a pattern a group played was deleted");
    }
    tracing::info!(pattern = %removed.id, favorite_removed, "a pattern was deleted");

    Ok(Json(DeletePatternResponse {
        ok: true,
        id: removed.id,
        favorite_removed,
        note,
    }))
}

/// True when a draft's slot is this pattern's: the editor's own slot during a pattern draft
/// (`draft:pattern:<name>`), or the pattern itself, which a later step may use.
fn drafts_pattern(slot: &str, pattern_id: &str) -> bool {
    slot == pattern_id || slot == format!("draft:{pattern_id}")
}

/// Puts a deleted pattern's favourite back after the pattern file could not be changed.
///
/// Only the favourite goes back, read and written inside the writer-serialised section: a copy of
/// all of `effects.json` taken before the delete would overwrite a preset or a favourite another
/// request saved in between.
async fn restore_favorite(app: &Arc<App>, id: &str, favorite_removed: bool) {
    if !favorite_removed {
        return;
    }
    let id = id.to_string();
    let result = crate::store_io::store_save(
        app,
        |store| {
            if !store.effects.favorites.contains(&id) {
                store.effects.favorites.push(id.clone());
                store.effects.favorites.sort();
            }
            Ok(())
        },
        |store| {
            store
                .save_effects()
                .map_err(|e| ApiFailure::internal(e.to_string()))
        },
    )
    .await;
    crate::logging::warn_if_err("a pattern's favourite could not be put back", result.map_err(|e| e.message));
}
