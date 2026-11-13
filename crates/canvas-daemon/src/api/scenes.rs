//! The scenes routes (Scenes plan, Task 6): list, create, replace, delete, set, one group's
//! values and brightness, and the lights flash.
//!
//! Every route that changes a scene takes the scene order first ([`crate::scenes::SceneLock`]) and
//! holds it from reading the revision to handing the engine the result, so two windows never
//! interleave (Review Focus 4). A scene is saved before the engine hears of it, and a save that
//! fails puts the old scene back.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use canvas_proto::api::{
    FlashRequest, GroupBrightnessPut, SceneActivePut, SceneCreate, ScenePut, ScenesResponse,
};
use canvas_proto::props::Props;
use canvas_proto::scene::{
    validate_scene, Group, Plays, Scene, GROUP_BRIGHTNESS_MAX, GROUP_NAME_MAX, SCENE_NAME_MAX,
};
use canvas_proto::stack::STACK_ID_PREFIX;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::scenes::{self, fresh_id, slug, SceneLock};
use crate::state::App;

/// The id no scene may take: `PUT /scenes/active` sets the active scene, so a scene with that id
/// could never be reached by `PUT /scenes/{id}`.
const RESERVED_SCENE_ID: &str = "active";

/// The most lights one flash request may name. A house has tens of lights; a list past this is a
/// mistake (or an attack on the lookup below), so it is refused before any light is looked up.
pub const FLASH_LIGHTS_MAX: usize = 256;

/// A route's failure: the API's usual error, or a `409` that carries the scene as it is now.
pub enum SceneFailure {
    Api(ApiFailure),
    Conflict(Box<Scene>),
}

impl From<ApiFailure> for SceneFailure {
    fn from(failure: ApiFailure) -> Self {
        Self::Api(failure)
    }
}

impl IntoResponse for SceneFailure {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Api(failure) => failure.into_response(),
            Self::Conflict(scene) => (
                axum::http::StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": canvas_proto::api::error_codes::CONFLICT,
                    "message": "this scene was changed in another window; its current state is in `scene`",
                    "scene": scene,
                })),
            )
                .into_response(),
        }
    }
}

/// Why a scene could not be saved: the file waits or is blocked (the owner's to fix, a 503), or
/// the disk refused (a 500).
fn save_failure(store: &crate::storage::Store, error: std::io::Error) -> ApiFailure {
    if store.scenes_pending.is_some() || store.scenes_blocked || store.lights_blocked {
        ApiFailure::service_unavailable(format!("the scenes cannot be saved yet: {error}"))
    } else {
        ApiFailure::internal(format!("could not save the scenes: {error}"))
    }
}

/// The whole answer of the list, the set and the delete routes.
async fn response(app: &App) -> ScenesResponse {
    let store = app.store.read().await;
    let active = scenes::active_scene(&store.scenes);
    ScenesResponse {
        active: active.id.clone(),
        scenes: store.scenes.scenes.clone(),
        group_states: scenes::group_states(app, active),
    }
}

/// Replaces the scene with `new`'s id and saves. On a failed save the old scene is back and the
/// error says why. Returns the scene it replaced.
async fn replace_saved(app: &App, new: Scene) -> ApiResult<Scene> {
    let mut store = app.store.write().await;
    let Some(slot) = store.scenes.scenes.iter().position(|s| s.id == new.id) else {
        return Err(ApiFailure::not_found(format!("no scene with the id {}", new.id)));
    };
    let old = std::mem::replace(&mut store.scenes.scenes[slot], new);
    if let Err(e) = store.save_scenes() {
        store.scenes.scenes[slot] = old;
        return Err(save_failure(&store, e));
    }
    Ok(old)
}

/// A copy of the scene with this id, if there is one.
async fn find_scene(app: &App, id: &str) -> ApiResult<Scene> {
    app.store
        .read()
        .await
        .scenes
        .scenes
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or_else(|| ApiFailure::not_found(format!("no scene with the id {id}")))
}

async fn is_active(app: &App, id: &str) -> bool {
    scenes::active_scene(&app.store.read().await.scenes).id == id
}

/// `GET /api/v1/scenes`.
pub async fn list_scenes(State(app): State<Arc<App>>) -> Json<ScenesResponse> {
    Json(response(&app).await)
}

/// `POST /api/v1/scenes`.
pub async fn create_scene(
    State(app): State<Arc<App>>,
    Json(request): Json<SceneCreate>,
) -> ApiResult<Json<Scene>> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiFailure::validation("a scene needs a name"));
    }
    if name.chars().count() > SCENE_NAME_MAX {
        return Err(ApiFailure::validation(format!("a scene's name is at most {SCENE_NAME_MAX} characters")));
    }
    let _lock = app.scene.lock().await;
    let mut store = app.store.write().await;
    let mut taken: Vec<&str> = store.scenes.scenes.iter().map(|s| s.id.as_str()).collect();
    taken.push(RESERVED_SCENE_ID);
    let id = fresh_id(&slug(&name), taken);
    let groups = match &request.copy_of {
        Some(source) => {
            let source = store
                .scenes
                .scenes
                .iter()
                .find(|s| &s.id == source)
                .ok_or_else(|| ApiFailure::not_found(format!("no scene with the id {source}")))?;
            // Fresh group ids: none of them is a group id any scene has.
            let mut used: Vec<String> =
                store.scenes.scenes.iter().flat_map(|s| s.groups.iter().map(|g| g.id.clone())).collect();
            let mut copied = Vec::with_capacity(source.groups.len());
            for group in &source.groups {
                let fresh = fresh_id(&slug(&group.name), used.iter().map(String::as_str));
                used.push(fresh.clone());
                copied.push(Group { id: fresh, ..group.clone() });
            }
            copied
        }
        None => scenes::everything_group(&store.lights.lights)
            .map(|g| Group { plays: Plays::Off, ..g })
            .into_iter()
            .collect(),
    };
    let scene = Scene { id, name, rev: 1, groups };
    store.scenes.scenes.push(scene.clone());
    if let Err(e) = store.save_scenes() {
        store.scenes.scenes.pop();
        return Err(save_failure(&store, e));
    }
    Ok(Json(scene))
}

/// `PUT /api/v1/scenes/{id}`.
pub async fn put_scene(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(put): Json<ScenePut>,
) -> Result<Json<Scene>, SceneFailure> {
    let lock = app.scene.lock().await;
    let old = find_scene(&app, &id).await?;
    if put.scene.id != id {
        return Err(ApiFailure::validation("the scene's id must be the id in the path").into());
    }
    if old.rev != put.expected_rev {
        return Err(SceneFailure::Conflict(Box::new(old)));
    }
    let mut new = put.scene;
    new.rev = old.rev + 1;
    check_names(&new)?;
    {
        let store = app.store.read().await;
        validate_scene(&new, &store.lights.lights).map_err(|e| ApiFailure::validation(e.to_string()))?;
    }
    scenes::check_html_limit(&app, &new).map_err(ApiFailure::validation)?;
    fit_group_props(&app, &mut new)?;
    replace_saved(&app, new.clone()).await?;
    if is_active(&app, &id).await {
        let _replace = crate::draft::hold_for_replace(&app).await;
        scenes::apply_group_change(&app, &lock, &old, &new).await;
    }
    Ok(Json(new))
}

/// A scene and its groups have a name, within the length the create route keeps.
fn check_names(scene: &Scene) -> ApiResult<()> {
    if scene.name.trim().is_empty() {
        return Err(ApiFailure::validation("a scene needs a name"));
    }
    if scene.name.chars().count() > SCENE_NAME_MAX {
        return Err(ApiFailure::validation(format!("a scene's name is at most {SCENE_NAME_MAX} characters")));
    }
    for group in &scene.groups {
        if group.name.trim().is_empty() {
            return Err(ApiFailure::validation(format!("group {} needs a name", group.id)));
        }
        if group.name.chars().count() > GROUP_NAME_MAX {
            return Err(ApiFailure::validation(format!("a group's name is at most {GROUP_NAME_MAX} characters")));
        }
    }
    Ok(())
}

/// The caps and the schema fit a props patch applies, for every group of a whole scene. A stack
/// keeps its namespaced values as they are (its own builder owns them), but they pass the caps.
fn fit_group_props(app: &App, scene: &mut Scene) -> Result<(), ApiFailure> {
    for group in &mut scene.groups {
        let Plays::Effect { effect, props } = &mut group.plays else { continue };
        crate::api::effects::check_props_caps(props)?;
        if effect.starts_with(STACK_ID_PREFIX) {
            continue;
        }
        if let Some(schema) = crate::api::effects::find_schema(&app.user_effects, &app.stacks, effect) {
            *props = crate::api::effects::fit_props_to_schema(&schema, std::mem::take(props), "the group");
        }
    }
    Ok(())
}

/// `DELETE /api/v1/scenes/{id}`.
pub async fn delete_scene(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<ScenesResponse>> {
    let lock = app.scene.lock().await;
    let was_active = {
        let mut store = app.store.write().await;
        let Some(slot) = store.scenes.scenes.iter().position(|s| s.id == id) else {
            return Err(ApiFailure::not_found(format!("no scene with the id {id}")));
        };
        if store.scenes.scenes.len() == 1 {
            return Err(ApiFailure::conflict("the last scene cannot be deleted"));
        }
        let before = store.scenes.clone();
        let was_active = store.scenes.active == id;
        store.scenes.scenes.remove(slot);
        if was_active {
            store.scenes.active = store.scenes.scenes[0].id.clone();
        }
        if let Err(e) = store.save_scenes() {
            store.scenes = before;
            return Err(save_failure(&store, e));
        }
        was_active
    };
    if was_active {
        let _replace = crate::draft::hold_for_replace(&app).await;
        scenes::apply_with(&app, &lock, true).await;
    }
    Ok(Json(response(&app).await))
}

/// `PUT /api/v1/scenes/active`.
///
/// The scene that is set must be able to run: a scene with more HTML groups than a scene may
/// have is refused, before the lights change.
pub async fn put_active(
    State(app): State<Arc<App>>,
    Json(request): Json<SceneActivePut>,
) -> ApiResult<Json<ScenesResponse>> {
    let lock = app.scene.lock().await;
    {
        let mut store = app.store.write().await;
        let Some(scene) = store.scenes.scenes.iter().find(|s| s.id == request.id) else {
            return Err(ApiFailure::not_found(format!("no scene with the id {}", request.id)));
        };
        // The scene must not run more HTML groups than a scene may: it is about to be on the
        // lights.
        scenes::check_html_limit(&app, scene).map_err(ApiFailure::validation)?;
        let before = std::mem::replace(&mut store.scenes.active, request.id.clone());
        if let Err(e) = store.save_scenes() {
            store.scenes.active = before;
            return Err(save_failure(&store, e));
        }
    }
    let _replace = crate::draft::hold_for_replace(&app).await;
    scenes::apply_with(&app, &lock, true).await;
    Ok(Json(response(&app).await))
}

/// Finds the group index of `group` in `scene`, or a 404.
fn group_index(scene: &Scene, group: &str) -> ApiResult<usize> {
    scene
        .groups
        .iter()
        .position(|g| g.id == group)
        .ok_or_else(|| ApiFailure::not_found(format!("scene {} has no group {group}", scene.id)))
}

/// `PATCH /api/v1/scenes/{id}/groups/{group}/props`.
///
/// The scene order, then the draft gate: the order every route takes. A group a takeover holds
/// keeps the draft; the change is in the scene, and the takeover's restore puts the scene back
/// with the change.
pub async fn patch_group_props(
    State(app): State<Arc<App>>,
    Path((id, group)): Path<(String, String)>,
    Json(patch): Json<Props>,
) -> ApiResult<Json<Scene>> {
    crate::api::effects::check_props_caps(&patch)?;
    let lock = app.scene.lock().await;
    let _gate = app.draft.gate().lock().await;
    let old = find_scene(&app, &id).await?;
    let at = group_index(&old, &group)?;
    let Plays::Effect { effect, props: current } = &old.groups[at].plays else {
        return Err(ApiFailure::conflict(format!("group {group} plays nothing; pick an effect first")));
    };
    if effect.starts_with(STACK_ID_PREFIX) {
        return Err(ApiFailure::conflict(format!(
            "{effect} is a stack; change its values in the stack builder, not with this route"
        )));
    }
    let patch = match crate::api::effects::find_schema(&app.user_effects, &app.stacks, effect) {
        Some(schema) => crate::api::effects::fit_props_to_schema(&schema, patch, "the group"),
        None => patch,
    };
    let mut merged = current.clone();
    merged.extend(patch);
    let mut new = old.clone();
    new.rev = old.rev + 1;
    new.groups[at].plays = Plays::Effect { effect: effect.clone(), props: merged.clone() };
    replace_saved(&app, new.clone()).await?;
    if is_active(&app, &id).await {
        // Through the same path as a whole-scene save, so what the engine runs and what a repeat
        // device list compares against stay one thing.
        scenes::apply_group_change(&app, &lock, &old, &new).await;
    }
    Ok(Json(new))
}

/// `PUT /api/v1/scenes/{id}/groups/{group}/brightness`.
///
/// The scene order, then the draft gate: the order every route takes. A group a takeover holds
/// keeps the draft; the change is in the scene, and the takeover's restore puts the scene back
/// with the change.
pub async fn put_group_brightness(
    State(app): State<Arc<App>>,
    Path((id, group)): Path<(String, String)>,
    Json(request): Json<GroupBrightnessPut>,
) -> ApiResult<Json<Scene>> {
    if request.brightness > GROUP_BRIGHTNESS_MAX {
        return Err(ApiFailure::validation(format!("brightness is 0 to {GROUP_BRIGHTNESS_MAX}")));
    }
    let lock = app.scene.lock().await;
    let _gate = app.draft.gate().lock().await;
    let old = find_scene(&app, &id).await?;
    let at = group_index(&old, &group)?;
    let mut new = old.clone();
    new.rev = old.rev + 1;
    new.groups[at].brightness = request.brightness;
    replace_saved(&app, new.clone()).await?;
    if is_active(&app, &id).await {
        scenes::apply_group_change(&app, &lock, &old, &new).await;
    }
    Ok(Json(new))
}

/// What `POST /effects/{id}/activate` does to the active scene: the groups it names (or every
/// group) play `effect` with `props`. `Ok(None)` when no group changes; a 503 while the scenes are
/// held in memory for the migration. Validates the result; the
/// caller saves it with [`commit_activation`] under the same lock.
pub async fn plan_activation(
    app: &App,
    effect: &str,
    props: &Props,
    groups: Option<&[String]>,
) -> ApiResult<Option<(Scene, Scene)>> {
    let old = {
        let store = app.store.read().await;
        if store.scenes_pending.is_some() {
            // The scenes are only in memory until the first device list; they are written (from
            // the layouts) when it arrives, so a change made now would be lost. Say so rather than
            // answer ok with the lights unchanged.
            tracing::info!("the scenes are not written yet; the effect is not set on a group");
            return Err(ApiFailure::service_unavailable(
                "the scenes are not written yet (the app is still waiting for its devices); try again in a moment",
            ));
        }
        scenes::active_scene(&store.scenes).clone()
    };
    let targets: Vec<usize> = match groups {
        None => (0..old.groups.len()).collect(),
        Some(names) => {
            let mut found = Vec::new();
            for name in names {
                let at = old
                    .groups
                    .iter()
                    .position(|g| g.name.eq_ignore_ascii_case(name) || g.id == *name)
                    .ok_or_else(|| ApiFailure::not_found(format!("scene {} has no group named {name}", old.name)))?;
                if !found.contains(&at) {
                    found.push(at);
                }
            }
            found
        }
    };
    let mut new = old.clone();
    for at in targets {
        new.groups[at].plays = Plays::Effect { effect: effect.to_string(), props: props.clone() };
    }
    if new.groups == old.groups {
        return Ok(None);
    }
    new.rev = old.rev + 1;
    scenes::check_html_limit(app, &new).map_err(ApiFailure::validation)?;
    Ok(Some((old, new)))
}

/// Saves and applies what [`plan_activation`] planned.
pub async fn commit_activation(
    app: &Arc<App>,
    lock: &SceneLock<'_>,
    planned: (Scene, Scene),
) -> ApiResult<()> {
    let (old, new) = planned;
    replace_saved(app, new.clone()).await?;
    scenes::apply_group_change(app, lock, &old, &new).await;
    Ok(())
}

/// `POST /api/v1/lights/flash`.
///
/// The identify override with several runs: the named lights white at full brightness, every other
/// LED black, under the same watchdog. `stop: true` ends it.
pub async fn flash_lights(
    State(app): State<Arc<App>>,
    Json(request): Json<FlashRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::transient::touch(&app);
    if request.stop {
        if app.identify.stop() {
            crate::identify::restore(&app).await;
        }
        return Ok(Json(serde_json::json!({ "ok": true })));
    }
    if request.lights.is_empty() {
        return Err(ApiFailure::validation("name at least one light to flash"));
    }
    if request.lights.len() > FLASH_LIGHTS_MAX {
        return Err(ApiFailure::validation(format!("a flash names at most {FLASH_LIGHTS_MAX} lights")));
    }
    let devices = app.devices.read().await.clone();
    let mut runs = Vec::new();
    {
        let store = app.store.read().await;
        for id in &request.lights {
            let light = store
                .lights
                .lights
                .iter()
                .find(|l| &l.id == id)
                .ok_or_else(|| ApiFailure::not_found(format!("no light with the id {id}")))?;
            // A light of a device that is gone has nothing to flash.
            let Some(device) = devices.iter().find(|d| d.connected && d.id == light.device_id) else {
                continue;
            };
            let Some(zone) = device.zones.iter().find(|z| z.index == light.zone_index) else {
                continue;
            };
            let (start, count) = match light.leds {
                Some(slice) => {
                    let start = slice.start.min(zone.led_count);
                    (start, slice.count.min(zone.led_count - start))
                }
                None => (0, zone.led_count),
            };
            if count > 0 {
                runs.push(crate::identify::Run {
                    device: device.id.clone(),
                    zone: zone.index,
                    led: start,
                    count,
                });
            }
        }
    }
    if runs.is_empty() {
        return Err(ApiFailure::conflict("none of these lights is connected"));
    }
    tracing::info!(lights = request.lights.len(), runs = runs.len(), "light flash request");
    let start = app.identify.start_runs(runs, std::time::Instant::now());
    // Send the first frames now; the ticker repeats them and runs the watchdog.
    if let crate::identify::Tick::Frames(frames) = app.identify.tick(std::time::Instant::now(), &devices) {
        app.identify.push(frames);
    }
    if start {
        crate::identify::spawn_ticker(app.clone());
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
