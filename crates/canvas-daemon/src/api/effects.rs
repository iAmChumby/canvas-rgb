//! The effect routes: list, detail, activate, props and favourites.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{
    ActivateEffect, EffectDetail, EffectSummary, EffectsResponse, SetFavorite,
};
use canvas_proto::props::{Props, PropertySchema};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;
use crate::user_effects::UserEffects;

/// Every effect's schema: the twelve built-ins first, then the user HTML effects sorted by
/// name. The registry is read once per call: it is cheap (twelve effects) and always
/// reflects the current build.
pub fn schemas(user: &UserEffects) -> Vec<PropertySchema> {
    let mut all: Vec<PropertySchema> =
        canvas_effects::registry().into_iter().map(|f| f().schema()).collect();
    all.extend(user.schemas());
    all
}

/// Finds a schema by id, case-insensitively (the CLI matches names case-insensitively).
pub fn find_schema(user: &UserEffects, id: &str) -> Option<PropertySchema> {
    let id = id.trim();
    let all = schemas(user);
    all.iter()
        .find(|s| s.id.eq_ignore_ascii_case(id))
        .or_else(|| {
            // The CLI accepts the effect's display name too.
            all.iter().find(|s| s.name.eq_ignore_ascii_case(id))
        })
        .cloned()
}

/// `GET /api/v1/effects`.
pub async fn list_effects(State(app): State<Arc<App>>) -> Json<EffectsResponse> {
    let store = app.store.read().await;
    let favorites = store.effects.favorites.clone();
    let effects: Vec<EffectSummary> = schemas(&app.user_effects)
        .into_iter()
        .map(|s| EffectSummary {
            favorite: favorites.iter().any(|f| f == &s.id),
            id: s.id,
            name: s.name,
            description: s.description,
            needs: s.needs,
        })
        .collect();
    let active = store.active_layout().and_then(|l| l.effect.as_ref().map(|e| e.id.clone()));
    Json(EffectsResponse { effects, active })
}

/// `GET /api/v1/effects/{id}`.
pub async fn get_effect(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<EffectDetail>> {
    let schema = find_schema(&app.user_effects, &id)
        .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id or name {id}")))?;
    let store = app.store.read().await;
    let favorite = store.effects.favorites.iter().any(|f| f == &schema.id);
    let user_presets = store.effects.presets.get(&schema.id).cloned().unwrap_or_default();
    let props = current_props(&store, &schema);
    Ok(Json(EffectDetail { schema, favorite, user_presets, props }))
}

/// The values in force for an effect: the active layout's when it is the active effect,
/// otherwise the saved ones, otherwise the schema defaults.
fn current_props(store: &crate::storage::Store, schema: &PropertySchema) -> Props {
    let active = store.active_layout().and_then(|l| l.effect.clone());
    if let Some(active) = active {
        if active.id == schema.id {
            return active.props;
        }
    }
    let mut props = store.effects.saved.get(&schema.id).cloned().unwrap_or_default();
    // Fill in the schema defaults for anything the user never set.
    for (key, def) in schema.defaults() {
        props.entry(key).or_insert(def);
    }
    props
}

/// `POST /api/v1/effects/{id}/activate`.
///
/// Activates the effect on the active layout. The props in the body override the saved
/// values.
pub async fn activate(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(request): Json<ActivateEffect>,
) -> ApiResult<Json<serde_json::Value>> {
    let schema = find_schema(&app.user_effects, &id)
        .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id or name {id}")))?;

    let mut props = schema.defaults();
    if let Some(preset_name) = &request.preset {
        let preset = schema
            .presets
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(preset_name))
            .ok_or_else(|| ApiFailure::not_found(format!("no preset named {preset_name}")))?;
        props = preset.props.clone();
    }
    // The saved values win over the defaults, the request wins over both.
    let saved = app
        .store
        .read()
        .await
        .effects
        .saved
        .get(&schema.id)
        .cloned()
        .unwrap_or_default();
    for (k, v) in saved {
        props.insert(k, v);
    }
    for (k, v) in request.props {
        props.insert(k, v);
    }

    // Save the props for next time, then apply the effect.
    {
        let mut store = app
            .store
            .write()
            .await;
        store.effects.saved.insert(schema.id.clone(), props.clone());
        store
            .save_effects()
            .map_err(|e| ApiFailure::internal(format!("could not save the effects: {e}")))?;

        if let Some(layout) = store.active_layout_mut() {
            layout.effect = Some(canvas_proto::layout::ActiveEffect {
                id: schema.id.clone(),
                props: props.clone(),
            });
        }
        store
            .save_layouts()
            .map_err(|e| ApiFailure::internal(format!("could not save the layouts: {e}")))?;
    }

    crate::apply_effect(&app, &schema.id, &props)
        .await
        .map_err(|e| ApiFailure::internal(format!("could not activate the effect: {e}")))?;

    tracing::info!(effect = %schema.id, "the effect was activated");
    Ok(Json(serde_json::json!({ "ok": true, "id": schema.id, "props": props })))
}

/// `PATCH /api/v1/effects/active/props`.
///
/// Changes the props of the effect that is running.
pub async fn patch_active_props(
    State(app): State<Arc<App>>,
    Json(props): Json<Props>,
) -> ApiResult<Json<serde_json::Value>> {
    let (schema, merged) = {
        let store = app.store.read().await;
        let active = store
            .active_layout()
            .and_then(|l| l.effect.clone())
            .ok_or_else(|| ApiFailure::not_found("no effect is active"))?;
        let schema = find_schema(&app.user_effects, &active.id)
            .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id {}", active.id)))?;
        let mut merged = active.props;
        merged.extend(props.clone());
        (schema, merged)
    };

    {
        let mut store = app
            .store
            .write()
            .await;
        store.effects.saved.insert(schema.id.clone(), merged.clone());
        store
            .save_effects()
            .map_err(|e| ApiFailure::internal(format!("could not save the effects: {e}")))?;
        if let Some(layout) = store.active_layout_mut() {
            if let Some(effect) = layout.effect.as_mut() {
                effect.props = merged.clone();
            }
        }
        store
            .save_layouts()
            .map_err(|e| ApiFailure::internal(format!("could not save the layouts: {e}")))?;
    }

    crate::apply_effect(&app, &schema.id, &merged)
        .await
        .map_err(|e| ApiFailure::internal(format!("could not change the props: {e}")))?;

    Ok(Json(serde_json::json!({ "ok": true, "id": schema.id, "props": merged })))
}

/// `PUT /api/v1/effects/{id}/favorite`.
pub async fn put_favorite(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(request): Json<SetFavorite>,
) -> ApiResult<Json<serde_json::Value>> {
    let schema = find_schema(&app.user_effects, &id)
        .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id or name {id}")))?;
    {
        let mut store = app
            .store
            .write()
            .await;
        store.effects.favorites.retain(|f| f != &schema.id);
        if request.favorite {
            store.effects.favorites.push(schema.id.clone());
        }
        store.effects.favorites.sort();
        store
            .save_effects()
            .map_err(|e| ApiFailure::internal(format!("could not save the effects: {e}")))?;
    }
    Ok(Json(serde_json::json!({ "ok": true, "id": schema.id, "favorite": request.favorite })))
}

/// `PUT /api/v1/effects/{id}`.
///
/// Replaces the effect's saved props and presets. Used by the editor's preset editor.
pub async fn put_effect(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(detail): Json<EffectDetail>,
) -> ApiResult<Json<serde_json::Value>> {
    let schema = find_schema(&app.user_effects, &id)
        .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id or name {id}")))?;
    {
        let mut store = app
            .store
            .write()
            .await;
        store.effects.saved.insert(schema.id.clone(), detail.props.clone());
        store
            .effects
            .presets
            .insert(schema.id.clone(), detail.user_presets.clone());
        store.effects.favorites.retain(|f| f != &schema.id);
        if detail.favorite {
            store.effects.favorites.push(schema.id.clone());
        }
        store.effects.favorites.sort();
        store
            .save_effects()
            .map_err(|e| ApiFailure::internal(format!("could not save the effects: {e}")))?;
    }
    Ok(Json(serde_json::json!({ "ok": true, "id": schema.id })))
}

/// The saved presets for every effect, for the editor.
pub fn saved_presets(store: &crate::storage::Store) -> BTreeMap<String, Vec<canvas_proto::props::Preset>> {
    store.effects.presets.clone()
}
