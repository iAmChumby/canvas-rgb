//! The stack routes: list, read, save and delete a saved effect stack (D95.4.1).
//!
//! A stack is written whole and read whole: the editor holds the draft and saves once, so a
//! reorder is one write and one undo step. The file is written atomically through
//! [`crate::storage::write_json`], like every other saved file.

use axum::extract::{Path, Query, State};
use axum::Json;
use canvas_proto::api::{
    DeleteStackQuery, DeleteStackResponse, StackDetail, StackLayerState, StackLayerStatus,
    StackSummary, StacksResponse,
};
use canvas_proto::stack::{Stack, STACK_ID_PREFIX};
use std::sync::Arc;

use crate::api::{effects, ApiFailure, ApiResult};
use crate::state::App;
use crate::stacks::{KnownEffects, StacksError};

/// `GET /api/v1/stacks`.
pub async fn list_stacks(State(app): State<Arc<App>>) -> Json<StacksResponse> {
    let stacks = app.stacks.all();
    Json(StacksResponse {
        stacks: stacks.iter().map(|stack| summary(&app, stack)).collect(),
    })
}

/// A stack as the library lists it.
fn summary(app: &Arc<App>, stack: &Stack) -> StackSummary {
    let missing = stack
        .layers
        .iter()
        .filter(|layer| {
            !layer.effect.starts_with(STACK_ID_PREFIX)
                && effects::find_schema(&app.user_effects, &app.stacks, &layer.effect).is_none()
        })
        .map(|layer| layer.id.clone())
        .collect();
    StackSummary {
        id: stack.id.clone(),
        name: stack.name().to_string(),
        layers: stack.layers.len() as u32,
        html_layers: stack
            .layers
            .iter()
            .filter(|layer| crate::stacks::is_html_effect(&layer.effect))
            .count() as u32,
        missing,
    }
}

/// `GET /api/v1/stacks/{id}`: the whole stack and what each layer is doing.
pub async fn get_stack(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<StackDetail>> {
    let stack = app
        .stacks
        .get(&id)
        .ok_or_else(|| ApiFailure::not_found(format!("no stack with the id or name {id}")))?;
    let layers = layer_statuses(&app, &stack);
    Ok(Json(StackDetail { stack, layers }))
}

/// What each layer is doing: its effect missing, the area empty, or the runtime's own report.
fn layer_statuses(app: &Arc<App>, stack: &Stack) -> Vec<StackLayerStatus> {
    let runtime = if app.stacks.running_id().as_deref() == Some(stack.id.as_str()) {
        app.stacks.runtime_statuses()
    } else {
        Vec::new()
    };
    let book = app.stacks.areas();
    let areas = crate::stack::read_recovering(&book);
    stack
        .layers
        .iter()
        .map(|layer| {
            let (status, last_ms) = if layer.effect.starts_with(STACK_ID_PREFIX) {
                (StackLayerState::NestedStack, None)
            } else if effects::find_schema(&app.user_effects, &app.stacks, &layer.effect).is_none() {
                (StackLayerState::MissingEffect, None)
            } else if areas.is_empty_area(&layer.area) {
                (StackLayerState::NoArea, None)
            } else if let Some(reported) = runtime.iter().find(|status| status.id == layer.id) {
                (reported.status, reported.last_ms)
            } else {
                (StackLayerState::Ok, None)
            };
            StackLayerStatus { id: layer.id.clone(), status, last_ms }
        })
        .collect()
}

/// `PUT /api/v1/stacks/{id}`.
///
/// The whole document in, the saved stack out. 409 when the name is another stack's, ignoring
/// case; 422 for a name, a layer id, a cap or a value that breaks the rules; 404 is not used: a
/// PUT of a new stack creates it.
pub async fn put_stack(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
) -> ApiResult<Json<Stack>> {
    // The body is read as bytes, so a body that is not a stack is answered with the API's own
    // error shape and its reason (an unknown blend mode, say), not axum's plain text.
    let bytes = crate::api::limited_body(body, effects::IMPORT_BODY_LIMIT)?;
    let stack: Stack = serde_json::from_slice(&bytes)
        .map_err(|e| ApiFailure::validation(format!("the body is not a stack: {e}")))?;

    // The id is the path: `stack:<name>`, or the bare name the builder uses. A save never
    // renames, so a body that names a different stack is refused.
    let canonical = if id.starts_with(STACK_ID_PREFIX) {
        id.clone()
    } else {
        format!("{STACK_ID_PREFIX}{id}")
    };
    if stack.id != canonical {
        return Err(ApiFailure::validation(format!(
            "the stack's id in the body ({}) does not match the path ({canonical})",
            stack.id
        )));
    }

    let known = KnownEffects::from_user(&app.user_effects);
    let saved = app.stacks.put(stack, &known).map_err(|error| match error {
        StacksError::Invalid(message) => ApiFailure::validation(message),
        StacksError::Conflict(message) => ApiFailure::conflict(message),
        StacksError::Io(message) => {
            ApiFailure::internal(format!("could not save the stack: {message}"))
        }
    })?;

    // A save of the stack that is on the engine updates its live values, so a blend or an
    // opacity shows at once; a changed structure waits for the activation that Save also does.
    let structure = crate::stack::structure_hash(&crate::stack::structure(&saved, &|id| {
        app.user_effects
            .source_and_file_name(id)
            .map(|(source, _)| source.to_string())
    }));
    if app.stacks.running_is(&saved.id, &structure) {
        app.stacks.update_live(&saved);
    }
    tracing::info!(stack = %saved.id, layers = saved.layers.len(), "a stack was saved");
    Ok(Json(saved))
}

/// `DELETE /api/v1/stacks/{id}`.
///
/// A stack on the lights is only deleted with `?force=true`: its layout names it, and the lights
/// would go black. A forced delete removes the stack, its presets and its favourite, and puts
/// black on the lights with a note (D95.1.6, D95.4.7).
pub async fn delete_stack(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(query): Query<DeleteStackQuery>,
) -> ApiResult<Json<DeleteStackResponse>> {
    let stack = app
        .stacks
        .get(&id)
        .ok_or_else(|| ApiFailure::not_found(format!("no stack with the id or name {id}")))?;

    let active = app.active_layout().await;
    let named_by = active
        .as_ref()
        .filter(|layout| {
            layout
                .effect
                .as_ref()
                .is_some_and(|effect| effect.id == stack.id)
        })
        .map(|layout| (layout.id.clone(), layout.name.clone()));
    if named_by.is_some() && !query.force {
        let (layout_id, layout_name) = named_by.clone().unwrap();
        return Err(ApiFailure::conflict(format!(
            "{} is on the lights in the layout {layout_name} ({layout_id}); delete it with ?force=true and the lights will go black",
            stack.id
        )));
    }

    let removed = app.stacks.delete(&stack.id).ok_or_else(|| {
        ApiFailure::not_found(format!("no stack with the id or name {id}"))
    })?;

    // Its presets and its favourite are keyed by the id and would be unreachable with it gone.
    let mut presets_removed = 0usize;
    let favorite_removed = {
        let mut store = app.store.write().await;
        if let Some(list) = store.effects.presets.remove(&removed.id) {
            presets_removed = list.len();
        }
        let before = store.effects.favorites.len();
        store.effects.favorites.retain(|favorite| favorite != &removed.id);
        let favorite_removed = store.effects.favorites.len() != before;
        if let Err(e) = store.save_effects() {
            return Err(ApiFailure::internal(format!(
                "the stack is gone but its presets could not be saved: {e}"
            )));
        }
        favorite_removed
    };

    // A stack on the lights ends with black and a note, the ending a deleted prior effect gets.
    let running = app.stacks.running_id().as_deref() == Some(removed.id.as_str());
    app.stacks.clear_running();
    let mut note = None;
    if running || named_by.is_some() {
        match crate::apply_effect(&app, "solid", &crate::draft::black()).await {
            Ok(()) => {
                note = Some(format!(
                    "{} was deleted while it was on the lights, so the lights are black",
                    removed.id
                ));
            }
            Err(error) => {
                note = Some(format!(
                    "{} was deleted while it was on the lights, and they could not be set to black: {error}",
                    removed.id
                ));
            }
        }
        tracing::warn!(stack = %removed.id, "a stack on the lights was deleted");
    }
    tracing::info!(stack = %removed.id, presets_removed, favorite_removed, "a stack was deleted");

    Ok(Json(DeleteStackResponse {
        ok: true,
        id: removed.id,
        presets_removed,
        favorite_removed,
        note,
    }))
}
