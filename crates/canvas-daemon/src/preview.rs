//! The stack preview engine and its slot (D95.4.4).
//!
//! The builder tunes a stack continuously, so the preview is not a render loop of its own: a
//! second engine runs the stack at [`canvas_proto::consts::PREVIEW_MAX_FPS`], gets the same
//! layout and input source as the lights' engine at the same call sites, and never has a device
//! sink, so a preview cannot touch a light. The engine is made the first time the builder opens
//! a preview or its WebSocket, and parks while nothing watches it.
//!
//! A PUT carries a session epoch and a sequence. The newest epoch owns the preview, and within
//! an epoch only a higher sequence is applied; anything else is superseded and answers 200 with
//! `applied: false`. The build runs outside the slot's gate, so Stop, DELETE and a newer PUT do
//! not wait for a slow HTML layer; the gate is taken to read the slot, to swap it and to apply.
//!
//! A PUT whose structure is the one already building joins that build: it updates the live map
//! and the props, makes that build the current one again and starts nothing, so a build longer
//! than the builder's beat still lands, and a structure that comes back (Y -> X -> Y) rejoins
//! the build still making it instead of queuing a second one. A PUT with a different structure
//! starts its own build beside the one in flight and supersedes its result, so the newest beat
//! never waits for a build it has already superseded; the engine is still written only under
//! the gate, so two builds never hand it an effect at once.

use canvas_proto::api::{
    StackLayerState, StackLayerStatus, StackPreviewLayer, StackPreviewResponse,
};
use canvas_proto::props::Props;
use canvas_proto::stack::{Stack, STACK_ID_PREFIX};
use canvas_proto::traits::{Effect, Engine, EngineSettings, InputSource};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::epoch::EpochBook;
use crate::stack::{read_recovering, LiveMap, LiveStack};
use crate::state::{App, PreviewRegistry};

/// The preview engine's settings: 15 fps is its render rate and the WebSocket sink's cap, and
/// it runs at full brightness because the builder shows the stack, not the room.
pub fn engine_settings() -> EngineSettings {
    EngineSettings {
        fps: canvas_proto::consts::PREVIEW_MAX_FPS,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    }
}

/// The preview engine the daemon makes: a second render thread, with no sink of its own.
pub fn real_engine() -> Arc<dyn Engine> {
    canvas_core::spawn(engine_settings())
}

/// The stack a preview is showing, with what it was built from.
pub struct PreviewSlot {
    /// `stack:` + the name, as the builder gave it.
    pub stack_id: String,
    /// The group the preview shows: its layers resolve in that group's canvas.
    pub group: String,
    /// The structure hash the effect was built from (D95.4.6).
    pub structure: String,
    /// The live map the effect holds, so a value change re-applies without a rebuild.
    pub live: LiveStack,
    /// The merged props the effect was initialised with.
    pub props: Props,
    /// What each layer was doing at the last PUT.
    pub layers: Vec<StackPreviewLayer>,
    /// The build errors by effect id, so a later value-only PUT still reports a missing one.
    pub errors: BTreeMap<String, String>,
    /// Where the effect reports a held or faulted HTML layer.
    pub statuses: Arc<Mutex<Vec<StackLayerStatus>>>,
}

/// One build running for this route.
///
/// A PUT whose structure equals a running build joins it: the live map and the props move, no
/// build starts, and that build becomes the current one again, so it lands with the newest
/// values. A PUT with a different structure starts a build of its own and makes it current; the
/// older build finishes and lands nowhere.
#[derive(Clone)]
struct InFlight {
    /// Identifies this build. Every new build takes a new token; the token in the book is the
    /// only build allowed to land.
    token: u64,
    stack_id: String,
    structure: String,
    /// The newest props a PUT (or a beat that joined) gave the effect.
    props: Props,
    /// The map the building effect reads, shared with every beat that joins.
    live: LiveStack,
    /// The build's errors, so a beat that joins can report a missing layer before it lands.
    errors: Arc<Mutex<BTreeMap<String, String>>>,
    /// Where the effect reports a held or faulted HTML layer.
    statuses: Arc<Mutex<Vec<StackLayerStatus>>>,
}

/// Every build running for this route, and which one may land.
///
/// A newer PUT starts its own build beside an older one instead of waiting for it: only the
/// current token lands, and the older result is discarded. A structure that comes back is made
/// current again, so its still-running build lands instead of a third one starting.
#[derive(Default)]
struct Builds {
    /// The builds still running. A build is removed when it lands or lets go.
    running: Vec<InFlight>,
    /// The token of the build whose result may land. A newer PUT, a DELETE or a structure that
    /// is already on screen takes it away.
    current: Option<u64>,
}

/// The preview engine and the slot it is showing.
pub struct StackPreview {
    /// The engine, once something has asked for it. `None` while the builder is closed.
    ///
    /// A std lock: a layout or input-source change is answered from a synchronous call site and
    /// must reach the engine, not be dropped because a tokio lock was busy at that instant.
    engine: Mutex<Option<Arc<dyn Engine>>>,
    /// How the daemon makes the engine. The tests install a fake instead.
    pub make_engine: Box<dyn Fn() -> Arc<dyn Engine> + Send + Sync>,
    /// True once the engine has been given its rate, group and input source.
    configured: AtomicBool,
    /// The group the preview shows, and what the engine needs to run it, as last published.
    latest: Mutex<Option<crate::scenes::GroupRun>>,
    /// The WebSocket subscribers of the preview engine (`?stack=1`).
    pub subs: PreviewRegistry,
    /// The stack the preview engine runs, and what it was built from.
    slot: Mutex<Option<PreviewSlot>>,
    /// The epoch order: which page owns the preview and which ones are older (D95.4.4).
    epochs: EpochBook,
    /// The builds running for this route and the one allowed to land.
    builds: Mutex<Builds>,
    /// The token the next build takes.
    tokens: AtomicU64,
    /// Orders the slot's transitions. The build itself runs outside it.
    gate: tokio::sync::Mutex<()>,
}

impl StackPreview {
    /// A preview with no engine yet; the daemon's real engine is made on first use.
    pub fn new() -> Self {
        Self::with_maker(Box::new(real_engine))
    }

    /// A preview that makes its engine with `make_engine`: the tests hand in a fake.
    pub fn with_maker(make_engine: Box<dyn Fn() -> Arc<dyn Engine> + Send + Sync>) -> Self {
        Self {
            engine: Mutex::new(None),
            make_engine,
            configured: AtomicBool::new(false),
            latest: Mutex::new(None),
            subs: PreviewRegistry::default(),
            slot: Mutex::new(None),
            epochs: EpochBook::default(),
            builds: Mutex::new(Builds::default()),
            tokens: AtomicU64::new(0),
            gate: tokio::sync::Mutex::new(()),
        }
    }

    /// Installs an engine a test built, instead of making one on first use.
    pub fn install_engine(&mut self, engine: Arc<dyn Engine>) {
        *self.engine.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(engine);
    }

    fn engine_guard(&self) -> std::sync::MutexGuard<'_, Option<Arc<dyn Engine>>> {
        self.engine.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The engine, made and configured on first use.
    ///
    /// It is given the active layout and the input source the lights' engine has *now*, not the
    /// ones the daemon started with: the engine is made when the builder first opens, which can
    /// be long after a layout change or the capture wiring (D95.4.4, item 6).
    pub async fn engine(&self, app: &App) -> Arc<dyn Engine> {
        let engine = {
            let mut guard = self.engine_guard();
            match guard.as_ref() {
                Some(engine) => Arc::clone(engine),
                None => {
                    let made = (self.make_engine)();
                    *guard = Some(Arc::clone(&made));
                    made
                }
            }
        };
        if self.configured.load(Ordering::SeqCst) {
            return engine;
        }
        // The flag is set last, after the engine has its rate, layout and source: a caller that
        // goes away between the read of the layout and the configuration leaves the flag false,
        // so the next open configures the engine instead of trusting a flag that is not true
        // yet. Two callers that overlap both configure it with the same values.
        let latest = self
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let run = match latest {
            Some(found) => found,
            None => crate::scenes::every_light_run(app).await,
        };
        engine.set_fps(canvas_proto::consts::PREVIEW_MAX_FPS);
        // The preview runs one group: the one being tuned, or one temporary group of every
        // light. It has no effect yet: the PUT that follows gives it one.
        let group = run.engine_group(None, &Props::new());
        engine.set_scene(run.lights.clone(), run.devices.clone(), vec![group], false);
        engine.set_input_source(app.input_source());
        self.configured.store(true, Ordering::SeqCst);
        engine
    }

    /// The group the preview shows, and what the engine needs to run it, as last published.
    pub fn group(&self) -> Option<crate::scenes::GroupRun> {
        self.latest.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// A group change reaches the preview engine when it exists. Before it does, the engine is
    /// given the last published group when it is made, so no change is dropped.
    pub fn set_group(&self, run: crate::scenes::GroupRun) {
        *self.latest.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(run);
    }

    /// An input-source change reaches the preview engine when it exists; otherwise the engine
    /// gets the app's current source when it is made.
    pub fn set_input_source(&self, source: Option<Arc<dyn InputSource>>) {
        if let Some(engine) = self.engine_guard().as_ref() {
            engine.set_input_source(source);
        }
    }

    /// The slot, copied, so a reader never holds the lock while it waits on anything.
    pub fn snapshot(&self) -> Option<PreviewSlot> {
        self.slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|slot| PreviewSlot {
                stack_id: slot.stack_id.clone(),
                group: slot.group.clone(),
                structure: slot.structure.clone(),
                live: Arc::clone(&slot.live),
                props: slot.props.clone(),
                layers: slot.layers.clone(),
                errors: slot.errors.clone(),
                statuses: Arc::clone(&slot.statuses),
            })
    }

    /// The epoch that owns the preview and its highest accepted sequence.
    pub fn watermark(&self) -> Option<(String, u64)> {
        self.epochs.live()
    }

    /// The newest epoch the daemon has seen, even after a DELETE cleared the live one.
    pub fn owner(&self) -> Option<String> {
        self.epochs.owner()
    }

    fn lock_builds(&self) -> std::sync::MutexGuard<'_, Builds> {
        self.builds.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A running build this request matches, copied. A structure already building is joined
    /// rather than built a second time.
    fn find_build(&self, matches: impl Fn(&InFlight) -> bool) -> Option<InFlight> {
        self.lock_builds().running.iter().find(|build| matches(build)).cloned()
    }

    /// Whether `token` is still the build allowed to land.
    fn is_current(&self, token: u64) -> bool {
        self.lock_builds().current == Some(token)
    }

    /// Makes a build the one allowed to land: a request for its structure again re-tokenises
    /// it, so Y -> X -> Y rejoins the build still making Y.
    fn adopt_build(&self, token: u64) {
        self.lock_builds().current = Some(token);
    }

    /// Starts a build for this structure and returns its token. Any build already running is
    /// superseded: its result will find another current token and land nowhere.
    fn begin_build(&self, stack: &Stack, structure: &str, props: Props) -> u64 {
        let token = self.tokens.fetch_add(1, Ordering::SeqCst) + 1;
        let mut builds = self.lock_builds();
        builds.running.push(InFlight {
            token,
            stack_id: stack.id.clone(),
            structure: structure.to_string(),
            props,
            live: Arc::new(std::sync::RwLock::new(LiveMap::from_stack(stack, 1))),
            errors: Arc::new(Mutex::new(BTreeMap::new())),
            statuses: Arc::new(Mutex::new(Vec::new())),
        });
        builds.current = Some(token);
        token
    }

    /// Gives a beat's values to the build `token`, if it is still running.
    fn update_in_flight(&self, token: u64, props: &Props) {
        if let Some(building) = self.lock_builds().running.iter_mut().find(|b| b.token == token) {
            building.props = props.clone();
        }
    }

    /// Takes the build `token` out of the book so it can land; `None` when a newer PUT, a
    /// DELETE or another structure superseded it. A superseded build is still removed, so a
    /// finished build never lingers for a later request to join.
    fn take_in_flight_if(&self, token: u64) -> Option<InFlight> {
        let mut builds = self.lock_builds();
        let at = builds.running.iter().position(|b| b.token == token)?;
        let building = builds.running.remove(at);
        if builds.current == Some(token) {
            builds.current = None;
            Some(building)
        } else {
            None
        }
    }

    /// Forgets a build that ended without landing (an error, a join error, a panic).
    fn forget_build(&self, token: u64) {
        let mut builds = self.lock_builds();
        builds.running.retain(|b| b.token != token);
        if builds.current == Some(token) {
            builds.current = None;
        }
    }

    /// A PUT with the structure already on screen supersedes any build for another one, but
    /// leaves the builds running: one can be rejoined if its structure comes back.
    fn supersede_build(&self) {
        self.lock_builds().current = None;
    }
}

/// Frees the route when a build ends without landing: an error, a join error, or a panic in the
/// blocking task. Every exit path after [`StackPreview::begin_build`] holds one, so a failed
/// build's token never stays in the book for later beats to join.
struct BuildLease<'a> {
    preview: &'a StackPreview,
    token: u64,
}

impl<'a> BuildLease<'a> {
    fn new(preview: &'a StackPreview, token: u64) -> Self {
        Self { preview, token }
    }
}

impl Drop for BuildLease<'_> {
    fn drop(&mut self) {
        self.preview.forget_build(self.token);
    }
}

impl Default for StackPreview {
    fn default() -> Self {
        Self::new()
    }
}

/// Why a preview PUT did not apply.
#[derive(Debug)]
pub enum PreviewError {
    /// Two previews already render: the answer is a 503 with `Retry-After`.
    Busy,
    /// The stack's name is another stack's, ignoring case: the answer is a 409, as Save's is.
    Conflict(String),
    /// The stack breaks a rule a Save enforces: the answer is a 422 with the reason.
    Validation(String),
    /// The daemon could not hand the effect over.
    Internal(String),
}

/// The structure identity of a stack (D95.4.6): layer ids, order, effect ids and each HTML
/// layer's source, through the hash the repository already uses.
pub fn structure_of(app: &App, stack: &Stack) -> String {
    let parts = crate::stack::structure(stack, &|id| {
        app.user_effects
            .source_and_file_name(id)
            .map(|(source, _)| source.to_string())
    });
    crate::stack::structure_hash(&parts)
}

/// The merged props a stack's layers start with (D95.4.3).
pub fn props_of(app: &App, stack: &Stack) -> Props {
    crate::stack::stack_props(stack, &|id| crate::api::effects::find_schema(&app.user_effects, &app.stacks, id))
}

/// `PUT /api/v1/stacks/preview` (D95.4.4).
///
/// The gate is taken to check the epoch and the slot, then the build runs on a blocking thread
/// outside it, and the gate is taken again to swap the slot. A PUT whose epoch is not the newest
/// page, or whose seq is not above the live one, lands nowhere and answers 200 with
/// `applied: false`.
///
/// `group` is the group the preview shows: its layers resolve in that group's canvas. With no
/// group, the preview shows one temporary flow-across group of every light.
///
/// A PUT whose structure is already building joins that build instead of starting another: a
/// build slower than the builder's beat is still the build that lands.
pub async fn put(
    app: &Arc<App>,
    stack: &Stack,
    epoch: &str,
    seq: u64,
    group: Option<&str>,
) -> Result<StackPreviewResponse, PreviewError> {
    // The group the preview shows. A group the active scene does not have is refused: the
    // builder would show a picture of nothing.
    let run = match group {
        Some(id) => crate::scenes::group_run(app, id).await.ok_or_else(|| {
            PreviewError::Validation(format!("the active scene has no group with the id {id}"))
        })?,
        None => crate::scenes::every_light_run(app).await,
    };
    app.stack_preview.set_group(run.clone());
    // The group's masks: the preview's layers resolve in that group's canvas.
    crate::scenes::publish_area_for(app, &run.id).await;
    // The render slot is taken only when a new build starts (below): a beat that only changes
    // values, or joins the build in flight, starts nothing and is never refused as busy.
    let slot;

    // The one check a Save also runs, on the canonical stack (D95.4.4, D95.4.5): a layer written
    // by its display name is the id it becomes, so the HTML cap counts it, the name rule leaves
    // out this stack by exact id as `Stacks::put` does, and the stack that is built and hashed is
    // the one a Save would store.
    let mut canonical = stack.clone();
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    crate::stacks::canonicalise_and_validate(&mut canonical, &app.stacks, &known).map_err(
        |refusal| match refusal {
            crate::stacks::StackRefusal::Conflict(message) => PreviewError::Conflict(message),
            crate::stacks::StackRefusal::Invalid(message) => PreviewError::Validation(message),
        },
    )?;
    let stack = &canonical;

    let structure = structure_of(app, stack);
    let props = props_of(app, stack);
    let engine = app.stack_preview.engine(app).await;

    let token;
    // The gate is held while the epoch, the slot and the build in flight are read and, when
    // nothing has to build, while the values are applied (D95.4.6).
    {
        let _gate = app.stack_preview.gate.lock().await;
        if !app.stack_preview.epochs.offer(epoch, seq) {
            return Ok(superseded(app, epoch, seq));
        }
        if let Some(slot) = app.stack_preview.snapshot() {
            if slot.stack_id == stack.id && slot.structure == structure && slot.group == run.id {
                crate::stack::replace_live(&slot.live, stack);
                // The structure on screen is the one the page wants; a build for another one
                // that is still running is superseded by this newer PUT.
                app.stack_preview.supersede_build();
                // The live map's lock is dropped before the props reach the engine.
                engine.set_group_props(&run.id, &props);
                let layers = reports(app, stack, &slot.errors, &slot.statuses, &run.id);
                {
                    let mut guard = app
                        .stack_preview
                        .slot
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Some(slot) = guard.as_mut() {
                        slot.props = props;
                        slot.layers = layers.clone();
                    }
                }
                tracing::info!(stack = %stack.id, seq, "the preview's values changed without a rebuild");
                return Ok(answer(app, true, epoch, seq, layers));
            }
        }
        // The same structure is already building: this beat joins it. The live map is the one
        // the building effect reads, so the values are there when it lands; its props are the
        // newest, and no second build starts. The build is made current again, so a structure
        // that comes back (Y -> X -> Y) lands rather than queuing a third build.
        if let Some(building) = app
            .stack_preview
            .find_build(|b| b.stack_id == stack.id && b.structure == structure)
        {
            crate::stack::replace_live(&building.live, stack);
            app.stack_preview.update_in_flight(building.token, &props);
            app.stack_preview.adopt_build(building.token);
            let errors = building
                .errors
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let layers = reports(app, stack, &errors, &building.statuses, &run.id);
            tracing::info!(stack = %stack.id, seq, "a beat joined the preview build in flight");
            return Ok(answer(app, true, epoch, seq, layers));
        }
        // A different structure, or the first open: this PUT's build owns the route.
        let Ok(acquired) = app.preview_slots.clone().try_acquire_owned() else {
            return Err(PreviewError::Busy);
        };
        slot = acquired;
        token = app.stack_preview.begin_build(stack, &structure, props.clone());
    }

    // Every exit path from here frees the route: an error, a join error, or a panic in the
    // blocking task must not leave a token for later beats to join. The lease stays in this
    // future rather than moving into the blocking task: the landing still needs the token in
    // the book, and this lease is what takes it back on an error, a join error, a panic, or a
    // client that goes away mid-build. The slot, on the other hand, is moved into the build,
    // so it is held for exactly as long as the build runs.
    let _lease = BuildLease::new(&app.stack_preview, token);
    // The newest request does not wait for a build it has already superseded: its build runs
    // beside any older one, and the older result is discarded when it lands. The engine is
    // written only under the gate, so no two builds hand it an effect at once.
    if !app.stack_preview.is_current(token) {
        return Ok(superseded(app, epoch, seq));
    }
    let Some(building) = app.stack_preview.find_build(|b| b.token == token) else {
        return Ok(superseded(app, epoch, seq));
    };

    // The build runs outside the gate, so DELETE and a newer PUT answer while it loads.
    let build_app = Arc::clone(app);
    let build_stack = stack.clone();
    let build_live = Arc::clone(&building.live);
    let build_statuses = Arc::clone(&building.statuses);
    let build_errors = Arc::clone(&building.errors);
    let build_group = run.id.clone();
    let fps = Arc::new(AtomicU32::new(canvas_proto::consts::PREVIEW_MAX_FPS));
    let effect = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        let leaf = crate::leaf_builder(&build_app.user_effects);
        let recording = {
            let errors = Arc::clone(&build_errors);
            Arc::new(move |id: &str| {
                let result = leaf(id);
                if let Err(error) = &result {
                    errors
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(id.to_string(), error.clone());
                }
                result
            })
        };
        Box::new(
            crate::stack::StackEffect::new(
                &build_stack,
                recording,
                build_live,
                build_app.stacks.areas(),
            )
            .with_clock(build_app.stacks.clock())
            .with_fps(fps)
            .with_group(&build_group)
            .with_status_sink(build_statuses),
        ) as Box<dyn Effect>
    })
    .await
    .map_err(|e| PreviewError::Internal(format!("the preview build task failed: {e}")))?;

    let _gate = app.stack_preview.gate.lock().await;
    // A newer PUT, a newer epoch, a DELETE or another structure while the build ran: this one
    // is stale.
    let Some(building) = app.stack_preview.take_in_flight_if(token) else {
        return Ok(superseded(app, epoch, seq));
    };
    // The preview engine runs the group being tuned, with the stack on it.
    let group = run.engine_group(Some(effect), &building.props);
    engine.set_scene(run.lights.clone(), run.devices.clone(), vec![group], false);

    let errors = building
        .errors
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let layers = reports(app, stack, &errors, &building.statuses, &run.id);
    let slot = PreviewSlot {
        stack_id: stack.id.clone(),
        group: run.id.clone(),
        structure,
        live: building.live,
        props: building.props,
        layers: layers.clone(),
        errors,
        statuses: building.statuses,
    };
    *app.stack_preview
        .slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(slot);
    // The build serves every beat that joined it, so the answer names the page that owns the
    // route now, which can be a newer one than the PUT that started the build.
    let (owned_epoch, owned_seq) = app
        .stack_preview
        .watermark()
        .unwrap_or_else(|| (epoch.to_string(), seq));
    tracing::info!(stack = %stack.id, seq = owned_seq, "the preview was built");
    Ok(answer(app, true, &owned_epoch, owned_seq, layers))
}

/// What `DELETE /stacks/preview` did.
pub struct ClearOutcome {
    /// False when the page that asked does not own the preview: nothing was dropped, and the
    /// page that owns it keeps previewing (D95.4.4, item 4).
    pub applied: bool,
    /// True when a preview was on screen and is dropped now.
    pub running: bool,
}

/// `DELETE /api/v1/stacks/preview`.
///
/// Drops the preview, clears the live sequence (the epoch order stays, so a stale tab cannot
/// re-take the preview) and answers 200. A build still running lands nowhere. The preview
/// engine, if it exists, paints black instead of keeping the last stack on the channel.
///
/// `epoch` is the page's session epoch when the caller sends one: a page that does not own the
/// preview drops nothing and is answered `applied: false`.
pub async fn clear(app: &Arc<App>, epoch: Option<&str>) -> ClearOutcome {
    let _gate = app.stack_preview.gate.lock().await;
    if let Some(epoch) = epoch {
        if !app.stack_preview.epochs.holds(epoch) {
            return ClearOutcome { applied: false, running: false };
        }
    }
    app.stack_preview.epochs.clear_live();
    app.stack_preview.supersede_build();
    let dropped = app
        .stack_preview
        .slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    let had = dropped.is_some();
    if had {
        if let (Some(engine), Some(slot)) = (app.stack_preview.engine_guard().as_ref(), dropped) {
            // The group the preview showed goes black: it has no effect any more.
            engine.set_group_effect(&slot.group, None, &crate::draft::black());
        }
        tracing::info!("the preview was dropped");
    }
    ClearOutcome { applied: true, running: had }
}

/// The answer a superseded PUT gives: 200, `applied: false`, and the epoch/seq that own the
/// preview so the page can stop resending the stale one.
fn superseded(app: &App, epoch: &str, seq: u64) -> StackPreviewResponse {
    let (owned_epoch, owned_seq) = app
        .stack_preview
        .watermark()
        .or_else(|| app.stack_preview.owner().map(|owner| (owner, 0)))
        .unwrap_or_else(|| (epoch.to_string(), seq));
    let layers = app
        .stack_preview
        .snapshot()
        .map(|slot| slot.layers)
        .unwrap_or_default();
    answer(app, false, &owned_epoch, owned_seq, layers)
}

/// The answer for a PUT that applied (or did not).
fn answer(
    app: &App,
    applied: bool,
    epoch: &str,
    seq: u64,
    layers: Vec<StackPreviewLayer>,
) -> StackPreviewResponse {
    StackPreviewResponse {
        applied,
        epoch: epoch.to_string(),
        seq,
        layers,
        running: app.stack_preview.snapshot().is_some(),
    }
}

/// What each layer is doing: nested, missing (with the build's reason), off the group, or the
/// runtime's own held/faulted report.
///
/// `group` is the group the preview shows: an area that matches nothing in it paints nothing,
/// and the layer says so.
fn reports(
    app: &App,
    stack: &Stack,
    errors: &BTreeMap<String, String>,
    statuses: &Arc<Mutex<Vec<StackLayerStatus>>>,
    group: &str,
) -> Vec<StackPreviewLayer> {
    let runtime = statuses
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let book = app.stacks.areas();
    let areas = read_recovering(&book);
    stack
        .layers
        .iter()
        .map(|layer| {
            let mask_error = layer.mask.as_ref().and_then(|mask| {
                errors.get(&mask.effect).map(|error| {
                    format!(
                        "the layer's mask effect {} is missing: {error}; the layer draws unmasked",
                        mask.effect
                    )
                })
            });
            let (status, error) = if layer.effect.starts_with(STACK_ID_PREFIX) {
                (
                    StackLayerState::NestedStack,
                    Some(format!(
                        "{} is a stack, and a stack cannot contain a stack",
                        layer.effect
                    )),
                )
            } else if let Some(error) = errors.get(&layer.effect) {
                (StackLayerState::MissingEffect, Some(error.clone()))
            } else if mask_error.is_some() {
                // The layer runs; its mask does not, so it draws unmasked (#256).
                (StackLayerState::MissingEffect, mask_error)
            } else if areas.is_empty_area(Some(group), &layer.area) {
                // The area matches no light of the group: the layer paints nothing, and says so.
                (StackLayerState::NoArea, Some("not in this group".into()))
            } else if let Some(reported) = runtime.iter().find(|status| status.id == layer.id) {
                (reported.status, None)
            } else {
                (StackLayerState::Ok, None)
            };
            StackPreviewLayer { id: layer.id.clone(), status, error }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_engine_runs_at_fifteen_fps() {
        let settings = engine_settings();
        assert_eq!(settings.fps, canvas_proto::consts::PREVIEW_MAX_FPS);
        assert_eq!(settings.fps, 15);
        assert_eq!(settings.brightness, 100);
        assert_eq!(settings.power, canvas_proto::api::Power::Running);
    }
}

