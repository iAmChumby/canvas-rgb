//! Scene store operations: the lights the devices bring, the repair of a damaged
//! `scenes.json`, and the default scene.
//!
//! Ids are plain and deterministic, like the layout ids before them (`default`, then the owner's
//! own names): a slug of a name, with `-2`, `-3` … when it is taken. Nothing here draws a random
//! number, so a test can name the id it expects.

use std::collections::HashSet;

use canvas_proto::device::{DeviceInfo, ZoneInfo};
use canvas_proto::props::Props;
use canvas_proto::scene::{
    zone_light_id, Arrangement, Group, GroupMode, Light, LightId, LightsFile, Plays, Scene,
    ScenesFile, DEFAULT_GROUP_NAME, DEFAULT_SCENE_NAME, GROUP_BRIGHTNESS_MAX, GROUP_NAME_MAX,
    LIGHTS_SCHEMA, SCENES_SCHEMA, SCENE_HTML_MAX, SCENE_NAME_MAX, HTML_EFFECT_PREFIX,
};

/// What the default scene plays: the effect the first run turns on, so the lights are lit.
pub const DEFAULT_EFFECT: &str = crate::first_run::FIRST_RUN_EFFECT;

/// A lowercase, dash-separated id from a name: `"Living room"` becomes `living-room`. A name
/// with nothing to keep becomes `scene`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "scene".into()
    } else {
        out
    }
}

/// `base`, or `base-2`, `base-3` … the first one `taken` does not hold. Deterministic.
pub fn fresh_id<'a>(base: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: HashSet<&str> = taken.into_iter().collect();
    if !taken.contains(base) {
        return base.to_string();
    }
    (2u32..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken.contains(id.as_str()))
        .expect("a free id exists")
}

/// The one group the default scene holds: every light, in list order, flowing across. `None`
/// when there are no lights (a group needs a member).
pub fn everything_group(lights: &[Light]) -> Option<Group> {
    (!lights.is_empty()).then(|| Group {
        id: slug(DEFAULT_GROUP_NAME),
        name: DEFAULT_GROUP_NAME.into(),
        members: lights.iter().map(|l| l.id.clone()).collect(),
        plays: Plays::Effect { effect: DEFAULT_EFFECT.into(), props: Props::new() },
        mode: GroupMode::FlowAcross,
        brightness: GROUP_BRIGHTNESS_MAX,
    })
}

/// The scene made on a first run and when no scene is left: `Everything`, holding every light.
pub fn default_scene(lights: &[Light]) -> Scene {
    Scene {
        id: slug(DEFAULT_SCENE_NAME),
        name: DEFAULT_SCENE_NAME.into(),
        rev: 1,
        groups: everything_group(lights).into_iter().collect(),
    }
}

/// A scenes file of one default scene, active.
pub fn default_scenes(lights: &LightsFile) -> ScenesFile {
    let scene = default_scene(&lights.lights);
    ScenesFile { schema_version: SCENES_SCHEMA, active: scene.id.clone(), scenes: vec![scene] }
}

/// An empty lights file.
pub fn empty_lights() -> LightsFile {
    LightsFile { schema_version: LIGHTS_SCHEMA, lights: Vec::new() }
}

/// The light that is a whole zone, with the arrangement its kind starts as.
fn whole_zone_light(device: &DeviceInfo, zone: &ZoneInfo) -> Light {
    Light {
        id: zone_light_id(&device.id, zone.index),
        device_id: device.id.clone(),
        zone_index: zone.index,
        leds: None,
        name: None,
        arrangement: Arrangement::default_for(&zone.kind),
        reversed: false,
    }
}

/// Adds a light for every zone with `led_count > 0` that has none. Never removes a light.
/// Returns the ids added. New lights join no group.
///
/// A zone has a light when any light names its device and zone index, whole or a slice of it, so
/// a zone the owner split into parts is never given a second whole-zone light.
///
/// Only whole-zone lights are made here. A split-zone light (`…p{n}`) is not recreated: if
/// `lights.json` is lost, a split zone comes back as one whole-zone light, and the slices'
/// labels, arrangements and `reversed` flags are gone (the files hold the only copy).
pub fn sync_lights(lights: &mut LightsFile, devices: &[DeviceInfo]) -> Vec<LightId> {
    let mut added = Vec::new();
    for device in devices {
        for zone in device.zones.iter().filter(|z| z.led_count > 0) {
            let has = lights
                .lights
                .iter()
                .any(|l| l.device_id == device.id && l.zone_index == zone.index);
            if !has {
                let light = whole_zone_light(device, zone);
                added.push(light.id.clone());
                lights.lights.push(light);
            }
        }
    }
    added
}

/// `text` cut to `max` characters (not bytes), `true` when it was cut.
fn cut_chars(text: &mut String, max: usize) -> bool {
    match text.char_indices().nth(max) {
        Some((at, _)) => {
            text.truncate(at);
            true
        }
        None => false,
    }
}

/// Makes a loaded file safe: after it, every scene passes
/// [`canvas_proto::scene::validate_scene`] against `lights`. It cuts scene and group names to
/// their maximum (by characters), clamps brightness to 100, gives a scene id used twice a numeric
/// suffix (the first keeps it), gives a group id used twice one too, drops a light's second
/// membership (the FIRST group that names it keeps it), drops members that name no light, then
/// empty groups. It points `active` at the first scene if it names none, and makes the default
/// scene if there are no scenes. Returns what it changed, for the log.
///
/// One exception: while `lights` is empty (the lights file was lost, or never written) members
/// that name no light are kept, because [`sync_lights`] brings the whole-zone ids back from the
/// devices. Those scenes validate again once the lights are back. (A lost split-zone light
/// does not come back; see [`sync_lights`].)
pub fn repair(scenes: &mut ScenesFile, lights: &LightsFile) -> Vec<String> {
    let mut changes = Vec::new();
    let known: HashSet<&str> = lights.lights.iter().map(|l| l.id.as_str()).collect();
    let lights_lost = lights.lights.is_empty();
    let mut scene_ids: Vec<String> = Vec::new();
    for scene in &mut scenes.scenes {
        if scene_ids.contains(&scene.id) {
            let new = fresh_id(&scene.id, scene_ids.iter().map(String::as_str));
            changes.push(format!("scene id {} was used twice; renamed {new}", scene.id));
            scene.id = new;
        }
        scene_ids.push(scene.id.clone());
        if cut_chars(&mut scene.name, SCENE_NAME_MAX) {
            changes.push(format!("scene {}: its name was cut to {SCENE_NAME_MAX} characters", scene.id));
        }
        let mut held: HashSet<LightId> = HashSet::new();
        for group in &mut scene.groups {
            if cut_chars(&mut group.name, GROUP_NAME_MAX) {
                changes.push(format!(
                    "scene {}: group {} name was cut to {GROUP_NAME_MAX} characters",
                    scene.id, group.id
                ));
            }
            if group.brightness > GROUP_BRIGHTNESS_MAX {
                group.brightness = GROUP_BRIGHTNESS_MAX;
                changes.push(format!("scene {}: group {} brightness was clamped to {GROUP_BRIGHTNESS_MAX}", scene.id, group.id));
            }
            let (mut unknown, mut elsewhere, mut twice) = (0, 0, 0);
            let mut here: HashSet<LightId> = HashSet::new();
            let mut kept = Vec::with_capacity(group.members.len());
            for member in std::mem::take(&mut group.members) {
                if !lights_lost && !known.contains(member.as_str()) {
                    unknown += 1;
                } else if held.contains(&member) {
                    elsewhere += 1;
                } else if !here.insert(member.clone()) {
                    twice += 1;
                } else {
                    kept.push(member);
                }
            }
            held.extend(here);
            group.members = kept;
            for (n, what) in [
                (unknown, "that name no light"),
                (elsewhere, "already in another group"),
                (twice, "listed twice in the group"),
            ] {
                if n > 0 {
                    changes.push(format!("scene {}: group {} lost {n} light(s) {what}", scene.id, group.id));
                }
            }
        }
        let before = scene.groups.len();
        scene.groups.retain(|g| !g.members.is_empty());
        if scene.groups.len() != before {
            changes.push(format!("scene {}: dropped {} empty group(s)", scene.id, before - scene.groups.len()));
        }
        let mut seen: Vec<String> = Vec::new();
        for group in &mut scene.groups {
            if seen.contains(&group.id) {
                let new = fresh_id(&group.id, seen.iter().map(String::as_str));
                changes.push(format!("scene {}: group id {} was used twice; renamed {new}", scene.id, group.id));
                group.id = new;
            }
            seen.push(group.id.clone());
        }
        // At most SCENE_HTML_MAX groups play an HTML effect: later ones, in scene order, go Off.
        let mut html = 0;
        for group in &mut scene.groups {
            let is_html = matches!(&group.plays, Plays::Effect { effect, .. } if effect.starts_with(HTML_EFFECT_PREFIX));
            if is_html {
                html += 1;
                if html > SCENE_HTML_MAX {
                    group.plays = Plays::Off;
                    changes.push(format!(
                        "scene {}: group {} was set to Off (more than {SCENE_HTML_MAX} HTML effect groups)",
                        scene.id, group.id
                    ));
                }
            }
        }
    }
    if scenes.scenes.is_empty() {
        *scenes = default_scenes(lights);
        changes.push("there were no scenes; made the default scene".into());
    } else if !scenes.scenes.iter().any(|s| s.id == scenes.active) {
        scenes.active = scenes.scenes[0].id.clone();
        changes.push(format!("the active scene named nothing; it is now {}", scenes.active));
    }
    changes
}

/// The active scene. `repair` guarantees there is one; on a file that skipped it, the first.
///
/// # Panics
/// When the file holds no scene at all, which `repair` and the loaders never leave.
pub fn active_scene(scenes: &ScenesFile) -> &Scene {
    scenes
        .scenes
        .iter()
        .find(|s| s.id == scenes.active)
        .or_else(|| scenes.scenes.first())
        .expect("a scenes file always holds a scene")
}

// ---------------------------------------------------------------- running a scene

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use canvas_proto::props::InputNeeds;
use canvas_proto::traits::{Effect, EngineGroup, GroupState, LightFrame, LightPreviewSink};

use crate::state::App;

/// Frames of the per-light feed waiting for a slow socket. The engine submits at most 15 a
/// second, so a socket that falls this far behind skips ahead.
const LIGHT_FEED_BACKLOG: usize = 4;

/// The editor windows that may watch the lights at once. One window is the normal case, a few
/// more cover a second monitor and a stale tab; past this the handshake is answered 503.
pub const LIGHT_FEED_MAX_WATCHERS: usize = 8;

/// How long a send to a watching window may take before the window is dropped. A window that stops
/// reading would otherwise hold its task and its watcher slot for as long as the socket stays open.
pub const LIGHT_FEED_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What the daemon keeps while a scene runs.
pub struct SceneRuntime {
    on: AtomicBool,
    order: tokio::sync::Mutex<()>,
    /// Why a group's effect could not be built, by group id, for the scene the engine runs.
    failures: Mutex<BTreeMap<String, String>>,
    /// What the engine was last handed, so a device list that changes nothing rebuilds nothing.
    applied: Mutex<Option<Applied>>,
    changed: tokio::sync::watch::Sender<u64>,
    /// How many effects the scene path has built, so a test can tell a rebuild from a keep.
    builds: std::sync::atomic::AtomicU64,
    /// The editor windows watching the lights, and the one engine sink that feeds them all.
    pub feed: LightFeed,
}

/// The inputs of one `set_scene`, kept to tell a repeat from a change.
#[derive(Clone, PartialEq)]
struct Applied {
    lights: Vec<Light>,
    devices: Vec<DeviceInfo>,
    scene: Scene,
}

impl Default for SceneRuntime {
    fn default() -> Self {
        Self {
            on: AtomicBool::new(false),
            order: tokio::sync::Mutex::new(()),
            failures: Mutex::new(BTreeMap::new()),
            applied: Mutex::new(None),
            changed: tokio::sync::watch::channel(0).0,
            builds: std::sync::atomic::AtomicU64::new(0),
            feed: LightFeed::default(),
        }
    }
}

/// Proof that the caller holds the scene order: only one route changes the scenes at a time, from
/// reading the revision to handing the engine the result (Review Focus 4).
pub struct SceneLock<'a>(#[allow(dead_code)] tokio::sync::MutexGuard<'a, ()>);

impl SceneRuntime {
    /// True once the engine has been handed a scene. The legacy layout path then stays out of the
    /// engine (`App::set_layout`), because a layout call would end scene mode.
    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }

    /// Takes the order. Held across a route's read, write and apply.
    pub async fn lock(&self) -> SceneLock<'_> {
        SceneLock(self.order.lock().await)
    }

    /// How many effects the scene path has built since the daemon started.
    pub fn builds(&self) -> u64 {
        self.builds.load(Ordering::SeqCst)
    }

    /// Why each group that could not be built is dark, by group id.
    pub fn failures(&self) -> BTreeMap<String, String> {
        self.failures.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Wakes the per-light feeds: the scene or the lights changed, so the index is sent again.
    pub fn notify(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// A receiver that wakes on [`SceneRuntime::notify`].
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }
}

/// The engine's one light-preview sink, fanned out to every socket that watches the lights.
pub struct LightFeed {
    tx: tokio::sync::broadcast::Sender<Arc<LightFrame>>,
    watchers: Mutex<usize>,
    /// [`LIGHT_FEED_SEND_TIMEOUT`], in milliseconds; a test shortens it.
    send_timeout_ms: std::sync::atomic::AtomicU64,
}

impl Default for LightFeed {
    fn default() -> Self {
        Self {
            tx: tokio::sync::broadcast::channel(LIGHT_FEED_BACKLOG).0,
            watchers: Mutex::new(0),
            send_timeout_ms: std::sync::atomic::AtomicU64::new(LIGHT_FEED_SEND_TIMEOUT.as_millis() as u64),
        }
    }
}

struct FeedSink(tokio::sync::broadcast::Sender<Arc<LightFrame>>);

impl LightPreviewSink for FeedSink {
    fn submit(&self, frame: LightFrame) {
        // No receiver is not an error: the last window just left.
        let _ = self.0.send(Arc::new(frame));
    }
}

impl LightFeed {
    /// One more window watches. The first one makes the engine start submitting light frames.
    /// `None` when [`LIGHT_FEED_MAX_WATCHERS`] windows already watch.
    pub fn join(&self, engine: &dyn canvas_proto::traits::Engine) -> Option<tokio::sync::broadcast::Receiver<Arc<LightFrame>>> {
        let mut watchers = self.watchers.lock().unwrap_or_else(|e| e.into_inner());
        if *watchers >= LIGHT_FEED_MAX_WATCHERS {
            return None;
        }
        *watchers += 1;
        if *watchers == 1 {
            engine.set_light_preview(Some(Arc::new(FeedSink(self.tx.clone()))));
        }
        Some(self.tx.subscribe())
    }

    /// A window left. The last one makes the engine stop.
    pub fn leave(&self, engine: &dyn canvas_proto::traits::Engine) {
        let mut watchers = self.watchers.lock().unwrap_or_else(|e| e.into_inner());
        *watchers = watchers.saturating_sub(1);
        if *watchers == 0 {
            engine.set_light_preview(None);
        }
    }

    /// How long a send to a window may take before the window is dropped.
    pub fn send_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.send_timeout_ms.load(Ordering::SeqCst))
    }

    /// Changes [`LightFeed::send_timeout`] (a test shortens it).
    pub fn set_send_timeout(&self, timeout: std::time::Duration) {
        self.send_timeout_ms.store(timeout.as_millis() as u64, Ordering::SeqCst);
    }

    /// How many windows watch.
    pub fn watchers(&self) -> usize {
        *self.watchers.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One window's place in the light feed. The place is given back when this is dropped, however the
/// window's task ends: a loop that breaks, an error, a cancel or a panic.
pub struct LightWatch {
    app: Arc<App>,
    frames: tokio::sync::broadcast::Receiver<Arc<LightFrame>>,
}

impl LightWatch {
    /// Takes a place, or `None` when [`LIGHT_FEED_MAX_WATCHERS`] windows already watch.
    pub fn join(app: &Arc<App>) -> Option<Self> {
        let frames = app.scene.feed.join(app.engine.as_ref())?;
        Some(Self { app: Arc::clone(app), frames })
    }

    /// The next frame, or why there is none (the window skipped ahead, or the feed closed).
    pub async fn recv(&mut self) -> Result<Arc<LightFrame>, tokio::sync::broadcast::error::RecvError> {
        self.frames.recv().await
    }
}

impl Drop for LightWatch {
    fn drop(&mut self) {
        self.app.scene.feed.leave(self.app.engine.as_ref());
    }
}

/// The effect for `id`, built the way the lights build it, and what it needs. Runs on a blocking
/// thread: an HTML effect runs its script while it loads.
fn build_effect(app: &App, id: &str) -> Result<(Box<dyn Effect>, InputNeeds), String> {
    app.scene.builds.fetch_add(1, Ordering::SeqCst);
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::create_effect(&app.user_effects, &app.stacks, id)
    }))
    .map_err(|payload| format!("the effect panicked while it loaded: {}", crate::panic_payload_text(&payload)))??;
    let schema = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        canvas_core::effect_guard::run_guarded(|| built.schema())
    }))
    .map_err(|payload| format!("the effect's schema panicked: {}", crate::panic_payload_text(&payload)))?;
    Ok((built, schema.needs))
}

/// What a scene's groups need from the screen and the audio capture.
fn needs_of(app: &App, scene: &Scene) -> InputNeeds {
    let mut needs = InputNeeds::default();
    for group in &scene.groups {
        if let Plays::Effect { effect, .. } = &group.plays {
            if let Some(schema) = crate::api::effects::find_schema(&app.user_effects, &app.stacks, effect) {
                needs.screen |= schema.needs.screen;
                needs.audio |= schema.needs.audio;
            }
        }
    }
    needs
}

/// Builds every group's effect and hands the scene to the engine. A group whose effect cannot be
/// built is dark (`effect: None`) and named in the failures that come back; the others run.
fn build_and_set(app: &App, lights: &[Light], devices: &[DeviceInfo], scene: &Scene, fade: bool) -> BTreeMap<String, String> {
    app.stacks.clear_running();
    let mut needs = InputNeeds::default();
    let mut failures = BTreeMap::new();
    let mut groups = Vec::with_capacity(scene.groups.len());
    for group in &scene.groups {
        let (effect, props) = match &group.plays {
            Plays::Off => (None, Props::new()),
            Plays::Effect { effect: id, props } => match build_effect(app, id) {
                Ok((built, group_needs)) => {
                    needs.screen |= group_needs.screen;
                    needs.audio |= group_needs.audio;
                    (Some(built), props.clone())
                }
                Err(why) => {
                    tracing::warn!(group = %group.id, effect = %id, error = %why, "a group's effect could not be built; the group is dark");
                    failures.insert(group.id.clone(), why);
                    (None, props.clone())
                }
            },
        };
        groups.push(EngineGroup {
            id: group.id.clone(),
            members: group.members.clone(),
            mode: group.mode,
            brightness: group.brightness,
            effect,
            props,
        });
    }
    app.inputs.apply_needs(needs);
    app.engine.set_scene(lights.to_vec(), devices.to_vec(), groups, fade);
    failures
}

/// Builds each group's effect through the daemon's builders and hands the active scene to the
/// engine. A group whose effect cannot be built gets `effect: None` and a failure text
/// ([`group_states`]). Start-up applies with `fade: false`; setting a scene applies with `true`.
pub async fn apply_active(app: &Arc<App>, fade: bool) {
    let lock = app.scene.lock().await;
    apply_with(app, &lock, fade).await;
}

/// What start-up does with the saved scenes: ends any draft, applies the active scene with no
/// fade, and logs what it restored. A group whose effect is missing is dark and named; the others
/// run. Lives here, not in `main`, so a test runs the same path.
pub async fn restore_active(app: &Arc<App>) {
    {
        // No draft survives a restart, but every apply follows the same rule: end one first,
        // and keep a beat off while the saved scene goes on.
        let _replace = crate::draft::hold_for_replace(app).await;
        apply_active(app, false).await;
    }
    let failures = app.scene.failures();
    let store = app.store.read().await;
    let scene = active_scene(&store.scenes);
    tracing::info!(scene = %scene.name, groups = scene.groups.len(), dark = failures.len(), "the active scene was restored");
    for group in &scene.groups {
        let Plays::Effect { effect, .. } = &group.plays else { continue };
        if failures.contains_key(&group.id) && effect == crate::migration::USER_POISON_ID {
            tracing::warn!(
                path = %crate::paths::effects_dir().join(crate::migration::USER_POISON_FILE).display(),
                "the user effect file is missing; install it to run this effect"
            );
        }
    }
}

/// [`apply_active`] for a device list: a list that changes nothing the engine was handed (the
/// same lights, the same devices, the same scene) rebuilds nothing, so the supervisor's refresh
/// never restarts an effect.
pub async fn apply_for_devices(app: &Arc<App>) {
    let lock = app.scene.lock().await;
    let key = snapshot(app).await;
    let unchanged = app.scene.is_on()
        && app.scene.applied.lock().unwrap_or_else(|e| e.into_inner()).as_ref() == Some(&key);
    if unchanged {
        return;
    }
    apply_with(app, &lock, false).await;
}

async fn snapshot(app: &App) -> Applied {
    let (lights, scene) = {
        let store = app.store.read().await;
        (store.lights.lights.clone(), active_scene(&store.scenes).clone())
    };
    Applied { lights, devices: app.devices.read().await.clone(), scene }
}

/// [`apply_active`] for a caller that holds the order.
pub async fn apply_with(app: &Arc<App>, _lock: &SceneLock<'_>, fade: bool) {
    let key = snapshot(app).await;
    let built = {
        let app = Arc::clone(app);
        let key = key.clone();
        tokio::task::spawn_blocking(move || build_and_set(&app, &key.lights, &key.devices, &key.scene, fade)).await
    };
    match built {
        Ok(failures) => {
            *app.scene.failures.lock().unwrap_or_else(|e| e.into_inner()) = failures;
            *app.scene.applied.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
            app.scene.on.store(true, Ordering::SeqCst);
            app.scene.notify();
        }
        Err(e) => tracing::error!(error = %e, "the scene could not be applied: the build task failed"),
    }
}

/// Brings the engine from `old` to `new`, the same scene saved with a change. It rebuilds only the
/// groups that changed: a new value for the same effect (`set_group_props`), a new brightness, a
/// group set to Off or to another effect (`set_group_effect` for that group alone). When a group
/// was added, removed or reordered, or its members or mode changed, the engine takes the whole
/// scene again (`set_scene` is the only way members reach it), so every effect is built again.
pub async fn apply_group_change(app: &Arc<App>, lock: &SceneLock<'_>, old: &Scene, new: &Scene) {
    let same_shape = old.groups.len() == new.groups.len()
        && old.groups.iter().zip(&new.groups).all(|(o, n)| o.id == n.id && o.members == n.members && o.mode == n.mode);
    if !same_shape {
        apply_with(app, lock, false).await;
        return;
    }
    let mut failures = app.scene.failures();
    for (before, after) in old.groups.iter().zip(&new.groups) {
        // A group that is dark (its effect could not be built) is tried again on every save, even
        // one that names the same effect: the file may be back.
        let dark = failures.contains_key(&after.id);
        if before.plays != after.plays || dark {
            match (&before.plays, &after.plays) {
                (Plays::Effect { effect: was, .. }, Plays::Effect { effect: now, props }) if was == now && !dark => {
                    app.engine.set_group_props(&after.id, props);
                }
                (_, Plays::Off) => {
                    failures.remove(&after.id);
                    app.engine.set_group_effect(&after.id, None, &Props::new());
                }
                (_, Plays::Effect { effect, props }) => {
                    let built = {
                        let app = Arc::clone(app);
                        let id = effect.clone();
                        tokio::task::spawn_blocking(move || build_effect(&app, &id)).await
                    };
                    // The store keeps the effect the save named. When it cannot be built the group
                    // is dark and the failure names why, so the store (what the group is meant to
                    // play) and the engine (what it plays) never disagree about which it is.
                    let built = built.unwrap_or_else(|e| Err(format!("the build task failed: {e}")));
                    match built {
                        Ok((made, _)) => {
                            failures.remove(&after.id);
                            app.engine.set_group_effect(&after.id, Some(made), props);
                        }
                        Err(why) => {
                            tracing::warn!(group = %after.id, effect = %effect, error = %why, "a group's effect could not be built; the group is dark");
                            failures.insert(after.id.clone(), why);
                            app.engine.set_group_effect(&after.id, None, props);
                        }
                    }
                }
            }
        }
        if before.brightness != after.brightness {
            app.engine.set_group_brightness(&after.id, after.brightness);
        }
    }
    app.inputs.apply_needs(needs_of(app, new));
    *app.scene.failures.lock().unwrap_or_else(|e| e.into_inner()) = failures;
    // What the engine runs is now `new`; a device list that repeats it needs no rebuild. Only the
    // scene changes in the key: the lights and devices stay the ones the engine was last handed. A
    // fresh snapshot here could hold a device list that arrived while this change held the lock and
    // that the engine never got, and the next `apply_for_devices` would then skip it as a repeat.
    if let Some(applied) = app.scene.applied.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        if applied.scene.id == new.id {
            applied.scene = new.clone();
        }
    }
    app.scene.notify();
}

/// How the groups of `scene` are doing: dark because the effect could not be built, or because it
/// panicked while it ran. One entry per group, in the scene's order.
pub fn group_states(app: &App, scene: &Scene) -> Vec<GroupState> {
    let built = app.scene.failures();
    let running = app.engine.group_states();
    scene
        .groups
        .iter()
        .map(|g| GroupState {
            id: g.id.clone(),
            failed: built
                .get(&g.id)
                .cloned()
                .or_else(|| running.iter().find(|s| s.id == g.id).and_then(|s| s.failed.clone())),
        })
        .collect()
}

/// How many groups of `scene` play an HTML effect, counting a stack as HTML when any of its layers
/// or masks is (`has_html_layer` answers for a stack id). Each is a script of its own, the slowest
/// group there is (`SCENE_HTML_MAX`).
pub fn html_group_count(scene: &Scene, has_html_layer: &dyn Fn(&str) -> bool) -> usize {
    scene
        .groups
        .iter()
        .filter(|g| match &g.plays {
            Plays::Effect { effect, .. } => effect.starts_with(HTML_EFFECT_PREFIX) || has_html_layer(effect),
            Plays::Off => false,
        })
        .count()
}

/// True when the saved stack `id` has an HTML layer or mask.
pub fn stack_has_html(stacks: &crate::stacks::Stacks, id: &str) -> bool {
    stacks.get(id).is_some_and(|stack| {
        stack.layers.iter().any(|layer| {
            layer.effect.starts_with(HTML_EFFECT_PREFIX)
                || layer.mask.as_ref().is_some_and(|m| m.effect.starts_with(HTML_EFFECT_PREFIX))
        })
    })
}

/// `Err` with the sentence to show when `scene` plays more HTML effects than a scene may.
pub fn check_html_limit(app: &App, scene: &Scene) -> Result<(), String> {
    let count = html_group_count(scene, &|id| stack_has_html(&app.stacks, id));
    if count > SCENE_HTML_MAX {
        return Err(format!(
            "at most {SCENE_HTML_MAX} group{} of a scene may play an HTML effect (a stack with an HTML layer counts); {count} do",
            if SCENE_HTML_MAX == 1 { "" } else { "s" }
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use canvas_proto::device::{DeviceId, DeviceKind, ZoneKind};
    use canvas_proto::layout::LedSlice;

    pub(crate) fn zone(index: u32, kind: ZoneKind, leds: u32) -> ZoneInfo {
        ZoneInfo {
            index,
            name: format!("Zone {index}"),
            kind,
            led_count: leds,
            min_leds: 0,
            max_leds: 64,
            resizable: false,
        }
    }

    pub(crate) fn device(id: &str, adapter_index: u32, zones: Vec<ZoneInfo>) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(id.into()),
            name: id.into(),
            vendor: "v".into(),
            kind: DeviceKind::Other,
            adapter: "openrgb".into(),
            adapter_index,
            serial: String::new(),
            location: String::new(),
            zones,
            connected: true,
        }
    }

    #[test]
    fn a_new_device_adds_lights_that_join_no_group() {
        let mut lights = empty_lights();
        let board = device("openrgb:board", 0, vec![zone(0, ZoneKind::Linear, 30), zone(1, ZoneKind::Single, 1)]);
        let mut scenes = default_scenes(&lights);
        assert!(scenes.scenes[0].groups.is_empty());
        let added = sync_lights(&mut lights, std::slice::from_ref(&board));
        assert_eq!(added, vec!["openrgb:board#z0".to_string(), "openrgb:board#z1".to_string()]);
        assert_eq!(lights.lights[0].arrangement, Arrangement::Strip);
        assert_eq!(lights.lights[1].arrangement, Arrangement::Single);
        // A second sync finds nothing new, and the new lights are in no group.
        assert!(sync_lights(&mut lights, std::slice::from_ref(&board)).is_empty());
        assert!(repair(&mut scenes, &lights).is_empty());
        assert_eq!(canvas_proto::scene::unassigned(active_scene(&scenes), &lights.lights).len(), 2);
    }

    #[test]
    fn a_zone_with_zero_leds_is_not_a_light() {
        let mut lights = empty_lights();
        let board = device("openrgb:board", 0, vec![zone(0, ZoneKind::Linear, 0), zone(1, ZoneKind::Linear, 5)]);
        assert_eq!(sync_lights(&mut lights, &[board]), vec!["openrgb:board#z1".to_string()]);
    }

    #[test]
    fn a_split_zone_gets_no_second_whole_zone_light() {
        let mut lights = empty_lights();
        let d = device("d", 0, vec![zone(0, ZoneKind::Linear, 30)]);
        let mut part = whole_zone_light(&d, &d.zones[0]);
        part.id = "d#z0p1".into();
        part.leds = Some(LedSlice { start: 0, count: 10 });
        lights.lights.push(part);
        assert!(sync_lights(&mut lights, &[d]).is_empty());
    }

    /// Review Focus 1: the device drops and returns with a new adapter index. Its lights keep
    /// their ids and their groups, and no light is added or lost.
    #[test]
    fn reconnect_keeps_group_membership() {
        let mut lights = empty_lights();
        let before = device("openrgb:board", 0, vec![zone(0, ZoneKind::Linear, 30)]);
        sync_lights(&mut lights, std::slice::from_ref(&before));
        let mut scenes = default_scenes(&lights);
        scenes.scenes[0].groups = everything_group(&lights.lights).into_iter().collect();
        let held = scenes.clone();

        // Gone: the device is not connected and lists no zone. Nothing is removed.
        let mut gone = before.clone();
        gone.connected = false;
        gone.zones.clear();
        assert!(sync_lights(&mut lights, &[gone]).is_empty());
        assert_eq!(lights.lights.len(), 1);

        // Back, on another adapter index: the same light, the same group.
        let mut back = before.clone();
        back.adapter_index = 7;
        assert!(sync_lights(&mut lights, &[back]).is_empty(), "no second light for the same zone");
        assert_eq!(lights.lights.len(), 1);
        assert_eq!(lights.lights[0].id, "openrgb:board#z0");
        assert!(repair(&mut scenes, &lights).is_empty(), "nothing to repair");
        assert_eq!(scenes, held, "the group still holds the light");
    }

    fn group(id: &str, members: &[&str]) -> Group {
        Group {
            id: id.into(),
            name: id.into(),
            members: members.iter().map(|m| m.to_string()).collect(),
            plays: Plays::Off,
            mode: GroupMode::FlowAcross,
            brightness: 100,
        }
    }

    /// Review Focus 5: a light in two groups, an empty group and an `active` naming nothing are
    /// repaired; a file with no scene gets the default one.
    #[test]
    fn damaged_scenes_file_still_lights_everything() {
        let mut lights = empty_lights();
        let board = device("b", 0, vec![zone(0, ZoneKind::Linear, 30), zone(1, ZoneKind::Linear, 20)]);
        sync_lights(&mut lights, &[board]);
        let mut scenes = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "nothing".into(),
            scenes: vec![Scene {
                id: "s".into(),
                name: "S".into(),
                rev: 3,
                groups: vec![group("a", &["b#z0"]), group("b", &["b#z0", "b#z1"]), group("empty", &[])],
            }],
        };
        let changes = repair(&mut scenes, &lights);
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert_eq!(scenes.active, "s");
        let s = active_scene(&scenes);
        canvas_proto::scene::validate_scene(s, &lights.lights).expect("the repaired scene is valid");
        // The FIRST group keeps the light both named; a repair that kept it in the wrong group
        // would leave `a` empty (and dropped) or `b` holding both.
        assert_eq!(s.groups.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(s.groups[0].members, vec!["b#z0".to_string()]);
        assert_eq!(s.groups[1].members, vec!["b#z1".to_string()]);
        assert_eq!(s.rev, 3, "repair keeps the revision");

        // No scene at all: the default scene, with every light in one group, playing something.
        let mut none = ScenesFile { schema_version: SCENES_SCHEMA, active: "x".into(), scenes: vec![] };
        repair(&mut none, &lights);
        let d = active_scene(&none);
        assert_eq!(d.name, DEFAULT_SCENE_NAME);
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].members.len(), 2);
        assert!(matches!(d.groups[0].plays, Plays::Effect { .. }));
    }

    /// A scene whose only fault is the one the test sets.
    fn scene_with(groups: Vec<Group>) -> ScenesFile {
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![Scene { id: "s".into(), name: "S".into(), rev: 1, groups }],
        }
    }

    fn two_lights() -> LightsFile {
        let mut lights = empty_lights();
        sync_lights(&mut lights, &[device("b", 0, vec![zone(0, ZoneKind::Linear, 30), zone(1, ZoneKind::Linear, 20)])]);
        lights
    }

    /// The FIRST group that names a light keeps it; the later group loses it.
    #[test]
    fn the_first_group_keeps_a_light_two_groups_hold() {
        let lights = two_lights();
        let mut scenes = scene_with(vec![group("a", &["b#z0"]), group("b", &["b#z0", "b#z1"])]);
        repair(&mut scenes, &lights);
        let groups = &scenes.scenes[0].groups;
        assert_eq!(groups[0].members, vec!["b#z0".to_string()]);
        assert_eq!(groups[1].members, vec!["b#z1".to_string()]);
    }

    /// A light listed twice inside one group is not "in another group".
    #[test]
    fn a_light_listed_twice_in_one_group_says_so() {
        let lights = two_lights();
        let mut scenes = scene_with(vec![group("a", &["b#z0", "b#z0", "b#z1"])]);
        let changes = repair(&mut scenes, &lights);
        assert_eq!(scenes.scenes[0].groups[0].members, vec!["b#z0".to_string(), "b#z1".to_string()]);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(changes[0].contains("twice"), "{changes:?}");
        assert!(!changes[0].contains("another group"), "{changes:?}");
    }

    /// With the lights known, a member that names no light is dropped, and its group with it when
    /// nothing is left (S2: the scene must pass `validate_scene`).
    #[test]
    fn members_naming_no_light_are_dropped_then_empty_groups() {
        let lights = two_lights();
        let mut scenes = scene_with(vec![group("a", &["gone", "b#z0"]), group("ghost", &["gone-too"])]);
        let changes = repair(&mut scenes, &lights);
        let groups = &scenes.scenes[0].groups;
        assert_eq!(groups.len(), 1, "{changes:?}");
        assert_eq!(groups[0].members, vec!["b#z0".to_string()]);
        canvas_proto::scene::validate_scene(&scenes.scenes[0], &lights.lights).expect("valid");
    }

    /// The one exception: with the lights file empty (lost), the members stay, because
    /// `sync_lights` brings the whole-zone ids back from the devices.
    #[test]
    fn members_stay_while_the_lights_file_is_empty() {
        let mut scenes = scene_with(vec![group("a", &["b#z0", "b#z1"])]);
        let changes = repair(&mut scenes, &empty_lights());
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(scenes.scenes[0].groups[0].members.len(), 2);
    }

    #[test]
    fn long_names_are_cut_and_brightness_is_clamped() {
        use canvas_proto::scene::{GROUP_NAME_MAX, SCENE_NAME_MAX};
        let lights = two_lights();
        let mut g = group("a", &["b#z0"]);
        g.name = "é".repeat(GROUP_NAME_MAX + 9);
        g.brightness = 255;
        let mut scenes = scene_with(vec![g]);
        scenes.scenes[0].name = "ü".repeat(SCENE_NAME_MAX + 5);
        repair(&mut scenes, &lights);
        assert_eq!(scenes.scenes[0].name.chars().count(), SCENE_NAME_MAX);
        assert_eq!(scenes.scenes[0].groups[0].name.chars().count(), GROUP_NAME_MAX);
        assert_eq!(scenes.scenes[0].groups[0].brightness, 100);
        canvas_proto::scene::validate_scene(&scenes.scenes[0], &lights.lights).expect("valid");
    }

    #[test]
    fn a_scene_id_used_twice_gets_a_suffix() {
        let lights = two_lights();
        let mut scenes = scene_with(vec![group("a", &["b#z0"])]);
        let twin = Scene { name: "Twin".into(), groups: vec![group("a", &["b#z1"])], ..scenes.scenes[0].clone() };
        scenes.scenes.push(twin);
        let changes = repair(&mut scenes, &lights);
        let ids: Vec<_> = scenes.scenes.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["s", "s-2"], "{changes:?}");
        assert_eq!(scenes.active, "s", "the first keeps the id the file's active names");
    }

    /// Property-style: damaged scenes built from a seeded generator all validate after repair,
    /// whatever mix of faults they hold.
    #[test]
    fn html_groups_over_the_limit_are_set_off_in_scene_order() {
        let ls = LightsFile {
            schema_version: LIGHTS_SCHEMA,
            lights: ["a", "b", "c"].iter().map(|id| Light {
                id: (*id).into(),
                device_id: canvas_proto::DeviceId("fake:1".into()),
                zone_index: 0,
                leds: None,
                name: None,
                arrangement: Arrangement::Strip,
                reversed: false,
            }).collect(),
        };
        let g = |id: &str, member: &str, effect: &str| Group {
            id: id.into(),
            name: id.into(),
            members: vec![member.into()],
            plays: Plays::Effect { effect: effect.into(), props: Default::default() },
            mode: GroupMode::FlowAcross,
            brightness: 100,
        };
        let mut file = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![Scene {
                id: "s".into(),
                name: "s".into(),
                rev: 1,
                groups: vec![g("g1", "a", "html:ink-drop"), g("g2", "b", "aurora"), g("g3", "c", "html:wet-smear")],
            }],
        };
        let changes = repair(&mut file, &ls);
        let plays: Vec<bool> = file.scenes[0].groups.iter().map(|g| g.plays == Plays::Off).collect();
        // With a limit of one, the first HTML group keeps playing and the second goes Off.
        let mut expect = vec![false, false, false];
        if SCENE_HTML_MAX < 2 {
            expect[2] = true;
        }
        assert_eq!(plays, expect);
        assert_eq!(changes.iter().filter(|c| c.contains("set to Off")).count(), expect.iter().filter(|x| **x).count());
        assert_eq!(canvas_proto::scene::validate_scene(&file.scenes[0], &ls.lights), Ok(()));
    }

    #[test]
    fn whatever_the_damage_the_repaired_scenes_validate() {
        let lights = two_lights();
        let pool = ["b#z0", "b#z1", "gone", "also-gone"];
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for round in 0..200 {
            let scene_count = 1 + next(3);
            let mut scenes = ScenesFile { schema_version: SCENES_SCHEMA, active: "nope".into(), scenes: Vec::new() };
            for _ in 0..scene_count {
                let mut groups = Vec::new();
                for _ in 0..next(4) {
                    let members: Vec<&str> = (0..next(4)).map(|_| pool[next(pool.len())]).collect();
                    let mut g = group(["a", "b"][next(2)], &members);
                    g.name = "n".repeat(next(90));
                    g.brightness = [0u8, 50, 100, 101, 255][next(5)];
                    groups.push(g);
                }
                scenes.scenes.push(Scene {
                    id: ["s", "t"][next(2)].into(),
                    name: "x".repeat(next(90)),
                    rev: 1,
                    groups,
                });
            }
            repair(&mut scenes, &lights);
            assert!(!scenes.scenes.is_empty());
            let mut ids = HashSet::new();
            for scene in &scenes.scenes {
                assert!(ids.insert(scene.id.clone()), "round {round}: scene id used twice");
                canvas_proto::scene::validate_scene(scene, &lights.lights)
                    .unwrap_or_else(|e| panic!("round {round}: {e} in {scene:?}"));
            }
            assert!(scenes.scenes.iter().any(|s| s.id == scenes.active), "round {round}");
        }
    }

    #[test]
    fn two_groups_with_one_id_are_told_apart() {
        let lights = empty_lights();
        let mut scenes = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![Scene { id: "s".into(), name: "S".into(), rev: 1, groups: vec![group("a", &["x"]), group("a", &["y"])] }],
        };
        assert_eq!(repair(&mut scenes, &lights).len(), 1);
        let ids: Vec<_> = scenes.scenes[0].groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "a-2"]);
    }

    /// A group playing a stack whose layers include HTML counts as HTML (the scene's HTML limit).
    #[test]
    fn a_stack_with_an_html_layer_counts_as_an_html_group() {
        let play = |id: &str, effect: &str| Group {
            plays: Plays::Effect { effect: effect.into(), props: Props::new() },
            ..group(id, &["x"])
        };
        let scene = Scene {
            id: "s".into(),
            name: "S".into(),
            rev: 1,
            groups: vec![
                play("a", "html:ink"),
                play("b", "stack:glow"),
                play("c", "stack:plain"),
                play("d", "aurora"),
                group("e", &["y"]),
            ],
        };
        let has_html = |id: &str| id == "stack:glow";
        assert_eq!(html_group_count(&scene, &has_html), 2, "the html: group and the stack with an HTML layer");
        assert_eq!(html_group_count(&scene, &|_| false), 1);
    }

    #[test]
    fn ids_are_slugs_with_a_numeric_suffix_when_taken() {
        assert_eq!(slug("Living room!"), "living-room");
        assert_eq!(slug("  "), "scene");
        assert_eq!(fresh_id("a", ["b"]), "a");
        assert_eq!(fresh_id("a", ["a", "a-2"]), "a-3");
    }
}
