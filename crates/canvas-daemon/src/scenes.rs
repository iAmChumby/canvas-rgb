//! Scene store operations: the lights the devices bring, the repair of a damaged
//! `scenes.json`, and the default scene.
//!
//! Ids are plain and deterministic, like the layout ids before them (`default`, then the owner's
//! own names): a slug of a name, with `-2`, `-3` … when it is taken. Nothing here draws a random
//! number, so a test can name the id it expects.

use std::collections::HashSet;

use canvas_proto::device::{DeviceInfo, ZoneInfo};
use canvas_proto::pattern::LightFit;
use canvas_proto::props::Props;
use canvas_proto::scene::{
    zone_light_id, Arrangement, ChainState, Group, GroupMode, Light, LightId, LightsFile, Plays,
    Scene, ScenesFile, DEFAULT_GROUP_NAME, DEFAULT_SCENE_NAME, GROUP_BRIGHTNESS_MAX, GROUP_NAME_MAX,
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
        sequence: None,
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
    LightsFile { chains: Vec::new(), schema_version: LIGHTS_SCHEMA, lights: Vec::new() }
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
        kind: None,
        copies: None,
        first_led: None,
    }
}

/// Adds a light for every zone with `led_count > 0` that has none. Never removes a light.
/// Returns the ids added. New lights join no group.
///
/// A zone has a light when any light names its device and zone index, whole or a slice of it, so
/// a zone the owner split into parts is never given a second whole-zone light.
///
/// A zone the owner answered for in chain setup with nothing plugged in (`ChainState::Empty`,
/// 0.15.0) gets no light at all: adding one here would put back the whole-zone light the answer
/// was about. A zone with no record behaves exactly as it did before chains existed.
///
/// Only whole-zone lights are made here. A split-zone light (`…p{n}`) is not recreated: if
/// `lights.json` is lost, a split zone comes back as one whole-zone light, and the slices'
/// labels, arrangements and `reversed` flags are gone (the files hold the only copy).
pub fn sync_lights(lights: &mut LightsFile, devices: &[DeviceInfo]) -> Vec<LightId> {
    let mut added = Vec::new();
    // A device the daemon never drives (`crate::is_driven`) gets no light (#393).
    for device in devices.iter().filter(|d| crate::is_driven(&d.id)) {
        for zone in device.zones.iter().filter(|z| z.led_count > 0) {
            let empty = lights.chains.iter().any(|c| {
                c.device_id == device.id
                    && c.zone_index == zone.index
                    && c.state == ChainState::Empty
            });
            if empty {
                continue;
            }
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
        // The HTML limit is not repaired here (#370): a scene keeps what the owner saved. The
        // groups past the limit are not built, and each says why on its own row
        // ([`html_limit_sentence`], [`build_and_set`]).
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

/// What the engine holds: the groups it has, and the groups a takeover put its effect on.
///
/// The engine drops an effect for a group it does not have (`canvas-core/src/engine.rs`:
/// "set_group_effect: no such group in the running scene"), so a takeover has to know what the
/// engine holds before it hands one an effect.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Held {
    /// The group ids the engine holds.
    pub groups: Vec<String>,
    /// The groups a takeover put its effect on.
    pub takeover: Vec<String>,
}

/// What the daemon keeps while a scene runs.
pub struct SceneRuntime {
    on: AtomicBool,
    /// What the engine holds: the groups it has, and the ones a takeover put its effect on.
    held: Mutex<Held>,
    order: tokio::sync::Mutex<()>,
    /// Why a group's effect could not be built, by group id, for the scene the engine runs.
    failures: Mutex<BTreeMap<String, String>>,
    /// Which groups of a scene run its HTML runtimes, when the scene starts more than a scene may,
    /// by scene id and then group id. The first build of such a scene seeds it from the scene's own
    /// order, which is what a file written before #384 did; after that the choice is by group id,
    /// so the order the groups are listed in (display order) never changes which ones run.
    html_kept: Mutex<BTreeMap<String, Vec<String>>>,
    /// What the engine was last handed, so a device list that changes nothing rebuilds nothing.
    applied: Mutex<Option<Applied>>,
    changed: tokio::sync::watch::Sender<u64>,
    /// How many effects the scene path has built, so a test can tell a rebuild from a keep.
    builds: std::sync::atomic::AtomicU64,
    /// How many takeovers have landed: the generation of the takeover on the lights.
    ///
    /// A guard that is dropped with its debt unpaid compares this with what it saw: a takeover
    /// that landed after it holds the lights now, and its own end puts the scene back.
    takeovers: std::sync::atomic::AtomicU64,
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
            held: Mutex::new(Held::default()),
            order: tokio::sync::Mutex::new(()),
            failures: Mutex::new(BTreeMap::new()),
            html_kept: Mutex::new(BTreeMap::new()),
            applied: Mutex::new(None),
            changed: tokio::sync::watch::channel(0).0,
            builds: std::sync::atomic::AtomicU64::new(0),
            takeovers: std::sync::atomic::AtomicU64::new(0),
            feed: LightFeed::default(),
        }
    }
}

/// Proof that the caller holds the scene order: only one route changes the scenes at a time, from
/// reading the revision to handing the engine the result (Review Focus 4).
pub struct SceneLock<'a>(#[allow(dead_code)] tokio::sync::MutexGuard<'a, ()>);

impl SceneRuntime {
    /// True once the engine has been handed a scene: the only render path (#310). The device list
    /// and every route follow this scene, never a layout.
    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }

    /// True while a takeover holds the lights: what the engine runs is not the scene in the
    /// store. A scene change then takes the whole scene again, because the engine's groups are
    /// not the store's.
    pub fn is_taken_over(&self) -> bool {
        !self.held().takeover.is_empty()
    }

    /// What the engine holds: the groups it has, and the ones a takeover put its effect on.
    pub fn held(&self) -> Held {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Records what the engine holds.
    pub fn set_held(&self, held: Held) {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = held;
    }

    /// Records that a takeover landed: what the engine holds, and the groups it took over.
    ///
    /// The generation goes up, so a guard that is dropped with its debt unpaid can tell that
    /// something else holds the lights now: that takeover's own end puts the scene back, and
    /// the guard must not put the scene over it.
    pub fn land_takeover(&self, held: Held) {
        self.takeovers.fetch_add(1, Ordering::SeqCst);
        self.set_held(held);
    }

    /// The generation of the takeover on the lights: how many takeovers have landed.
    pub fn takeover_generation(&self) -> u64 {
        self.takeovers.load(Ordering::SeqCst)
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

    /// The groups of `scene` that run its HTML runtimes, by group id: empty until the first build
    /// of a scene over the limit seeds it (see [`build_and_set`]).
    pub fn html_kept(&self, scene: &str) -> Vec<String> {
        self.html_kept
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(scene)
            .cloned()
            .unwrap_or_default()
    }

    /// Records which groups of `scene` run its HTML runtimes. An empty list forgets the scene's
    /// choice, so the next build over the limit seeds it again from the scene's own order.
    pub fn set_html_kept(&self, scene: &str, groups: Vec<String>) {
        let mut kept = self.html_kept.lock().unwrap_or_else(|e| e.into_inner());
        if groups.is_empty() {
            kept.remove(scene);
        } else {
            kept.insert(scene.to_string(), groups);
        }
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
///
/// `group` is the group the effect plays in: a stack resolves its layers in that group's
/// canvas.
fn build_effect(app: &App, id: &str, group: Option<&str>) -> Result<(Box<dyn Effect>, InputNeeds), String> {
    app.scene.builds.fetch_add(1, Ordering::SeqCst);
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::create_effect(&app.user_effects, &app.stacks, &app.patterns, id, group)
    }))
    .map_err(|payload| format!("the effect panicked while it loaded: {}", crate::panic_payload_text(&payload)))??;
    let schema = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        canvas_core::effect_guard::run_guarded(|| built.schema())
    }))
    .map_err(|payload| format!("the effect's schema panicked: {}", crate::panic_payload_text(&payload)))?;
    Ok((built, schema.needs))
}

/// What every group of the scene needs from the screen and the audio capture.
///
/// `apply_needs` sets what the captures do, it does not merge: the needs of every group the
/// engine holds have to be in it, or the capture another group needs stops.
pub fn needs_of(app: &App, scene: &Scene) -> InputNeeds {
    let mut needs = InputNeeds::default();
    for group in &scene.groups {
        if let Plays::Effect { effect, .. } = &group.plays {
            if let Some(schema) = crate::api::effects::find_schema(&app.user_effects, &app.stacks, &app.patterns, effect) {
                needs.screen |= schema.needs.screen;
                needs.audio |= schema.needs.audio;
            }
        }
    }
    needs
}

/// The effect that is running in a scene, and the props it runs with.
///
/// The first group that plays an effect is the one that is running, and the props of that group
/// are the ones it runs with. `None` when no group of the scene plays an effect.
pub fn running_effect(scene: &Scene) -> Option<(String, Props)> {
    scene
        .groups
        .iter()
        .find_map(|group| match &group.plays {
            Plays::Effect { effect, props } => Some((effect.clone(), props.clone())),
            _ => None,
        })
}

/// Which groups of `scene` take one of its [`SCENE_HTML_MAX`] HTML runtimes, true at the group's
/// place in `scene.groups`. The groups kept from the last build of this scene come first, then the
/// rest in the scene's own order: the order a scene that has never been built over the limit is
/// seeded in (#384). A group takes its runtime before it is built, so a group whose build fails
/// keeps its place, built or not.
///
/// **The one rule**, read by [`build_and_set`] to decide what runs and by
/// [`groups_over_the_html_limit`] to tell a group the limit left dark from one whose build failed
/// (#419). The two never disagree about which group is dark because there is one function here.
fn html_runs(scene: &Scene, kept: &[String], slots_of: &dyn Fn(&str) -> usize) -> Vec<bool> {
    let rank = |id: &str| kept.iter().position(|k| k == id).unwrap_or(usize::MAX);
    let mut by_rank: Vec<usize> = (0..scene.groups.len()).collect();
    by_rank.sort_by_key(|&at| (rank(&scene.groups[at].id), at));
    let mut runtimes = 0usize;
    let mut runs = vec![true; scene.groups.len()];
    for at in by_rank {
        let slots = group_html_runtimes(&scene.groups[at], slots_of);
        if runtimes + slots > SCENE_HTML_MAX {
            runs[at] = false;
        } else {
            runtimes += slots;
        }
    }
    runs
}

/// Builds every group's effect and hands the scene to the engine. A group whose effect cannot be
/// built is dark (`effect: None`) and named in the failures that come back; the others run.
///
/// The HTML limit is enforced here (#370): [`SCENE_HTML_MAX`] HTML runtimes run, a group past them
/// is not built, and the row of each such group carries the sentence
/// ([`html_limit_sentence`]). The scene keeps it: nothing is written Off.
///
/// Which groups take the runtimes is the one thing here that a reorder could change, so it is
/// keyed on the groups' ids, never on their place (#384). The first build of a scene over the
/// limit seeds the choice from the scene's own order — what a file written before #384 did, so it
/// loads as it did — and every later build puts those groups first, so a reorder, a device list, a
/// pattern change or a takeover's end never changes which groups run.
fn build_and_set(app: &App, lights: &[Light], devices: &[DeviceInfo], scene: &Scene, fade: bool) -> BTreeMap<String, String> {
    app.stacks.clear_running();
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    let slots_of = |id: &str| saved_stack_slots(app, id, &known);
    let total = html_runtime_count(scene, &slots_of);
    let mut needs = InputNeeds::default();
    let mut failures = BTreeMap::new();

    // Which groups take an HTML runtime, decided before anything is built: the ones kept from the
    // last build of this scene first, then the rest in the scene's own order ([`html_runs`]).
    let runs = html_runs(scene, &app.scene.html_kept(&scene.id), &slots_of);
    let runs_html: Vec<String> = scene
        .groups
        .iter()
        .zip(&runs)
        .filter(|(group, runs)| **runs && group_html_runtimes(group, &slots_of) > 0)
        .map(|(group, _)| group.id.clone())
        .collect();

    // Which groups keep their pattern's frames fitted, decided before anything is built (#481).
    let pattern_over = pattern_budget_refusals(app, scene, lights, devices);

    let mut groups = Vec::with_capacity(scene.groups.len());
    for (at, group) in scene.groups.iter().enumerate() {
        let (effect, props) = match &group.plays {
            Plays::Off => (None, Props::new()),
            Plays::Effect { effect: id, props } => {
                if let Some(why) = pattern_over.get(&group.id) {
                    tracing::warn!(group = %group.id, effect = %id, "a group over the scene's pattern budget did not build; its row says why");
                    failures.insert(group.id.clone(), why.clone());
                    (None, props.clone())
                } else if !runs[at] {
                    // Over the limit: the group keeps what the owner saved, and does not run.
                    tracing::warn!(
                        group = %group.id,
                        effect = %id,
                        "a group over the scene's HTML limit did not build; its row says why"
                    );
                    failures.insert(group.id.clone(), html_limit_sentence(total));
                    (None, props.clone())
                } else {
                    match build_effect(app, id, Some(&group.id)) {
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
                    }
                }
            }
        };
        groups.push(EngineGroup {
            id: group.id.clone(),
            members: group.members.clone(),
            mode: group.mode,
            sequence: group.sequence,
            brightness: group.brightness,
            effect,
            props,
        });
    }
    // The choice is kept by group id from here on; a scene back within the limit forgets it, so a
    // scene that goes over again seeds from the order it is saved in then.
    if total > SCENE_HTML_MAX {
        app.scene.set_html_kept(&scene.id, runs_html);
    } else {
        app.scene.set_html_kept(&scene.id, Vec::new());
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

/// [`apply_active`] for a caller that already orders the lights with the draft gate.
///
/// The scene order is *not* taken here, and that is deliberate: a scene write holds the scene
/// order and then takes the draft gate (see [`crate::api::scenes::put_scene`]), while a restore
/// holds the draft gate. A restore that took the scene order here would wait for a write that
/// waits for the gate it holds, and the two would deadlock. The draft gate is enough: every
/// route that replaces what is on the lights holds it (see [`crate::draft::hold_for_replace`]),
/// so the restores and the replacements never interleave.
///
/// The callers that replace the lights without the draft gate are the ones that end the draft
/// first (the effect delete) or that only re-plan the lights (the light patch, which waits for a
/// takeover to end). A route that replaces what plays and does not end the draft would put the
/// scene over it, so it takes the gate.
pub async fn apply_active_ungated(app: &Arc<App>, fade: bool) {
    apply_body(app, fade).await;
}

/// What start-up does with the saved scenes: ends any draft, applies the active scene with no
/// fade, and logs what it restored. A group whose effect is missing is dark and named; the others
/// run. Lives here, not in `main`, so a test runs the same path.
pub async fn restore_active(app: &Arc<App>) {
    {
        // No draft survives a restart, but every apply follows the same rule: end one first,
        // and keep a beat off while the saved scene goes on.
        let replace = crate::draft::hold_for_replace(app).await;
        apply_active_ungated(app, false).await;
        // The scene is on the lights: the takeover is over, and the guard owes nothing.
        replace.settle();
    }
    let failures = app.scene.failures();
    let store = app.store.read().await;
    let scene = active_scene(&store.scenes);
    tracing::info!(target: crate::logging::LOG_TARGET, scene = %scene.name, groups = scene.groups.len(), dark = failures.len(), "the active scene was restored");
    for group in &scene.groups {
        let Plays::Effect { effect, .. } = &group.plays else { continue };
        if failures.contains_key(&group.id) && effect == crate::migration::USER_POISON_ID {
            tracing::warn!(
                target: crate::logging::LOG_TARGET,
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
    // The draft gate: a takeover holds the lights, and it must not be put over by a device list
    // that arrived in between.
    let _gate = app.draft.gate().lock().await;
    // The check is under the gate: a takeover that landed while this waited is seen here.
    if app.scene.is_taken_over() {
        return;
    }
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
    apply_body(app, fade).await;
}

/// The body of every apply: the active scene is built and handed to the engine.
async fn apply_body(app: &Arc<App>, fade: bool) {
    let key = snapshot(app).await;
    let built = {
        let app = Arc::clone(app);
        let key = key.clone();
        tokio::task::spawn_blocking(move || build_and_set(&app, &key.lights, &key.devices, &key.scene, fade)).await
    };
    match built {
        Ok(failures) => {
            *app.scene.failures.lock().unwrap_or_else(|e| e.into_inner()) = failures;
            // The scene is on the lights again: a takeover is over, and the engine holds the
            // scene's own groups.
            let held = Held {
                groups: key.scene.groups.iter().map(|g| g.id.clone()).collect(),
                takeover: Vec::new(),
            };
            *app.scene.applied.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
            app.scene.on.store(true, Ordering::SeqCst);
            app.scene.set_held(held);
            app.scene.notify();
            // The masks of the groups follow the scene: a stack resolves its layers in the group
            // it plays in.
            publish_group_areas(app).await;
        }
        Err(e) => {
            // The build task failed: nothing was built, so the takeover is over and the
            // groups are dark. The lights are never left on a takeover nothing owns.
            let why = format!("the scene could not be built: {e}");
            let failures: BTreeMap<String, String> =
                key.scene.groups.iter().map(|group| (group.id.clone(), why.clone())).collect();
            *app.scene.failures.lock().unwrap_or_else(|e| e.into_inner()) = failures;
            // The takeover is over before the engine is handed anything: the flag is cleared
            // first, so a panic in the engine cannot leave it set.
            let held = Held {
                groups: key.scene.groups.iter().map(|g| g.id.clone()).collect(),
                takeover: Vec::new(),
            };
            app.scene.on.store(true, Ordering::SeqCst);
            app.scene.set_held(held);
            app.scene.notify();
            let groups: Vec<EngineGroup> = key
                .scene
                .groups
                .iter()
                .map(|group| EngineGroup {
                    id: group.id.clone(),
                    members: group.members.clone(),
                    mode: group.mode,
                    sequence: group.sequence,
                    brightness: group.brightness,
                    effect: None,
                    props: match &group.plays {
                        Plays::Effect { props, .. } => props.clone(),
                        Plays::Off => Props::new(),
                    },
                })
                .collect();
            tracing::error!(error = %e, "the scene could not be applied: the build task failed; the groups are dark");
            app.engine.set_scene(key.lights.clone(), key.devices.clone(), groups, false);
        }
    }
}

// ---------------------------------------------------------------- takeovers

/// The id of the group a takeover uses when no group of the active scene plays the effect: one
/// temporary flow-across group of every light.
pub const TAKEOVER_GROUP: &str = "takeover";

/// How a takeover makes its effect: one call per group it lands on, with that group's id. It
/// is an `Arc` so it can cross to the blocking thread that builds the effect.
pub type EffectBuilder = Arc<dyn Fn(&str) -> Result<Box<dyn Effect>, String> + Send + Sync>;

/// One group, with what the engine needs to run it.
#[derive(Clone, Debug)]
pub struct GroupRun {
    /// The group's id.
    pub id: String,
    /// The group's members, in the owner's order.
    pub members: Vec<LightId>,
    /// How the group's members share the canvas.
    pub mode: GroupMode,
    /// How a pattern runs through the members (#468 phase 4).
    pub sequence: Option<canvas_proto::scene::SequenceShape>,
    /// Every light the daemon knows.
    pub lights: Vec<Light>,
    /// The devices the daemon sees.
    pub devices: Vec<DeviceInfo>,
}

impl GroupRun {
    /// The engine's group, with an effect on it.
    pub fn engine_group(&self, effect: Option<Box<dyn Effect>>, props: &Props) -> EngineGroup {
        EngineGroup {
            id: self.id.clone(),
            members: self.members.clone(),
            mode: self.mode,
            sequence: self.sequence,
            brightness: GROUP_BRIGHTNESS_MAX,
            effect,
            props: props.clone(),
        }
    }
}

/// The group of the active scene with this id, with what the engine needs to run it.
/// `None` when the active scene has no group with that id.
pub async fn group_run(app: &App, id: &str) -> Option<GroupRun> {
    let (group, lights) = {
        let store = app.store.read().await;
        let scene = active_scene(&store.scenes);
        let group = scene.groups.iter().find(|g| g.id == id)?.clone();
        (group, store.lights.lights.clone())
    };
    let devices = app.devices.read().await.clone();
    Some(GroupRun {
        id: group.id,
        members: group.members,
        mode: group.mode,
        sequence: group.sequence,
        lights,
        devices,
    })
}

/// The temporary group of every light, flowing across: what a takeover with no group of its
/// own shows, and what a stack preview with no group shows.
pub async fn every_light_run(app: &App) -> GroupRun {
    let lights = app.store.read().await.lights.lights.clone();
    let devices = app.devices.read().await.clone();
    GroupRun {
        id: TAKEOVER_GROUP.to_string(),
        members: lights.iter().map(|l| l.id.clone()).collect(),
        mode: GroupMode::FlowAcross,
        sequence: None,
        lights,
        devices,
    }
}

/// The scenes that have a group playing `effect`, by id and name.
pub async fn scenes_playing(app: &App, effect: &str) -> Vec<(String, String)> {
    let store = app.store.read().await;
    store
        .scenes
        .scenes
        .iter()
        .filter(|scene| {
            scene.groups.iter().any(|group| match &group.plays {
                // The store matches a pattern's id ignoring ASCII case (`Patterns::get`), so a
                // group that names it in other case still plays it: the refusal must see it too.
                Plays::Effect { effect: playing, .. } => playing.eq_ignore_ascii_case(effect),
                Plays::Off => false,
            })
        })
        .map(|scene| (scene.id.clone(), scene.name.clone()))
        .collect()
}

/// Publishes the masks of the active scene's groups: a stack resolves its layers in the group
/// it plays in.
pub async fn publish_group_areas(app: &Arc<App>) {
    let (lights, devices) = {
        let store = app.store.read().await;
        (store.lights.lights.clone(), app.devices.read().await.clone())
    };
    let scene = {
        let store = app.store.read().await;
        active_scene(&store.scenes).clone()
    };
    let mut maps = BTreeMap::new();
    for group in &scene.groups {
        let plan = canvas_core::arrangement::build_group_plan(
            &group.members,
            group.mode,
            &lights,
            &devices,
        );
        maps.insert(
            group.id.clone(),
            crate::stack::AreaMap::build_for_group(
                &plan,
                &lights,
                canvas_core::SCENE_SAMPLING,
                0,
            ),
        );
    }
    // The temporary group of every light: what a takeover with no group of its own shows.
    let plan = canvas_core::arrangement::build_group_plan(
        &every_light_run(app).await.members,
        GroupMode::FlowAcross,
        &lights,
        &devices,
    );
    maps.insert(
        TAKEOVER_GROUP.to_string(),
        crate::stack::AreaMap::build_for_group(
            &plan,
            &lights,
            canvas_core::SCENE_SAMPLING,
            0,
        ),
    );
    app.stacks.publish_group_areas(maps);
}

/// The groups of the active scene that play `effect`, by id, in the scene's order.
pub async fn groups_playing(app: &App, effect: &str) -> Vec<String> {
    let store = app.store.read().await;
    active_scene(&store.scenes)
        .groups
        .iter()
        .filter(|group| match &group.plays {
            Plays::Effect { effect: playing, .. } => playing == effect,
            Plays::Off => false,
        })
        .map(|group| group.id.clone())
        .collect()
}

/// The schema of a built effect, behind the guard: a panicking schema is an error, never a
/// crash.
fn schema_of(effect: &dyn Effect) -> Result<canvas_proto::props::PropertySchema, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        canvas_core::effect_guard::run_guarded(|| effect.schema())
    }))
    .map_err(|payload| format!("the effect's schema panicked: {}", crate::panic_payload_text(&payload)))
}

/// Publishes the masks of one group, when the book does not hold it yet.
///
/// The book holds the active scene's groups (and the takeover group's) after the scene is
/// applied. A stack preview shows a group of its own, and a group the book does not hold would
/// resolve to nothing ("not in this group"), so the preview publishes its own.
pub async fn publish_area_for(app: &Arc<App>, group: &str) {
    let book = app.stacks.areas();
    if crate::stack::read_recovering(&book).groups.contains_key(group) {
        return;
    }
    let run = match group_run(app, group).await {
        Some(run) => run,
        None => every_light_run(app).await,
    };
    let plan = canvas_core::arrangement::build_group_plan(
        &run.members,
        run.mode,
        &run.lights,
        &run.devices,
    );
    let map = crate::stack::AreaMap::build_for_group(
        &plan,
        &run.lights,
        canvas_core::SCENE_SAMPLING,
        0,
    );
    app.stacks.publish_group_area(group.to_string(), map);
}

/// The groups a takeover of `effect` lands on: the groups of the active scene whose `Plays`
/// names it, or the one temporary group of every light when none does.
///
/// The caller reads this before it builds the takeover's effect, so a stack is built for the
/// group it lands on and resolves its areas in that group's canvas.
pub async fn takeover_groups(app: &App, effect: &str) -> Vec<String> {
    let groups = groups_playing(app, effect).await;
    if groups.is_empty() {
        vec![TAKEOVER_GROUP.to_string()]
    } else {
        groups
    }
}

/// The takeover's effect, and the groups it goes on.
struct Takeover<'a> {
    /// The groups the takeover goes on.
    groups: &'a [String],
    /// The props the takeover's effect runs with.
    props: &'a Props,
    /// The builder, for the groups beyond the first.
    build: &'a EffectBuilder,
    /// The instance the caller built for the first group, and the group it was built for.
    ready: Option<(String, Box<dyn Effect>)>,
}

impl Takeover<'_> {
    /// The instance the caller built, when it was built for `group`.
    fn take_ready(&mut self, group: &str) -> Option<Box<dyn Effect>> {
        if let Some((for_group, effect)) = self.ready.take() {
            if for_group == group {
                return Some(effect);
            }
            // It was built for another group: a stack resolves its areas in the group's canvas,
            // so it cannot be used here.
            self.ready = None;
        }
        None
    }

    /// The instance for `group`: the one the caller built, when it was built for that group,
    /// or a new one.
    async fn instance(&mut self, group: &str) -> Result<Box<dyn Effect>, String> {
        if let Some(effect) = self.take_ready(group) {
            return Ok(effect);
        }
        build_one(self.build, group).await
    }

    /// The instance for `group`, built on this thread: the same, without the await.
    fn instance_blocking(&mut self, group: &str) -> Result<Box<dyn Effect>, String> {
        if let Some(effect) = self.take_ready(group) {
            return Ok(effect);
        }
        (self.build)(group)
    }
}

/// What one group needs from the screen and the audio capture.
fn group_needs(app: &App, group: &Group) -> InputNeeds {
    let mut needs = InputNeeds::default();
    if let Plays::Effect { effect, .. } = &group.plays {
        if let Some(schema) = crate::api::effects::find_schema(&app.user_effects, &app.stacks, &app.patterns, effect) {
            needs.screen |= schema.needs.screen;
            needs.audio |= schema.needs.audio;
        }
    }
    needs
}

/// What the engine's groups need once the takeover is on them: the takeover's effect and the
/// scene's groups it does not hold.
///
/// `apply_needs` sets what the captures do, it does not merge: the needs of every group the
/// engine holds have to be in it, or the capture another group needs stops.
fn needs_with_takeover(
    app: &App,
    scene: &Scene,
    targets: &[String],
    draft: InputNeeds,
) -> InputNeeds {
    let mut needs = draft;
    for group in &scene.groups {
        if targets.contains(&group.id) {
            // The takeover's effect is on that group, not the one the scene names.
            continue;
        }
        let group_needs = group_needs(app, group);
        needs.screen |= group_needs.screen;
        needs.audio |= group_needs.audio;
    }
    needs
}

/// Puts a takeover on the lights: the effect on every group of the active scene whose `Plays`
/// names it, or on one temporary flow-across group of every light when no group plays it.
///
/// The engine decides how: an effect for a group the engine does not have is dropped, so the
/// takeover only hands one to a group the engine holds. When the engine holds the takeover
/// group (a draft of an effect no group plays) and the takeover now lands on a group of the
/// scene, or when the groups of the last takeover are not the groups of this one, the whole
/// scene goes on again.
///
/// `ready` is an instance the caller built outside the gate, with the group it was built for:
/// it goes on that group, so the caller does not build it twice. `build` makes the rest, one per
/// group. It runs on a blocking thread: building an HTML effect runs its scripts, which must
/// not block the async workers.
///
/// Returns the groups the takeover is on: the scene's own group ids, or [`TAKEOVER_GROUP`].
pub async fn apply_takeover(
    app: &Arc<App>,
    effect: &str,
    props: &Props,
    ready: Option<(String, Box<dyn Effect>)>,
    build: EffectBuilder,
) -> Result<Vec<String>, String> {
    let target = takeover_groups(app, effect).await;
    let (scene, lights, devices) = {
        let store = app.store.read().await;
        (
            active_scene(&store.scenes).clone(),
            store.lights.lights.clone(),
            app.devices.read().await.clone(),
        )
    };
    let scene_groups: Vec<String> = scene.groups.iter().map(|g| g.id.clone()).collect();
    let held = app.scene.held();
    let mut takeover = Takeover { groups: &target, props, build: &build, ready };

    // The takeover group: the engine holds that group and nothing else.
    if target.len() == 1 && target[0] == TAKEOVER_GROUP {
        if lights.is_empty() {
            return Ok(Vec::new());
        }
        let made = takeover.instance(TAKEOVER_GROUP).await?;
        let schema = schema_of(made.as_ref())?;
        let group = EngineGroup {
            id: TAKEOVER_GROUP.to_string(),
            members: lights.iter().map(|l| l.id.clone()).collect(),
            mode: GroupMode::FlowAcross,
            sequence: None,
            brightness: GROUP_BRIGHTNESS_MAX,
            effect: Some(made),
            props: props.clone(),
        };
        if !schema.id.starts_with(canvas_proto::stack::STACK_ID_PREFIX) {
            app.stacks.clear_running();
        }
        app.engine.set_scene(lights, devices, vec![group], false);
        app.inputs.apply_needs(schema.needs);
        // The engine's groups are not the store's now: a scene change takes the whole scene.
        app.scene.land_takeover(Held {
            groups: vec![TAKEOVER_GROUP.to_string()],
            takeover: target.clone(),
        });
        app.scene.notify();
        return Ok(target);
    }

    // The engine holds the scene's groups and the groups the last takeover held are among the
    // ones this one lands on: only the target groups change.
    let keeps = held.takeover.iter().all(|group| target.contains(group));
    if holds_groups(&held.groups, &scene_groups) && keeps {
        // Every instance is built before any of them lands: a build that fails leaves the engine
        // as it was, with the groups it had.
        let mut instances = Vec::with_capacity(target.len());
        let mut draft_needs = InputNeeds::default();
        let mut stack = true;
        for group in &target {
            let made = takeover.instance(group).await?;
            let schema = schema_of(made.as_ref())?;
            draft_needs.screen |= schema.needs.screen;
            draft_needs.audio |= schema.needs.audio;
            if !schema.id.starts_with(canvas_proto::stack::STACK_ID_PREFIX) {
                stack = false;
            }
            instances.push((group.clone(), made));
        }
        if !stack {
            app.stacks.clear_running();
        }
        // The engine holds the scene's groups with the takeover's effect on the target ones: what
        // it holds needs the captures of every group it holds.
        let needs = needs_with_takeover(app, &scene, &target, draft_needs);
        for (group, made) in instances {
            app.engine.set_group_effect(&group, Some(made), props);
        }
        app.inputs.apply_needs(needs);
        app.scene.land_takeover(Held { groups: scene_groups, takeover: target.clone() });
        app.scene.notify();
        return Ok(target);
    }

    // The engine does not have the groups this takeover lands on, or the groups the last one
    // held are not among them: the whole scene goes on again, with the takeover's effect on
    // the target groups and the scene's own effect on the others.
    let built = {
        let app = Arc::clone(app);
        let lights = lights.clone();
        let devices = devices.clone();
        let scene = scene.clone();
        let target = target.clone();
        let props = props.clone();
        let build = Arc::clone(&build);
        let ready = takeover.ready.take();
        tokio::task::spawn_blocking(move || {
            let mut takeover = Takeover {
                groups: &target,
                props: &props,
                build: &build,
                ready,
            };
            let mut failures = BTreeMap::new();
            let outcome =
                build_and_set_with(&app, &lights, &devices, &scene, &mut takeover, &mut failures);
            (outcome, failures)
        })
        .await
    };
    match built {
        Ok((Ok(()), failures)) => {
            // The groups whose effect could not be built are dark, and the API says so.
            *app.scene.failures.lock().unwrap_or_else(|e| e.into_inner()) = failures;
            app.scene.land_takeover(Held { groups: scene_groups, takeover: target.clone() });
            app.scene.notify();
            Ok(target)
        }
        Ok((Err(why), failures)) => {
            // The build failed: nothing landed, so the engine holds what it held, and the
            // failures are the ones it had. The partial map is not written over them.
            tracing::warn!(error = %why, failures = ?failures, "the takeover's build failed; the engine holds what it held");
            Err(why)
        }
        Err(e) => Err(format!("the takeover's build task failed: {e}")),
    }
}

/// Builds every group's effect and hands the scene to the engine, with the takeover's effect on
/// the groups it names. A group whose effect cannot be built is dark (`effect: None`) and named
/// in the failures.
///
/// A takeover's effect that cannot be built stops the whole thing: the groups it would have
/// landed on keep what they had.
fn build_and_set_with(
    app: &App,
    lights: &[Light],
    devices: &[DeviceInfo],
    scene: &Scene,
    takeover: &mut Takeover<'_>,
    failures: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let mut needs = InputNeeds::default();
    let mut groups = Vec::with_capacity(scene.groups.len());
    for group in &scene.groups {
        let (effect, group_props) = if takeover.groups.contains(&group.id) {
            let made = takeover.instance_blocking(&group.id)?;
            let schema = schema_of(made.as_ref())?;
            needs.screen |= schema.needs.screen;
            needs.audio |= schema.needs.audio;
            (Some(made), takeover.props.clone())
        } else {
            match &group.plays {
                Plays::Off => (None, Props::new()),
                Plays::Effect { effect: id, props } => match build_effect(app, id, Some(&group.id)) {
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
            }
        };
        groups.push(EngineGroup {
            id: group.id.clone(),
            members: group.members.clone(),
            mode: group.mode,
            sequence: group.sequence,
            brightness: group.brightness,
            effect,
            props: group_props,
        });
    }
    // Every group is built: anything that is not a stack means no stack is on the engine now
    // (whatever was running is gone, and a later activation of it must build again).
    app.stacks.clear_running();
    app.inputs.apply_needs(needs);
    app.engine.set_scene(lights.to_vec(), devices.to_vec(), groups, false);
    Ok(())
}

/// Puts a pattern draft on the lights: every light the pattern fits exactly shows it, and the
/// active scene's other lights stay on at a dim level.
///
/// First call: the whole scene goes on again with (a) one group,
/// [`canvas_proto::pattern::PATTERN_EDIT_GROUP`], holding every light whose fit against the
/// pattern's `built_on` is `Exact` and playing the draft, and (b) the active scene's groups with
/// those lights taken out of their members (a group left empty is dropped) at their own
/// brightness times [`canvas_proto::pattern::PATTERN_EDIT_DIM_PERCENT`] / 100.
///
/// A later call for the pattern already on the lights (a stroke) swaps the group's effect with
/// `engine.set_group_effect` and builds nothing: painting stays cheap.
///
/// Returns the number of lights lit; zero means no device of that kind exists.
pub async fn apply_pattern_takeover(
    app: &Arc<App>,
    pattern: &canvas_proto::pattern::Pattern,
) -> Result<usize, String> {
    let (scene, lights, devices) = {
        let store = app.store.read().await;
        (
            active_scene(&store.scenes).clone(),
            store.lights.lights.clone(),
            app.devices.read().await.clone(),
        )
    };
    let lit: Vec<LightId> = lights
        .iter()
        .filter(|light| {
            canvas_proto::pattern::fit_kind(
                &pattern.built_on,
                canvas_core::arrangement::light_led_count(light, &devices),
                light.arrangement,
            ) == canvas_proto::pattern::FitKind::Exact
        })
        .map(|light| light.id.clone())
        .collect();

    // A stroke of the pattern already on the lights: the engine holds the pattern's group, with
    // the members this pattern fits, so one swap is the whole change.
    let held = app.scene.held();
    let playing = app.draft.snapshot().and_then(|slot| match slot.kind {
        crate::draft::SlotKind::Pattern { pattern_id, pattern: playing } if pattern_id == pattern.id => {
            Some(playing)
        }
        _ => None,
    });
    let same_draft = playing.is_some();
    if same_draft && held.groups.iter().any(|group| group == canvas_proto::pattern::PATTERN_EDIT_GROUP) {
        // The very document already on the lights (the editor's heartbeat): nothing to swap. A swap
        // installs a new effect, which restarts the effect's clock and fits every frame again, so
        // a playing pattern would start over every heartbeat (#481).
        if playing.as_deref() == Some(pattern) {
            return Ok(lit.len());
        }
        app.engine.set_group_effect(
            canvas_proto::pattern::PATTERN_EDIT_GROUP,
            Some(Box::new(crate::pattern_effect::PatternEffect::new(pattern.clone()))),
            &Props::new(),
        );
        // The takeover is the same one, and the generation moves so a guard does not put the
        // scene over it.
        app.scene.land_takeover(held);
        app.scene.notify();
        return Ok(lit.len());
    }

    // The whole scene goes on again. A dimmed group's effect may be a stack with HTML layers, so
    // it is built on a blocking thread.
    let built = {
        let app = Arc::clone(app);
        let scene = scene.clone();
        let pattern = pattern.clone();
        let lit = lit.clone();
        tokio::task::spawn_blocking(move || {
            build_pattern_takeover(&app, &scene, &pattern, &lit)
        })
        .await
    };
    let PatternTakeover { groups, failures, needs, held: held_groups } = built
        .map_err(|error| format!("the pattern takeover task failed: {error}"))??;
    app.stacks.clear_running();
    app.inputs.apply_needs(needs);
    app.engine.set_scene(lights, devices, groups, false);
    *app.scene.failures.lock().unwrap_or_else(|e| e.into_inner()) = failures;
    // Every group the takeover made is the takeover's, the pattern's own and the dimmed ones: a
    // scene change and a pattern edit both wait for it to end.
    app.scene.land_takeover(Held { groups: held_groups.clone(), takeover: held_groups });
    app.scene.notify();
    Ok(lit.len())
}

/// What a pattern takeover built: the groups the engine is handed, why any dimmed group is dark,
/// what the captures are needed for, and the ids of the groups the takeover made its own.
struct PatternTakeover {
    groups: Vec<EngineGroup>,
    failures: BTreeMap<String, String>,
    needs: InputNeeds,
    held: Vec<String>,
}

/// The groups of a pattern takeover: the pattern's own group and the scene's groups at their dim
/// level, with the lights the pattern fits taken out of them.
fn build_pattern_takeover(
    app: &App,
    scene: &Scene,
    pattern: &canvas_proto::pattern::Pattern,
    lit: &[LightId],
) -> Result<PatternTakeover, String> {
    let mut groups = Vec::with_capacity(scene.groups.len() + 1);
    let mut held = Vec::with_capacity(scene.groups.len() + 1);
    let mut needs = InputNeeds::default();
    let mut failures = BTreeMap::new();
    // No light of the pattern's kind: no group of its own. There is nothing to show, and a group
    // with no member cannot be planned.
    if !lit.is_empty() {
        groups.push(EngineGroup {
            id: canvas_proto::pattern::PATTERN_EDIT_GROUP.to_string(),
            members: lit.to_vec(),
            mode: GroupMode::FlowAcross,
            sequence: None,
            brightness: GROUP_BRIGHTNESS_MAX,
            effect: Some(Box::new(crate::pattern_effect::PatternEffect::new(pattern.clone()))),
            props: Props::new(),
        });
        held.push(canvas_proto::pattern::PATTERN_EDIT_GROUP.to_string());
    }
    let dim = |brightness: u8| -> u8 {
        (brightness as u16 * canvas_proto::pattern::PATTERN_EDIT_DIM_PERCENT as u16 / 100) as u8
    };
    for group in &scene.groups {
        let members: Vec<LightId> = group
            .members
            .iter()
            .filter(|member| !lit.contains(member))
            .cloned()
            .collect();
        if members.is_empty() {
            continue;
        }
        let (effect, props) = match &group.plays {
            Plays::Off => (None, Props::new()),
            Plays::Effect { effect: id, props } => match build_effect(app, id, Some(&group.id)) {
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
            members,
            mode: group.mode,
            sequence: group.sequence,
            brightness: dim(group.brightness),
            effect,
            props,
        });
        held.push(group.id.clone());
    }
    Ok(PatternTakeover { groups, failures, needs, held })
}

/// Builds one instance of a takeover's effect, on a blocking thread: building an HTML effect
/// runs its scripts, which must not block the async workers. The group id is the one the
/// instance is for: a stack resolves its layers in that group's canvas.
async fn build_one(build: &EffectBuilder, group: &str) -> Result<Box<dyn Effect>, String> {
    let build = Arc::clone(build);
    let group = group.to_string();
    tokio::task::spawn_blocking(move || build(&group))
        .await
        .map_err(|e| format!("the effect load task failed: {e}"))?
}

/// Whether `old` and `new` are the same scene with its groups in another order: the same group
/// ids, each with the same members, mode and sequence. The order of the list is display order only
/// (#384), so a reorder is not a change of shape. A group's `sequence` is part of its shape (#468):
/// the engine fits a pattern to the run it makes, so a change of it must reach the engine.
fn same_shape(old: &Scene, new: &Scene) -> bool {
    old.groups.len() == new.groups.len()
        && new.groups.iter().all(|after| {
            old.groups.iter().any(|before| {
                before.id == after.id
                    && before.members == after.members
                    && before.mode == after.mode
                    && before.sequence == after.sequence
            })
        })
}

/// Whether the engine holds exactly `scene_groups`, whatever order each list is in. Group order is
/// display order only (#384): the engine keeps the order it was handed, so a reorder leaves it
/// holding the same groups in its own order.
fn holds_groups(held: &[String], scene_groups: &[String]) -> bool {
    held.len() == scene_groups.len() && scene_groups.iter().all(|id| held.contains(id))
}

/// The groups of `scene` the HTML limit leaves dark, by group id: [`html_runs`]'s rule for the
/// scene a save left, so [`apply_group_change`] can tell a group the limit left dark from one whose
/// effect could not be built (#419). The groups kept from that scene's last build come first, which
/// is the order [`build_and_set`] decided in (#384), not the scene's own order.
fn groups_over_the_html_limit(app: &App, scene: &Scene) -> HashSet<String> {
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    let slots_of = |id: &str| saved_stack_slots(app, id, &known);
    let runs = html_runs(scene, &app.scene.html_kept(&scene.id), &slots_of);
    scene
        .groups
        .iter()
        .zip(&runs)
        .filter(|(_, runs)| !**runs)
        .map(|(group, _)| group.id.clone())
        .collect()
}

/// The effect a group's plays names, when it plays one. A pattern or a stack is an id like any
/// other: whether it spends an HTML runtime is [`group_html_runtimes`]'s question.
fn effect_named(plays: &Plays) -> Option<&str> {
    match plays {
        Plays::Effect { effect, .. } => Some(effect),
        Plays::Off => None,
    }
}

/// Brings the engine from `old` to `new`, the same scene saved with a change. It rebuilds only the
/// groups that changed: a new value for the same effect (`set_group_props`), a new brightness, a
/// group set to Off or to another effect (`set_group_effect` for that group alone). When a group
/// was added or removed, or its members or mode changed, the engine takes the whole scene again
/// (`set_scene` is the only way members reach it), so every effect is built again. A reorder of
/// the groups is not a change: the groups are paired by id, and the order they are listed in is
/// display order only (#384).
///
/// A takeover (a draft, the onboarding greeting) holds the lights: the engine's groups are the
/// takeover's, so a group the takeover holds is left alone and takes the change when the
/// takeover ends (its restore applies the active scene). A change of the scene's shape needs the
/// whole scene, which would put the scene over the takeover, so it waits for the takeover too.
pub async fn apply_group_change(app: &Arc<App>, lock: &SceneLock<'_>, old: &Scene, new: &Scene) {
    let held = app.scene.held().takeover;
    let taken_over = !held.is_empty();
    if !same_shape(old, new) {
        if taken_over {
            // The engine takes the whole scene for a change of its shape, and that would put the
            // scene over the takeover. The change waits for the takeover to end.
            return;
        }
        apply_with(app, lock, false).await;
        return;
    }
    // The engine holds other groups than the scene's (a takeover held the takeover group of
    // every light): the whole scene goes on again, so the engine holds the scene's groups.
    let scene_groups: Vec<String> = new.groups.iter().map(|g| g.id.clone()).collect();
    if !taken_over && !holds_groups(&app.scene.held().groups, &scene_groups) {
        apply_with(app, lock, false).await;
        return;
    }
    let mut failures = app.scene.failures();
    // The groups the HTML limit leaves dark, read from the scene the save left, in the order
    // `build_and_set` decided in (#419, #384).
    let over_the_limit = groups_over_the_html_limit(app, new);
    for after in &new.groups {
        // A reorder pairs the groups by id, not by place (#384); a group the save removed is not
        // here, and a group it added has no before, so the shape changed and the whole scene went
        // on above.
        let Some(before) = old.groups.iter().find(|g| g.id == after.id) else { continue };
        // A group the takeover holds keeps the draft: its change lands when the takeover ends.
        if taken_over && held.contains(&after.id) {
            continue;
        }
        // A group that is dark because its effect could not be built is tried again on every
        // save, even one that names the same effect: the file may be back. A group the HTML limit
        // left dark is not (#419): this save freed no runtime, so building it would put the
        // engine over the cap the scene was loaded past. It is built again when its own effect
        // changes, or when a full build decides it runs.
        let dark = failures.contains_key(&after.id);
        let limited =
            dark && over_the_limit.contains(&after.id) && effect_named(&before.plays) == effect_named(&after.plays);
        if before.plays != after.plays || (dark && !limited) {
            match (&before.plays, &after.plays) {
                (Plays::Effect { effect: was, .. }, Plays::Effect { effect: now, props }) if was == now && (!dark || limited) => {
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
                        let group = after.id.clone();
                        tokio::task::spawn_blocking(move || build_effect(&app, &id, Some(&group))).await
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
    // The captures follow what the engine runs: a takeover holds the lights, and it set the
    // needs its own effect has.
    if !taken_over {
        app.inputs.apply_needs(needs_of(app, new));
    }
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

/// After a pattern is saved or deleted: every group of the active scene that plays `id` takes the
/// new one, or goes dark with the failure text when the pattern is gone.
///
/// A group a takeover holds is left alone: the takeover's own end applies the active scene, which
/// builds the saved pattern. A group of a scene that is not active takes the change when that
/// scene is applied. Nothing here writes the scene: the store already names the pattern.
pub async fn pattern_changed(app: &Arc<App>, id: &str) {
    // The scene order, as a save takes it: a scene write and this change never interleave.
    let _lock = app.scene.lock().await;
    let scene = {
        let store = app.store.read().await;
        active_scene(&store.scenes).clone()
    };
    let held = app.scene.held();
    let scene_groups: Vec<String> = scene.groups.iter().map(|g| g.id.clone()).collect();
    // A takeover holds groups that are not the scene's (the takeover group of every light): the
    // engine has no scene group to change, and the takeover's end applies the scene, which builds
    // the pattern as it is now.
    if !holds_groups(&held.groups, &scene_groups) {
        tracing::debug!(
            pattern = %id,
            "a takeover holds the lights; the pattern reaches its groups when the scene goes back"
        );
        return;
    }
    let mut failures = app.scene.failures();
    let mut changed = false;
    for group in &scene.groups {
        let Plays::Effect { effect, props } = &group.plays else { continue };
        // A group that names the pattern in other case plays it (`Patterns::get` matches that
        // way), so the edit must reach it: compare as the store does.
        if !effect.eq_ignore_ascii_case(id) {
            continue;
        }
        // A group the takeover holds keeps the draft: its change lands when the takeover ends.
        if held.takeover.contains(&group.id) {
            continue;
        }
        changed = true;
        let built = {
            let app = Arc::clone(app);
            let id = effect.clone();
            let group = group.id.clone();
            tokio::task::spawn_blocking(move || build_effect(&app, &id, Some(&group))).await
        };
        let built = built.unwrap_or_else(|e| Err(format!("the build task failed: {e}")));
        match built {
            Ok((made, _)) => {
                failures.remove(&group.id);
                app.engine.set_group_effect(&group.id, Some(made), props);
            }
            Err(why) => {
                // A pattern that is gone is a build failure like a missing HTML file: only this
                // group is dark, and the API names it.
                tracing::warn!(group = %group.id, pattern = %effect, error = %why, "a group's pattern is gone; the group is dark");
                failures.insert(group.id.clone(), why);
                app.engine.set_group_effect(&group.id, None, props);
            }
        }
    }
    if !changed {
        return;
    }
    *app.scene.failures.lock().unwrap_or_else(|e| e.into_inner()) = failures;
    app.scene.notify();
}

/// The fit of each of a group's member lights against the pattern it plays. Empty for a group
/// that plays no pattern, and for one whose pattern is gone (the group is dark and says why).
///
/// A light with no LEDs is left out: its device is not connected, or its zone reports none, so
/// the engine plays nothing there. It is neither exact nor flagged, and the picker's count is of
/// the lights the pattern lands on.
///
/// A group with a `sequence` (#468) fits the pattern once to all its lights' LEDs joined end to
/// end, so every light reports the fit of that whole run, not of its own LED count.
fn group_fits(app: &App, group: &Group, lights: &[Light], devices: &[DeviceInfo]) -> Vec<LightFit> {
    let Plays::Effect { effect, .. } = &group.plays else {
        return Vec::new();
    };
    if !effect.starts_with(canvas_proto::pattern::PATTERN_ID_PREFIX) {
        return Vec::new();
    }
    let Some(pattern) = app.patterns.get(effect) else {
        return Vec::new();
    };
    // The members that have LEDs, in the group's order, with their counts.
    let placed: Vec<(&LightId, &Light, u32)> = group
        .members
        .iter()
        .filter_map(|member| {
            // A member the daemon does not hold has no LEDs to play on.
            let light = lights.iter().find(|light| &light.id == member)?;
            let led_count = canvas_core::arrangement::light_led_count(light, devices);
            (led_count > 0).then_some((member, light, led_count))
        })
        .collect();
    // A sequence (#468): the pattern is fitted once to every member's LEDs joined end to end, as
    // one ring or one line, so every member reports the fit of that whole run.
    let run = group.sequence.map(|shape| {
        let total: u32 = placed.iter().map(|(_, _, count)| *count).sum();
        let arrangement = match shape {
            canvas_proto::scene::SequenceShape::Ring => canvas_proto::scene::Arrangement::Ring,
            canvas_proto::scene::SequenceShape::Line => canvas_proto::scene::Arrangement::Strip,
        };
        canvas_proto::pattern::fit_kind(&pattern.built_on, total, arrangement)
    });
    placed
        .into_iter()
        .map(|(member, light, led_count)| LightFit {
            light: member.clone(),
            fit: run.unwrap_or_else(|| {
                canvas_proto::pattern::fit_kind(&pattern.built_on, led_count, light.arrangement)
            }),
        })
        .collect()
}

/// How the groups of `scene` are doing: dark because the effect could not be built, or because it
/// panicked while it ran. One entry per group, in the scene's order.
///
/// A group that plays a pattern carries the fit of each of its member lights, worked out from
/// `lights` and the devices as they are now: nothing is cached, so a resize shows at the next read.
///
/// The engine's own states come from the daemon's watcher threads, so this never waits for the
/// render thread while the caller holds the store's lock (#241). A daemon whose engine has never
/// answered says so: an empty list would say that no group failed, and a group can fail.
pub async fn group_states(app: &App, scene: &Scene, lights: &[Light]) -> crate::api::ApiResult<Vec<GroupState>> {
    let running = app
        .engine_group_states()
        .await
        .ok_or_else(|| crate::api::ApiFailure::service_unavailable(crate::state::ENGINE_NOT_ANSWERING))?;
    Ok(group_states_of(app, scene, lights, Some(&running)).await)
}

/// The group states a route that has already written answers with (#241).
///
/// The write landed; only the read that follows it may be impossible, and a 503 then says the
/// write failed when it did not. The answer is the state the daemon knows: the engine's last
/// answer when there is one, and the scene's own groups when there is not. The daemon says
/// nothing about the groups the engine never told it about.
pub async fn group_states_known(app: &App, scene: &Scene, lights: &[Light]) -> Vec<GroupState> {
    let running = app.engine_group_states().await;
    group_states_of(app, scene, lights, running.as_deref()).await
}

/// The group states of `scene`, with the engine's own states when they are known.
async fn group_states_of(
    app: &App,
    scene: &Scene,
    lights: &[Light],
    running: Option<&[GroupState]>,
) -> Vec<GroupState> {
    let built = app.scene.failures();
    let devices = app.devices.read().await.clone();
    scene
        .groups
        .iter()
        .map(|g| GroupState {
            fits: group_fits(app, g, lights, &devices),
            id: g.id.clone(),
            failed: built.get(&g.id).cloned().or_else(|| {
                running
                    .and_then(|states| states.iter().find(|s| s.id == g.id))
                    .and_then(|s| s.failed.clone())
            }),
        })
        .collect()
}

/// How many HTML runtimes `scene` starts, in scene order (#370): one for an `html:` group, and one
/// for each HTML layer or mask of a stack. A stack with two HTML layers is two, and `slots_of`
/// answers that for the effect ids a stack has (design Q5: runtimes, not groups).
///
/// **The one count.** Every door reads it: save and create ([`check_saveable`]), setting a scene
/// and activating (`api/scenes.rs`), a takeover ([`check_takeover_html_limit`], `draft.rs`) and
/// saving a stack ([`check_stack_html_limit`], `api/stacks.rs`). [`build_and_set`] uses it to
/// decide which groups run, in the order it keeps by group id (#384).
pub fn html_runtime_count(scene: &Scene, slots_of: &dyn Fn(&str) -> usize) -> usize {
    scene.groups.iter().map(|group| group_html_runtimes(group, slots_of)).sum()
}

/// The HTML runtimes one group starts: 1 when it plays an `html:` effect, `slots_of` when it plays
/// anything else (a stack's HTML slots, and 0 for a canvas effect).
pub fn group_html_runtimes(group: &Group, slots_of: &dyn Fn(&str) -> usize) -> usize {
    match &group.plays {
        Plays::Effect { effect, .. } if effect.starts_with(HTML_EFFECT_PREFIX) => 1,
        Plays::Effect { effect, .. } => slots_of(effect),
        Plays::Off => 0,
    }
}

/// [`html_runtime_count`] against the app's own stacks: the count every door uses.
pub fn scene_html_runtimes(app: &App, scene: &Scene) -> usize {
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    html_runtime_count(scene, &|id| saved_stack_slots(app, id, &known))
}

/// The HTML slots of the saved stack `id`: its HTML layers and the HTML masks that resolve, by
/// the same rule that caps a stack ([`crate::stacks::html_slots`]).
fn saved_stack_slots(app: &App, id: &str, known: &crate::stacks::KnownEffects) -> usize {
    app.stacks.get(id).map_or(0, |stack| stack_slots(&stack, known))
}

/// The HTML slots of `stack`, layer by layer.
fn stack_slots(stack: &canvas_proto::stack::Stack, known: &crate::stacks::KnownEffects) -> usize {
    stack.layers.iter().map(|layer| crate::stacks::html_slots(layer, known)).sum()
}

/// The groups of `scene` whose pattern would keep more fitted LED-frames than the scene may
/// ([`SCENE_PATTERN_LED_FRAMES_MAX`], #481), each with the sentence its row carries. The patterns of
/// the scene's groups are counted in the order of the groups' ids, never of their places, so a
/// reorder never changes which group runs (the rule #384 set for the HTML runtimes); a group that
/// does not fit is dark and the others run. Lights of one shape count once, as the engine fits them.
fn pattern_budget_refusals(app: &App, scene: &Scene, lights: &[Light], devices: &[DeviceInfo]) -> BTreeMap<String, String> {
    let mut refused = BTreeMap::new();
    let mut by_id: Vec<&canvas_proto::scene::Group> = scene.groups.iter().collect();
    by_id.sort_by(|a, b| a.id.cmp(&b.id));
    let mut kept: u64 = 0;
    for group in by_id {
        let Plays::Effect { effect: id, .. } = &group.plays else { continue };
        if !id.starts_with(canvas_proto::pattern::PATTERN_ID_PREFIX) {
            continue;
        }
        let Some(pattern) = app.patterns.get(id) else { continue };
        let shapes: Vec<(u32, canvas_proto::scene::Arrangement)> = lights
            .iter()
            .filter(|light| group.members.contains(&light.id))
            .map(|light| (canvas_core::arrangement::light_led_count(light, devices), light.arrangement))
            .collect();
        let cost = canvas_core::pattern_fit::fitted_led_frames(&pattern, &shapes, group.sequence.is_some());
        if kept + cost > canvas_core::pattern_fit::SCENE_PATTERN_LED_FRAMES_MAX {
            refused.insert(group.id.clone(), pattern_budget_sentence(&pattern, cost, kept));
        } else {
            kept += cost;
        }
    }
    refused
}

/// The sentence for a pattern that would keep more fitted frames than a scene may (#481): the
/// limit, the pattern's own count and what the scene already keeps, and the way out.
pub fn pattern_budget_sentence(pattern: &canvas_proto::pattern::Pattern, cost: u64, kept: u64) -> String {
    format!(
        "{} has {} frames, and on this group's lights that is {cost} fitted LED-frames. A scene keeps {} at most and this one already keeps {kept}. Give the pattern fewer frames, or play it on fewer lights.",
        pattern.name(),
        pattern.frames.len(),
        canvas_core::pattern_fit::SCENE_PATTERN_LED_FRAMES_MAX,
    )
}

/// The sentence for a scene that starts more HTML runtimes than it may (#370, design Q3), with the
/// limit and the scene's own count in it. One function, so every refusal and the row of every group
/// past the limit say the same words, and the words name the number and the fix.
pub fn html_limit_sentence(count: usize) -> String {
    format!(
        "A scene runs {SCENE_HTML_MAX} HTML effect{} at a time. This one has {count}. Move one to another scene.",
        if SCENE_HTML_MAX == 1 { "" } else { "s" }
    )
}

/// Checks the HTML limit for a takeover of `effect` on `targets`: the scene must not start more
/// HTML runtimes than it may.
///
/// `stack` is the stack the takeover brings, when it brings one: the draft's own layers count,
/// not the saved stack's, because the draft is what runs.
pub async fn check_takeover_html_limit(
    app: &App,
    effect: &str,
    targets: &[String],
    stack: Option<&canvas_proto::stack::Stack>,
) -> Result<(), String> {
    let scene = {
        let store = app.store.read().await;
        active_scene(&store.scenes).clone()
    };
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    // The runtimes each targeted group starts: the takeover's own, on every group it lands on.
    let runtimes = match stack {
        Some(stack) => stack_slots(stack, &known),
        None if effect.starts_with(HTML_EFFECT_PREFIX) => 1,
        None => saved_stack_slots(app, effect, &known),
    };
    let count = if targets.iter().any(|group| group == TAKEOVER_GROUP) {
        // The takeover group replaces the scene's groups, so only the takeover's effect counts.
        runtimes
    } else {
        scene
            .groups
            .iter()
            .map(|group| {
                if targets.contains(&group.id) {
                    runtimes
                } else {
                    group_html_runtimes(group, &|id| saved_stack_slots(app, id, &known))
                }
            })
            .sum()
    };
    if count > SCENE_HTML_MAX {
        return Err(html_limit_sentence(count));
    }
    Ok(())
}

/// Checks the HTML limit for a stack save: a save is refused only when it puts a scene that plays
/// this stack over the limit.
///
/// The rule is per scene, before and after the draft: a scene's count with the stack as saved
/// against its count with the stack being saved. Two things make a save harmless, and both are
/// allowed: a stack no scene plays (its count is the same before and after), and a save that keeps
/// or lowers the count (nothing got worse). The second is what lets the owner fix an over-limit
/// scene by editing the stack, so a pair already over the limit on file is not locked. A save is
/// refused when it raises a scene's count to more than [`SCENE_HTML_MAX`]. The refusal starts
/// with the scene's name: the stack builder shows it, and there "this one" would name nothing.
pub async fn check_stack_html_limit(
    app: &App,
    stack: &canvas_proto::stack::Stack,
) -> Result<(), String> {
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    let draft = stack_slots(stack, &known);
    // The group's effect names this stack as `Stacks::get` finds it: its id or its name, ignoring
    // ASCII case, trimmed. A scene holds either.
    let names_this = |effect: &str| {
        let wanted = effect.trim();
        wanted.eq_ignore_ascii_case(&stack.id) || wanted.eq_ignore_ascii_case(stack.name())
    };
    let scenes = {
        let store = app.store.read().await;
        store.scenes.scenes.clone()
    };
    for scene in &scenes {
        let old = html_runtime_count(scene, &|id| saved_stack_slots(app, id, &known));
        let new = html_runtime_count(scene, &|id| {
            if names_this(id) {
                draft
            } else {
                saved_stack_slots(app, id, &known)
            }
        });
        if new > old && new > SCENE_HTML_MAX {
            return Err(format!("{}: {}", scene.name, html_limit_sentence(new)));
        }
    }
    Ok(())
}

/// `Err` with the sentence to show when `scene` starts more HTML runtimes than a scene may. The
/// spine's `validate_scene` does not count them (#370), so every path that saves or sets a scene
/// calls this, and the user reads this sentence.
pub fn check_html_limit(app: &App, scene: &Scene) -> Result<(), String> {
    let count = scene_html_runtimes(app, scene);
    if count > SCENE_HTML_MAX {
        return Err(html_limit_sentence(count));
    }
    Ok(())
}

/// Every check a scene passes before it is written, in the order a save reports them: the
/// spine's [`canvas_proto::scene::validate_scene`] against `lights`, the scene's and groups'
/// names, then [`check_html_limit`]. Each failure is a 422 with the sentence its own check gives.
///
/// Every new write path (group move, `PUT /api/v1/scenes`, create with `groups`, undo) must call
/// this and not the three checks one by one: a path that forgets one lets a scene onto the disk
/// that the other paths refuse.
pub fn check_saveable(app: &App, scene: &Scene, lights: &[Light]) -> Result<(), crate::api::ApiFailure> {
    use crate::api::ApiFailure;
    canvas_proto::scene::validate_scene(scene, lights).map_err(|e| ApiFailure::validation(e.to_string()))?;
    check_names(scene)?;
    check_html_limit(app, scene).map_err(ApiFailure::validation)
}

/// A scene and its groups have a name, within the length the create route keeps.
fn check_names(scene: &Scene) -> Result<(), crate::api::ApiFailure> {
    use crate::api::ApiFailure;
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
            sequence: None,
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
        g.name = "Ã©".repeat(GROUP_NAME_MAX + 9);
        g.brightness = 255;
        let mut scenes = scene_with(vec![g]);
        scenes.scenes[0].name = "Ã¼".repeat(SCENE_NAME_MAX + 5);
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
    fn repair_keeps_html_groups_over_the_limit() {
        let ls = LightsFile {
            chains: Vec::new(),
            schema_version: LIGHTS_SCHEMA,
            lights: ["a", "b", "c", "d"].iter().map(|id| Light {
                id: (*id).into(),
                device_id: canvas_proto::DeviceId("fake:1".into()),
                zone_index: 0,
                leds: None,
                name: None,
                arrangement: Arrangement::Strip,
                reversed: false,
                kind: None,
                copies: None,
                first_led: None,
            }).collect(),
        };
        let g = |id: &str, member: &str, effect: &str| Group {
            id: id.into(),
            name: id.into(),
            members: vec![member.into()],
            plays: Plays::Effect { effect: effect.into(), props: Default::default() },
            mode: GroupMode::FlowAcross,
            brightness: 100,
            sequence: None,
        };
        // One group over the limit: SCENE_HTML_MAX HTML groups, a built-in one, then one more.
        let mut file = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![Scene {
                id: "s".into(),
                name: "s".into(),
                rev: 1,
                groups: vec![
                    g("g1", "a", "html:ink-drop"),
                    g("g2", "b", "aurora"),
                    g("g3", "c", "html:wet-smear"),
                    g("g4", "d", "html:palette-knife"),
                ],
            }],
        };
        let written = file.clone();
        let changes = repair(&mut file, &ls);
        // #370: repair stops rewriting the owner's scene. The group past the limit is kept as it
        // was saved; `build_and_set` is what does not build it, and its row says why.
        assert_eq!(SCENE_HTML_MAX, 2);
        assert_eq!(file, written, "the scene reads back as it was written, group for group");
        assert!(changes.iter().all(|c| !c.contains("set to Off")), "{changes:?}");
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

    /// #370 counts HTML runtimes, not groups (design Q5): a stack with two HTML layers is two, so
    /// one `html:` group beside it is over the limit of two.
    #[test]
    fn a_stack_with_two_html_layers_counts_as_two_runtimes() {
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
        let slots = |id: &str| if id == "stack:glow" { 2 } else { 0 };
        assert_eq!(html_runtime_count(&scene, &slots), 3, "the html: group and two HTML layers");
        assert_eq!(group_html_runtimes(&scene.groups[1], &slots), 2, "the stack alone");
        assert_eq!(group_html_runtimes(&scene.groups[2], &slots), 0);
        assert_eq!(html_runtime_count(&scene, &|_| 0), 1);
    }

    #[test]
    fn ids_are_slugs_with_a_numeric_suffix_when_taken() {
        assert_eq!(slug("Living room!"), "living-room");
        assert_eq!(slug("  "), "scene");
        assert_eq!(fresh_id("a", ["b"]), "a");
        assert_eq!(fresh_id("a", ["a", "a-2"]), "a-3");
    }

    /// The owner said the header is empty: the next device list must not add the whole-zone light
    /// that the owner's answer was about.
    #[test]
    fn sync_lights_skips_a_zone_marked_empty() {
        let mut lights = empty_lights();
        let board = device("openrgb:mb", 0, vec![zone(0, ZoneKind::Linear, 30), zone(1, ZoneKind::Linear, 12)]);
        lights.chains.push(canvas_proto::scene::ChainRecord {
            device_id: DeviceId("openrgb:mb".into()),
            zone_index: 0,
            state: canvas_proto::scene::ChainState::Empty,
        });
        assert_eq!(sync_lights(&mut lights, &[board]), vec!["openrgb:mb#z1".to_string()]);
        assert!(lights.lights.iter().all(|l| l.id != "openrgb:mb#z0"));
    }

    /// A header the owner did not answer for is a light exactly as it was before chains existed.
    #[test]
    fn sync_lights_still_adds_a_whole_zone_light_for_a_zone_not_set_up() {
        let mut lights = empty_lights();
        let board = device("openrgb:mb", 0, vec![zone(0, ZoneKind::Linear, 30), zone(1, ZoneKind::Linear, 12)]);
        // Recorded as set up: the rule is about `Empty` alone, so both zones get their light.
        lights.chains.push(canvas_proto::scene::ChainRecord {
            device_id: DeviceId("openrgb:mb".into()),
            zone_index: 0,
            state: canvas_proto::scene::ChainState::SetUp,
        });
        assert_eq!(
            sync_lights(&mut lights, &[board]),
            vec!["openrgb:mb#z0".to_string(), "openrgb:mb#z1".to_string()]
        );
    }
}
