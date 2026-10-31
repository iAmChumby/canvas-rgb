//! The effect routes: list, detail, activate, props and favourites.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use canvas_proto::api::{
    ActivateEffect, DeleteEffectQuery, EffectDetail, EffectSummary, EffectsResponse, FailedEffect, ImportEffect,
    ImportEffectResponse, PresetCreate, PresetReplace, PresetsResponse, SetFavorite,
};
use canvas_proto::props::{Preset, Props, PropertySchema};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;
use crate::user_effects::UserEffects;

/// Every effect's schema: the eleven built-ins first, then the user HTML effects sorted by
/// name. The registry is read once per call: it is cheap (eleven effects) and always
/// reflects the current build.
pub fn schemas(user: &UserEffects) -> Vec<PropertySchema> {
    let mut all: Vec<PropertySchema> =
        canvas_effects::registry().into_iter().map(|f| f().schema()).collect();
    all.extend(user.schemas());
    all
}

/// Finds a schema by id, case-insensitively (the CLI matches names case-insensitively).
///
/// The hidden onboarding effects are found here (so the activate route runs them) but are
/// not in [`schemas`], so they never reach `GET /effects`.
pub fn find_schema(user: &UserEffects, id: &str) -> Option<PropertySchema> {
    let id = id.trim();
    let mut all = schemas(user);
    all.extend(canvas_effects::onboard::hidden_schemas());
    all.iter()
        .find(|s| s.id.eq_ignore_ascii_case(id))
        .or_else(|| {
            // The CLI accepts the effect's display name too.
            all.iter().find(|s| s.name.eq_ignore_ascii_case(id))
        })
        .cloned()
}

/// The user effect files that did not parse, with the parser's message.
fn failed_effects(user: &UserEffects) -> Vec<FailedEffect> {
    user.failures()
        .into_iter()
        .map(|(path, error)| FailedEffect {
            file: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            error,
        })
        .collect()
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
    let failed = failed_effects(&app.user_effects);
    Json(EffectsResponse { effects, active, failed })
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
/// Activates the effect on the active layout. Without a preset the saved values sit on the
/// defaults; with one the preset does. The props in the body override either.
pub async fn activate(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(request): Json<ActivateEffect>,
) -> ApiResult<Json<serde_json::Value>> {
    let schema = find_schema(&app.user_effects, &id)
        .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id or name {id}")))?;

    let mut props = schema.defaults();
    if let Some(preset_name) = &request.preset {
        // A built-in preset first, then the user's own; the save routes keep the two
        // namespaces apart, so a name never matches both.
        let user_presets = app
            .store
            .read()
            .await
            .effects
            .presets
            .get(&schema.id)
            .cloned()
            .unwrap_or_default();
        let preset = schema
            .presets
            .iter()
            .chain(user_presets.iter())
            .find(|p| p.name.eq_ignore_ascii_case(preset_name))
            .ok_or_else(|| ApiFailure::not_found(format!("no preset named {preset_name}")))?;
        // A preset sits on the defaults and never on the saved values, so a key it does not
        // mention is the default, not whatever the last session left behind.
        props.extend(fit_props_to_schema(&schema, preset.props.clone(), &preset.name));
    } else {
        // The saved values win over the defaults.
        let saved = app
            .store
            .read()
            .await
            .effects
            .saved
            .get(&schema.id)
            .cloned()
            .unwrap_or_default();
        props.extend(saved);
    }
    // The request wins over everything.
    props.extend(request.props);

    // The onboarding effects are transient: nothing about them is saved, the layout keeps its
    // own effect, and a timer puts that effect back (see `crate::transient`).
    let transient = canvas_effects::onboard::is_onboard_id(&schema.id);

    // Save the props for next time, then apply the effect.
    if !transient {
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

    // Any activation ends an earlier timer; an onboarding effect arms a new one below.
    app.transient.cancel();
    crate::apply_effect(&app, &schema.id, &props)
        .await
        .map_err(|e| ApiFailure::internal(format!("could not activate the effect: {e}")))?;
    if transient {
        crate::transient::arm(&app, &schema.id);
    }

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

/// The longest preset name, in characters.
const PRESET_NAME_MAX: usize = 40;
/// The most properties one preset may hold, and the most bytes its props may take as JSON.
/// Far above any effect's schema; they stop a client from filling `effects.json`.
const PRESET_MAX_KEYS: usize = 64;
const PRESET_MAX_BYTES: usize = 8 * 1024;
/// The most user presets one effect may have.
const PRESETS_PER_EFFECT_MAX: usize = 100;

/// Invisible format characters (bidirectional controls, zero-width marks, the soft hyphen,
/// the byte order mark). `char::is_control` does not see them; in a name they can make two
/// presets look the same or reorder the text around them.
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// A preset name as it is stored: trimmed, not empty, short, no control or format characters.
fn clean_preset_name(name: &str) -> Result<String, ApiFailure> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiFailure::validation("a preset needs a name"));
    }
    if name.chars().count() > PRESET_NAME_MAX {
        return Err(ApiFailure::validation(format!(
            "a preset name is at most {PRESET_NAME_MAX} characters"
        )));
    }
    if name.chars().any(|c| c.is_control() || is_format_char(c)) {
        return Err(ApiFailure::validation(
            "a preset name has no control, bidirectional or invisible characters",
        ));
    }
    Ok(name.to_string())
}

/// The props of a request, within the caps.
fn check_preset_caps(props: &Props) -> Result<(), ApiFailure> {
    if props.len() > PRESET_MAX_KEYS {
        return Err(ApiFailure::validation(format!(
            "a preset holds at most {PRESET_MAX_KEYS} properties"
        )));
    }
    let bytes = serde_json::to_vec(props).map(|v| v.len()).unwrap_or(usize::MAX);
    if bytes > PRESET_MAX_BYTES {
        return Err(ApiFailure::validation(format!(
            "a preset's values take at most {PRESET_MAX_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Fits saved props to the effect's schema as it is now: a key the effect no longer has, a value
/// of the wrong type and a choice that is not an option are dropped, and a number is clamped to
/// the schema's range. A preset outlives the file it was made for (an effect can be imported
/// again with other properties), so this runs on every use and every save.
pub fn fit_props_to_schema(schema: &PropertySchema, props: Props, what: &str) -> Props {
    use canvas_proto::props::{PropKind, PropValue};
    let mut fitted = Props::new();
    for (key, value) in props {
        let Some(def) = schema.properties.iter().find(|d| d.key == key) else {
            tracing::warn!(effect = %schema.id, preset = %what, key = %key, "dropped a preset value the effect no longer has");
            continue;
        };
        let kept = match (&def.kind, value) {
            (PropKind::Number { min, max, .. }, PropValue::Number(v)) if v.is_finite() => {
                let clamped = v.clamp(*min, *max);
                if clamped != v {
                    tracing::warn!(effect = %schema.id, preset = %what, key = %key, from = v, to = clamped, "clamped a preset value to the effect's range");
                }
                Some(PropValue::Number(clamped))
            }
            (PropKind::Hue { .. }, v @ PropValue::Hue(h)) if h.is_finite() => Some(v),
            (PropKind::Color { .. }, v @ PropValue::Color(_)) => Some(v),
            (PropKind::Bool { .. }, v @ PropValue::Bool(_)) => Some(v),
            (PropKind::Choice { options, .. }, v @ PropValue::Choice(_)) => match &v {
                PropValue::Choice(c) if options.iter().any(|o| o == c) => Some(v),
                _ => None,
            },
            _ => None,
        };
        match kept {
            Some(v) => {
                fitted.insert(key, v);
            }
            None => {
                tracing::warn!(effect = %schema.id, preset = %what, key = %key, "dropped a preset value that does not fit the effect");
            }
        }
    }
    fitted
}

/// Whether a name is taken by a built-in or user preset, ignoring ASCII case. `except` is the
/// user preset being renamed, which may keep its own name in another case.
fn clash(schema: &PropertySchema, user: &[Preset], name: &str, except: Option<&str>) -> bool {
    schema.presets.iter().any(|p| p.name.eq_ignore_ascii_case(name))
        || user.iter().any(|p| {
            p.name.eq_ignore_ascii_case(name)
                && !except.is_some_and(|e| p.name.eq_ignore_ascii_case(e))
        })
}

/// Stores the new list for an effect and writes `effects.json`. When the write fails the list goes
/// back to what it was, so the answer to the client and the memory agree: nothing half-saved.
fn commit_presets(
    store: &mut crate::storage::Store,
    id: &str,
    next: Vec<Preset>,
) -> Result<(), ApiFailure> {
    let before = store.effects.presets.get(id).cloned();
    store.effects.presets.insert(id.to_string(), next);
    if let Err(e) = store.save_effects() {
        match before {
            Some(list) => {
                store.effects.presets.insert(id.to_string(), list);
            }
            None => {
                store.effects.presets.remove(id);
            }
        }
        return Err(ApiFailure::internal(format!("could not save the effects: {e}")));
    }
    Ok(())
}

/// `POST /api/v1/effects/{id}/presets`.
///
/// Saves the props under a name. A name equal, ignoring case, to a built-in or user preset
/// of the effect is a conflict.
pub async fn create_preset(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(request): Json<PresetCreate>,
) -> ApiResult<Json<PresetsResponse>> {
    let schema = find_schema(&app.user_effects, &id)
        .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id or name {id}")))?;
    let name = clean_preset_name(&request.name)?;
    check_preset_caps(&request.props)?;
    let props = fit_props_to_schema(&schema, request.props, &name);
    let mut store = app.store.write().await;
    let mut list = store.effects.presets.get(&schema.id).cloned().unwrap_or_default();
    if clash(&schema, &list, &name, None) {
        return Err(ApiFailure::conflict(format!("{name} is already a preset of this effect")));
    }
    if list.len() >= PRESETS_PER_EFFECT_MAX {
        return Err(ApiFailure::validation(format!(
            "an effect has at most {PRESETS_PER_EFFECT_MAX} presets"
        )));
    }
    list.push(Preset { name, props });
    commit_presets(&mut store, &schema.id, list.clone())?;
    Ok(Json(PresetsResponse { user_presets: list }))
}

/// `PUT /api/v1/effects/{id}/presets/{name}`.
///
/// Overwrites and/or renames one user preset. `expected` is what the client last saw: when the
/// stored props differ, another window changed the preset and the answer is a conflict.
pub async fn replace_preset(
    State(app): State<Arc<App>>,
    Path((id, name)): Path<(String, String)>,
    Json(request): Json<PresetReplace>,
) -> ApiResult<Json<PresetsResponse>> {
    let schema = find_schema(&app.user_effects, &id)
        .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id or name {id}")))?;
    let rename_to = request.rename_to.as_deref().map(clean_preset_name).transpose()?;
    check_preset_caps(&request.props)?;
    let props = fit_props_to_schema(&schema, request.props, &name);
    let mut store = app.store.write().await;
    let mut list = store.effects.presets.get(&schema.id).cloned().unwrap_or_default();
    let index = list
        .iter()
        .position(|p| p.name == name)
        .ok_or_else(|| ApiFailure::not_found(format!("no preset named {name}")))?;
    if list[index].props != request.expected {
        return Err(ApiFailure::conflict(format!("{name} was changed in another window")));
    }
    if let Some(new_name) = &rename_to {
        if clash(&schema, &list, new_name, Some(&name)) {
            return Err(ApiFailure::conflict(format!(
                "{new_name} is already a preset of this effect"
            )));
        }
    }
    list[index].props = props;
    if let Some(new_name) = rename_to {
        list[index].name = new_name;
    }
    commit_presets(&mut store, &schema.id, list.clone())?;
    Ok(Json(PresetsResponse { user_presets: list }))
}

/// `DELETE /api/v1/effects/{id}/presets/{name}`.
pub async fn delete_preset(
    State(app): State<Arc<App>>,
    Path((id, name)): Path<(String, String)>,
) -> ApiResult<Json<PresetsResponse>> {
    let schema = find_schema(&app.user_effects, &id)
        .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id or name {id}")))?;
    let mut store = app.store.write().await;
    let mut list = store.effects.presets.get(&schema.id).cloned().unwrap_or_default();
    let index = list
        .iter()
        .position(|p| p.name == name)
        .ok_or_else(|| ApiFailure::not_found(format!("no preset named {name}")))?;
    list.remove(index);
    commit_presets(&mut store, &schema.id, list.clone())?;
    Ok(Json(PresetsResponse { user_presets: list }))
}

/// `GET /api/v1/effects/{id}/thumbnail`.
///
/// The effect is rendered headless at its schema defaults, on a blocking thread, and cached
/// until the user-effects set changes. The body is the binary format in
/// [`crate::thumbnail`].
pub async fn thumbnail(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<axum::response::Response> {
    let schema = find_schema(&app.user_effects, &id)
        .ok_or_else(|| ApiFailure::not_found(format!("no effect with the id or name {id}")))?;
    let generation = app.user_effects.generation();

    if let Some(bytes) = app.thumbnails.get(generation, &schema.id) {
        return Ok(binary_response(&bytes));
    }
    // A render that was given up is remembered: the library is told the tile was given up
    // instead of asking for it again on every open.
    if let Some(why) = app.thumbnails.failure(generation, &schema.id) {
        return Err(ApiFailure::service_unavailable(why));
    }

    let user = app.user_effects.clone();
    let effect_id = schema.id.clone();
    let props = schema.defaults();
    let rendering = Arc::clone(&app);
    // The permit is taken before the render and handed to the render task, so it is held
    // for as long as the render runs: a request whose client goes away does not let a
    // third render in while this one is still going.
    let slot = app.thumbnails.acquire_slot().await;
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let _slot = slot;
        // Another render may have finished while this one waited for the slot.
        if let Some(bytes) = rendering.thumbnails.get(generation, &effect_id) {
            return Ok((*bytes).clone());
        }
        if let Some(why) = rendering.thumbnails.failure(generation, &effect_id) {
            return Err(why);
        }
        let mut effect = crate::create_effect_for_tile(&user, &effect_id)?;
        crate::thumbnail::render(effect.as_mut(), &props)
    })
    .await
    .map_err(|e| ApiFailure::internal(format!("the thumbnail render stopped: {e}")))?;

    match bytes {
        Ok(bytes) => {
            let bytes = Arc::new(bytes);
            app.thumbnails.put(generation, &schema.id, bytes.clone());
            Ok(binary_response(&bytes))
        }
        Err(why) => {
            // The render was given up. It is remembered, so the next open of the library
            // does not render it again for the whole budget, and the answer says what
            // happened instead of being a bare 500.
            let why = format!("the tile for {id} was given up: {why}");
            app.thumbnails.put_failure(generation, &schema.id, why.clone());
            Err(ApiFailure::service_unavailable(why))
        }
    }
}

fn binary_response(bytes: &[u8]) -> axum::response::Response {
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        bytes.to_vec(),
    )
        .into_response()
}

/// The saved presets for every effect, for the editor.
pub fn saved_presets(store: &crate::storage::Store) -> BTreeMap<String, Vec<canvas_proto::props::Preset>> {
    store.effects.presets.clone()
}

/// `POST /api/v1/effects/import`.
///
/// Adds a SignalRGB-format HTML effect to the effects folder.
///
/// The file name is checked first, then the file is parsed with the builder the loader
/// uses: a file that does not parse is a validation error and nothing is written. The
/// file lands in the folder atomically (a temp file in the same folder, then a rename),
/// the folder is rescanned and the effect's summary comes back.
pub async fn import_effect(
    State(app): State<Arc<App>>,
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
) -> ApiResult<Json<ImportEffectResponse>> {
    // The body is read as bytes, so an oversized body is answered with the API's own
    // error shape and not with axum's plain text.
    let bytes = body.map_err(|rejection| match rejection {
        axum::extract::rejection::BytesRejection::FailedToBufferBody(
            axum::extract::rejection::FailedToBufferBody::LengthLimitError(_),
        ) => ApiFailure::new(
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            canvas_proto::api::error_codes::VALIDATION,
            format!("the request body is over the limit of {IMPORT_BODY_LIMIT} bytes"),
        ),
        other => ApiFailure::validation(format!("the request body could not be read: {other}")),
    })?;
    let request: ImportEffect = serde_json::from_slice(&bytes)
        .map_err(|e| ApiFailure::validation(format!("the body is not an import request: {e}")))?;

    // The file name is checked before anything else.
    crate::user_effects::check_file_name(&request.file_name).map_err(ApiFailure::validation)?;

    if request.html.len() > canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES {
        return Err(ApiFailure::validation(format!(
            "the effect is {} bytes; the limit is {}",
            request.html.len(),
            canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES
        )));
    }

    // One import or one delete at a time, so the two never race each other over the
    // same file.
    let _guard = app.effect_writes.lock().await;

    // The file that is already there, if any. The names are compared without regard to
    // case: on NTFS two names that differ only in case are the same file, and the
    // clash must be found the same way on every file system.
    let existing = crate::user_effects::existing_file(&app.effects_dir, &request.file_name);
    if existing.is_some() && !request.replace {
        return Err(ApiFailure::conflict(format!(
            "{} is already installed; ask for a replace to overwrite it",
            request.file_name
        )));
    }
    // The file that is really written: the one that is there, or the one that is asked
    // for.
    let path = existing
        .clone()
        .unwrap_or_else(|| app.effects_dir.join(&request.file_name));
    // The id comes from the file's name, so it is the id the loader gives it.
    let id = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .map(|stem| format!("{}{stem}", crate::user_effects::USER_EFFECT_PREFIX))
        .ok_or_else(|| ApiFailure::validation("the file has no name"))?;

    // The file is parsed before anything is written: a broken file is refused and the
    // effects folder stays as it was. The parse runs the effect's script, so it goes
    // on a blocking thread and not on a tokio worker.
    let parse_id = id.clone();
    let parse_html = request.html.clone();
    let effect = tokio::task::spawn_blocking(move || {
        (crate::user_effects::html_builder())(&parse_id, &parse_html)
    })
    .await
    .map_err(|e| ApiFailure::internal(format!("the parse task stopped: {e}")))?
    .map_err(|e| ApiFailure::validation(format!("the effect does not parse: {e}")))?;
    let schema = effect.schema();

    // The write. A new file is created with a link, so a file that appears between the
    // check and the write is never overwritten.
    if request.replace {
        crate::user_effects::write_atomic(&path, request.html.as_bytes())
            .map_err(|e| ApiFailure::internal(format!("could not write the effect file: {e}")))?;
    } else {
        crate::user_effects::write_atomic_new(&path, request.html.as_bytes()).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                ApiFailure::conflict(format!(
                    "{} appeared while the effect was being imported; ask for a replace to overwrite it",
                    request.file_name
                ))
            } else {
                ApiFailure::internal(format!("could not write the effect file: {e}"))
            }
        })?;
    }

    // The folder is rescanned, so the effect is in the library at once. The rescan
    // parses the file, so it goes on a blocking thread too.
    {
        let user = app.user_effects.clone();
        let dir = app.effects_dir.clone();
        tokio::task::spawn_blocking(move || user.rescan(&dir))
            .await
            .map_err(|e| ApiFailure::internal(format!("the rescan task stopped: {e}")))?;
    }

    let favorite = {
        let store = app.store.read().await;
        store.effects.favorites.iter().any(|f| f == &id)
    };
    tracing::info!(effect = %id, replaced = existing.is_some(), "a user effect was imported");

    Ok(Json(ImportEffectResponse {
        effect: EffectSummary {
            id,
            name: schema.name,
            description: schema.description,
            needs: schema.needs,
            favorite,
        },
        replaced: existing.is_some(),
    }))
}

/// `DELETE /api/v1/effects/{id}`.
///
/// Removes a user HTML effect. Only an `html:` id can be deleted: the built-in effects
/// are part of the app.
///
/// The file goes from the effects folder and the folder is rescanned, so the effect
/// leaves the library at once.
pub async fn delete_effect(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(query): Query<DeleteEffectQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let stem = id
        .strip_prefix(crate::user_effects::USER_EFFECT_PREFIX)
        .ok_or_else(|| {
            ApiFailure::validation(format!(
                "{id} is not a user effect; only html: effects can be deleted"
            ))
        })?;
    // The stem comes out of the URL, so it is checked like any other file name: no
    // folder, no `..`, no reserved Windows name.
    let file_name = format!("{stem}.html");
    crate::user_effects::check_file_name(&file_name).map_err(ApiFailure::validation)?;

    // One import or one delete at a time.
    let _guard = app.effect_writes.lock().await;

    // The file is looked up without regard to case, the same way the import looks for
    // a clash.
    let path = crate::user_effects::existing_file(&app.effects_dir, &file_name)
        .ok_or_else(|| ApiFailure::not_found(format!("no user effect with the id {id}")))?;
    std::fs::remove_file(&path)
        .map_err(|e| ApiFailure::internal(format!("could not remove the effect file: {e}")))?;

    // The folder is rescanned, so the effect leaves the library at once.
    {
        let user = app.user_effects.clone();
        let dir = app.effects_dir.clone();
        tokio::task::spawn_blocking(move || user.rescan(&dir))
            .await
            .map_err(|e| ApiFailure::internal(format!("the rescan task stopped: {e}")))?;
    }
    tracing::info!(effect = %id, "a user effect was deleted");

    // The effect's presets go with it unless the owner chose to keep them: left behind they
    // would come back, unseen, the day the same file is imported again.
    let mut presets_removed = 0usize;
    if !query.keep_presets {
        let mut store = app.store.write().await;
        if let Some(list) = store.effects.presets.remove(&id) {
            presets_removed = list.len();
            if let Err(e) = store.save_effects() {
                store.effects.presets.insert(id.clone(), list);
                return Err(ApiFailure::internal(format!(
                    "the effect file is gone but its presets could not be removed: {e}"
                )));
            }
        }
    }

    // The effect was the active one: the daemon falls back the same way it does when
    // the file is missing. The active layout still names it, so the restore path
    // cannot build it any more and says so, exactly as it does at start.
    let active = app.active_layout().await.and_then(|l| l.effect);
    if let Some(active) = active {
        if active.id == id {
            tracing::warn!(effect = %id, "the active user effect was deleted");
            if let Err(e) = crate::apply_effect(&app, &active.id, &active.props).await {
                tracing::warn!(
                    effect = %active.id,
                    error = %e,
                    "the deleted effect could not be restored"
                );
            }
        }
    }

    Ok(Json(serde_json::json!({ "ok": true, "id": id, "presets_removed": presets_removed })))
}

/// The largest request body the import route accepts, in bytes.
///
/// The body is JSON, and a file byte can take up to six bytes in it: a byte that has
/// to be escaped becomes `\u00XX`. A file of [`EFFECT_IMPORT_MAX_BYTES`] bytes is
/// therefore at most six times that in the body, plus a little for the keys and the
/// file name. The limit is that, with 4 KiB of room.
pub const IMPORT_BODY_LIMIT: usize =
    6 * canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES + 4096;
