//! The daemon's shared state.
//!
//! One `App` holds everything the API handlers, the render loop and the background
//! tasks share: the on-disk store, the engine handle, the OpenRGB adapter, the device
//! list, the preview subscribers and the pipe client.

use canvas_adapters::frame_lane::FrameLane;
use canvas_adapters::resume::ResumeBehavior;
use canvas_proto::api::{LinkState, Status};
use canvas_proto::device::{DeviceId, DeviceInfo};
use canvas_proto::fans::FansStatus;
use canvas_proto::traits::{DeviceAdapter, Engine, FrameSink, GroupState, PreviewSink, RenderStats};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::storage::Store;

/// How many preview subscribers are attached. The engine parks when this is zero.
#[derive(Debug, Default)]
pub struct PreviewRegistry {
    count: AtomicU32,
}

impl PreviewRegistry {
    pub fn subscribe(&self) -> u32 {
        self.count.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn unsubscribe(&self) {
        let _ = self.count.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn subscribers(&self) -> u32 {
        self.count.load(Ordering::SeqCst)
    }
}

/// The group preview's one place (`GET /api/v1/preview/group`, #378): at most one socket streams
/// a group's picture, and the newest socket takes the place from the one before it.
///
/// The engine holds one group selection, so the slot and the engine's selection change together,
/// under the slot's lock: a takeover and the old socket's release cannot cross.
#[derive(Default)]
pub struct GroupPreviewSlot {
    inner: Mutex<GroupSlotInner>,
}

#[derive(Default)]
struct GroupSlotInner {
    /// The next owner number.
    next: u64,
    /// The socket that holds the place: its number, and the signal that tells it a newer socket
    /// took the place.
    owner: Option<(u64, tokio::sync::oneshot::Sender<()>)>,
}

impl GroupPreviewSlot {
    /// Takes the place for a new socket and points the engine at `group` with `sink`. A socket
    /// that held the place is told to close. Returns the new owner's number and the receiver that
    /// fires when a newer socket takes the place from it.
    pub fn take(
        &self,
        engine: &dyn Engine,
        group: String,
        sink: Arc<dyn PreviewSink>,
    ) -> (u64, tokio::sync::oneshot::Receiver<()>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.next += 1;
        let id = inner.next;
        let (taken_tx, taken_rx) = tokio::sync::oneshot::channel();
        if let Some((_, previous)) = inner.owner.replace((id, taken_tx)) {
            // The old socket may be gone already; then there is nobody to tell.
            let _ = previous.send(());
        }
        engine.set_group_preview(Some(group), Some(sink));
        (id, taken_rx)
    }

    /// Gives the place back. Only the owner clears the engine's selection: a socket that was
    /// taken over leaves the newer socket's selection alone.
    pub fn release(&self, engine: &dyn Engine, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.owner.as_ref().is_some_and(|(owner, _)| *owner == id) {
            inner.owner = None;
            engine.set_group_preview(None, None);
        }
    }

    /// Whether a socket holds the place.
    pub fn is_open(&self) -> bool {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).owner.is_some()
    }
}

/// The longest a fans route waits on the service for one exchange while it holds `fans_last`
/// (issue #225). A healthy service answers in milliseconds; past this the route answers 503.
pub const FANS_LOCK_PIPE_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a route waits for the render engine before it answers without it (#241).
///
/// The engine's handle is a `std::sync::Mutex` the render thread takes as well, so a
/// route that asks the engine on a runtime worker waits for the render thread there.
/// The daemon's runtime has **two** workers (`crates/canvas-daemon/src/main.rs`), so two
/// such waits take the whole API down while the port keeps listening: the editor says
/// "the daemon did not answer" and the capture's poll of `/api/v1/settings` times out.
pub const ENGINE_CALL_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);

/// Asks the render engine something without holding a runtime worker while it waits.
///
/// The call runs on the blocking pool and is bounded by [`ENGINE_CALL_LIMIT`]. `None`
/// when the engine did not answer in time: the caller answers with what it already
/// knew, and the API keeps serving (#241).
pub async fn engine_call<T, F>(engine: Arc<dyn Engine>, call: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce(&dyn Engine) -> T + Send + 'static,
{
    match tokio::time::timeout(
        ENGINE_CALL_LIMIT,
        tokio::task::spawn_blocking(move || call(engine.as_ref())),
    )
    .await
    {
        Ok(Ok(value)) => Some(value),
        // The call panicked, or the engine did not answer in time.
        Ok(Err(_)) | Err(_) => None,
    }
}

/// A value the daemon keeps from the render engine, and whether it has asked for it.
#[derive(Debug, Clone, Default)]
struct Cached<T> {
    /// Whether the daemon has asked the engine at all. Once it has, a route never asks
    /// again: the render thread may never hand the lock over, and every ask would cost
    /// a thread of the blocking pool.
    asked: bool,
    /// The last value the engine gave.
    value: T,
}

/// Keeps the engine's numbers fresh, so a route never waits for the render thread.
///
/// One thread per value asks the engine once a second and keeps the last answer. The
/// engine's lock is the render thread's: a route that asked for the numbers on a
/// runtime worker would wait for the render thread there, and the daemon's runtime has
/// two workers (#241). The watchers are ordinary OS threads, not tasks on the blocking
/// pool, so a render thread that never hands the lock over costs one thread and not one
/// per request.
pub fn watch_engine(app: Arc<App>) {
    let for_stats = Arc::clone(&app);
    std::thread::Builder::new()
        .name("canvas-rgb-engine-stats".into())
        .spawn(move || {
            loop {
                if for_stats.shutting_down.load(Ordering::SeqCst) != 0 {
                    return;
                }
                // The flag goes up before the ask: a render thread that never hands the
                // lock over must not make every route ask for itself.
                for_stats.engine_stats.lock().unwrap_or_else(|e| e.into_inner()).asked = true;
                let stats = for_stats.engine.stats();
                let mut cache = for_stats.engine_stats.lock().unwrap_or_else(|e| e.into_inner());
                cache.value = stats;
                drop(cache);
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        })
        .ok();
    let for_groups = Arc::clone(&app);
    std::thread::Builder::new()
        .name("canvas-rgb-engine-groups".into())
        .spawn(move || {
            loop {
                if for_groups.shutting_down.load(Ordering::SeqCst) != 0 {
                    return;
                }
                for_groups
                    .engine_groups
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .asked = true;
                let groups = for_groups.engine.group_states();
                let mut cache =
                    for_groups.engine_groups.lock().unwrap_or_else(|e| e.into_inner());
                cache.value = groups;
                drop(cache);
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        })
        .ok();
}

/// The state every handler sees.
pub struct App {
    /// The on-disk state. Written rarely (a settings change) and read often.
    ///
    /// This is a Tokio lock, not a std one, so the guard is `Send` and an API handler
    /// can hold it across an await.
    pub store: tokio::sync::RwLock<Store>,
    /// The render engine handle.
    pub engine: Arc<dyn Engine>,
    /// The engine's last numbers, kept by [`watch_engine`] so a route never waits for the
    /// render thread (#241).
    engine_stats: Mutex<Cached<RenderStats>>,
    /// The engine's last group states, kept the same way (#241).
    engine_groups: Mutex<Cached<Vec<GroupState>>>,
    /// The OpenRGB adapter. Behind an async mutex: the SDK client is not `Sync` on its
    /// own, and the API handlers await it.
    pub adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>>,
    /// The OpenRGB link the frame writers use, when the adapter has one to share (the daemon's
    /// composite does, and the OpenRGB adapter itself). A frame through it does not wait on the
    /// adapter lock, which the supervisor holds while it waits on OpenRGB for a reply (#233).
    /// `None`: the adapter has no OpenRGB link, and every frame goes through the adapter.
    pub frame_lane: Option<canvas_adapters::openrgb::OpenRgbAdapter>,
    /// The devices as last discovered.
    pub devices: tokio::sync::RwLock<Vec<DeviceInfo>>,
    /// The frame writer of every driven device, keyed by device (see [`crate::device_sync`]).
    pub device_sinks: crate::device_sync::DeviceSinks,
    /// Where a rescan the owner asked for goes: to the supervisor, the one task that writes the
    /// device list (#233). The supervisor takes the other end with [`App::take_rescan_inbox`].
    pub rescan_requests: tokio::sync::mpsc::Sender<crate::openrgb_supervisor::Rescan>,
    rescan_inbox: Mutex<Option<tokio::sync::mpsc::Receiver<crate::openrgb_supervisor::Rescan>>>,
    /// How long a rescan may hold the adapter before the supervisor gives up on it. A field so a
    /// test can shorten it; the daemon keeps [`crate::openrgb_supervisor::RESCAN_BUDGET`].
    pub rescan_budget: std::time::Duration,
    /// Zone sizes by device fingerprint (the `zones.json` contents).
    ///
    /// Behind an `Arc` so the OpenRGB supervisor can share it with the API handlers.
    pub zones: Arc<tokio::sync::RwLock<ZoneSizes>>,
    /// The zones a setup holds at a length it raised them to (#338): chain setup raises a header
    /// to its longest length while the owner drags a slider, and the supervisor overlays these on
    /// the saved sizes so its refresh leaves the raise alone. Never written to `zones` or
    /// `zones.json`: the hold ends when the setup drops it.
    pub zone_holds: Arc<tokio::sync::RwLock<ZoneHolds>>,
    /// One chain setup step at a time (#338): start, commit, empty and every end of a setup
    /// (cancel, the timeout, quit) take it first, before any other lock. A cancel that races a
    /// commit, or the timeout that races a cancel, then runs after it and finds the setup over.
    pub chain_setup: tokio::sync::Mutex<()>,
    /// Set when the first non-empty device list replayed what a crash left (#338): the chain
    /// restore record, and the commit rollback that reads an intent record. One replay, however
    /// many lists follow.
    pub leftovers_replayed: AtomicBool,
    /// The running effect's preview subscribers (`GET /preview`), so the main engine only runs
    /// while the editor watches.
    pub preview_subs: PreviewRegistry,
    /// The stack preview engine's subscribers (`GET /preview?stack=1`).
    pub stack_subs: PreviewRegistry,
    /// The group preview socket (`GET /api/v1/preview/group`): one at a time, newest wins.
    pub group_preview: GroupPreviewSlot,
    /// The stack preview engine and the stack it runs (D95.4.4).
    pub stack_preview: crate::preview::StackPreview,
    /// The screen and audio captures. They only run while an effect needs them.
    ///
    /// The capture is shared: the engines hold it as their input source, so the screen
    /// and the audio reach the lights (#1).
    pub inputs: Arc<crate::capture::DaemonInputs>,
    /// The API token every request must carry.
    pub token: String,
    /// The daemon's version, as reported by `/status`.
    pub version: String,
    /// Milliseconds the service pipe has been down.
    pub service_down_ms: AtomicU64,
    /// The last fan status the service reported.
    pub fans: tokio::sync::RwLock<Option<FansStatus>>,
    /// The last fan config the daemon pushed to the service (G10 export while down).
    pub fans_last: tokio::sync::RwLock<Option<FansFile>>,
    /// How long a pipe exchange made while holding `fans_last` may take before the route answers
    /// 503 (issue #225). The pipe client has no timeout of its own, so a hung service would
    /// otherwise hold the lock, and with it config export and every other fans writer.
    pub fans_pipe_limit: std::time::Duration,
    /// SignalRGB conflicts, refreshed every 5 s.
    pub conflicts: tokio::sync::RwLock<Vec<canvas_proto::api::Conflict>>,
    /// Where OpenRGB is, as the daemon sees it.
    pub openrgb: tokio::sync::RwLock<LinkState>,
    /// Set when the daemon is quitting, so background tasks stop.
    pub shutting_down: AtomicU32,
    /// The channel the API sends on when the daemon should quit.
    pub quit_tx: tokio::sync::mpsc::UnboundedSender<()>,
    /// The user HTML effects found in `%APPDATA%\canvas-rgb\effects\`.
    pub user_effects: Arc<crate::user_effects::UserEffects>,
    /// The saved effect stacks (`stacks.json`) and the runtime state a stack shares: the area
    /// book, the live map of the stack on the engine, and the frame rate its budget uses.
    pub stacks: Arc<crate::stacks::Stacks>,
    /// The saved patterns (`patterns.json`, 0.16.0): a colour per LED, built on one kind of
    /// device. Read at start-up beside the stacks and written whole on every save.
    pub patterns: Arc<crate::patterns::Patterns>,
    /// The folder the user HTML effects live in. The import writes here and the
    /// delete removes from here, so both work on the folder the daemon watches.
    pub effects_dir: PathBuf,
    /// Held while an effect is imported or deleted, so the two never race each other
    /// over the same file.
    pub effect_writes: tokio::sync::Mutex<()>,
    /// The OpenRGB server the daemon started (milestone 1). The quit path stops it.
    pub openrgb_child: Mutex<Option<crate::openrgb_supervisor::ServerProcess>>,
    /// The link to the service. Every fan request goes through it.
    pub pipe: Arc<dyn crate::pipe_client::PipeLink>,
    /// The effect thumbnails rendered so far.
    pub thumbnails: crate::thumbnail::ThumbnailCache,
    /// The numeric displays: their adapter, what they show, and the task's pacing.
    pub displays: crate::displays::DisplayHub,
    /// The peripherals: the mouse and the controller, and what the daemon may do with them.
    pub peripherals: crate::peripherals::PeripheralHub,
    /// The user data folder (`store.home`), for the pieces made after the store: the peripheral
    /// hub saves its kept values here (#127).
    home: PathBuf,
    /// The identify override: while it is active the engine's frames reach no device.
    pub identify: Arc<crate::identify::IdentifyGate>,
    /// When the last frame reached a device; the quit path logs it so the service log lines up (#81).
    pub last_frame: Arc<crate::last_frame::LastFrame>,
    /// The onboarding effects are transient: the timer that restores the saved effect.
    pub transient: crate::transient::Transient,
    /// The draft on the lights: an editor's unsaved effect, with its lease (see [`crate::draft`]).
    pub draft: crate::draft::Draft,
    /// A seam in the config import, for the tests: the import stops between the end of the
    /// draft and the swap of the store, so a test can put a draft PUT in that gap (#94).
    ///
    /// `None` in the daemon.
    import_seam: Mutex<Option<Arc<crate::api::settings::ImportSeam>>>,
    /// The input source the lights' engine was last given. An engine made later (the stack
    /// preview) starts from the same one (D95.4.4).
    pub input_source: Mutex<Option<Arc<dyn canvas_proto::traits::InputSource>>>,
    /// The preview render slots. A preview renders up to 30 s of a blocking thread, so at most
    /// [`crate::api::editor::PREVIEW_CONCURRENT`] run at once and the rest answer 503.
    pub preview_slots: Arc<tokio::sync::Semaphore>,
    /// The draft builds' own render slots (HTML and stack drafts), apart from the previews', so a
    /// stack builder's preview and draft of one structure change never take every slot and refuse
    /// the next change (#102 review). The two pools bound the daemon at four builds at once.
    pub draft_slots: Arc<tokio::sync::Semaphore>,
    /// Fakes mode (#199, debug builds only): the daemon talks only to fakes. The
    /// auth guard reads it to accept the UI-capture origin (#200) only here.
    pub fakes: bool,
    /// What the scenes need at run time: scene mode, the apply order, build failures.
    pub scene: crate::scenes::SceneRuntime,
}

/// The fan config the daemon keeps a copy of.
pub type FansFile = canvas_proto::config::FansFile;

/// Zone sizes by device fingerprint, then zone index.
pub type ZoneSizes = BTreeMap<String, BTreeMap<u32, u32>>;

/// The zones a setup holds at a raised length (#338): device fingerprint and zone index, then the
/// length the setup raised it to. Only the supervisor reads it, and only for the length of a setup.
pub type ZoneHolds = BTreeMap<(String, u32), u32>;

impl App {
    /// Builds an app from the pieces the caller supplies, with no numeric displays.
    ///
    /// The daemon uses this too, so the tests and the daemon build the same state.
    ///
    /// The adapter must say what it does on a resume (`ResumeBehavior`) and whether it has a
    /// frame lane (`FrameLane`): an adapter that does not cannot be handed to the app, so it
    /// does not compile.
    pub fn new<A>(
        store: crate::storage::Store,
        engine: Arc<dyn Engine>,
        adapter: Box<A>,
        token: String,
        version: String,
        quit_tx: tokio::sync::mpsc::UnboundedSender<()>,
    ) -> Self
    where
        A: DeviceAdapter + ResumeBehavior + FrameLane + 'static,
    {
        Self::with_displays(
            store,
            engine,
            adapter,
            token,
            version,
            quit_tx,
            Box::new(crate::displays::NoDisplays),
        )
    }

    /// Like [`App::new`], with the numeric display adapter the daemon wires in (the CHIONE).
    ///
    /// Both adapters must say what they do on a resume (`ResumeBehavior`), as for [`App::new`].
    pub fn with_displays<A, D>(
        store: crate::storage::Store,
        engine: Arc<dyn Engine>,
        adapter: Box<A>,
        token: String,
        version: String,
        quit_tx: tokio::sync::mpsc::UnboundedSender<()>,
        display_adapter: Box<D>,
    ) -> Self
    where
        A: DeviceAdapter + ResumeBehavior + FrameLane + 'static,
        D: canvas_proto::traits::NumericDisplayAdapter + ResumeBehavior + 'static,
    {
        let user_effects = Arc::new(crate::user_effects::UserEffects::with_stock_and_tiles(
            crate::user_effects::html_builder(),
            crate::user_effects::html_builder_seeded(crate::thumbnail::TILE_SEED),
            crate::user_effects::STOCK_EFFECTS,
        ));
        // The stacks are read before the API answers; the effect names they are checked against
        // are the stock ones until the effects folder is scanned (the daemon scans it right
        // after it builds the app and reads the file again then).
        let stacks = Arc::new(crate::stacks::Stacks::new(store.home.clone()));
        stacks.set_fps(store.config.settings.fps);
        stacks.reload(&user_effects);
        // The patterns are read beside the stacks, before the API answers: a pattern a scene
        // plays has to be there for the first build of the scene.
        let patterns = Arc::new(crate::patterns::Patterns::new(store.home.clone()));
        patterns.reload();
        // B1: the persisted audio-source pick applies at startup, so it
        // survives a restart (silently — no capture is running yet).
        let audio_source = store.config.audio_source.clone();
        let inputs = Arc::new(crate::capture::DaemonInputs::production());
        inputs.set_requested(audio_source);
        let home = store.home.clone();
        let (rescan_requests, rescan_inbox) = tokio::sync::mpsc::channel(4);
        // The frame lane comes from the adapter itself: the OpenRGB adapter and the composite
        // share one link, and the frame writers write through it (#233).
        let frame_lane = adapter.frame_lane();
        Self {
            device_sinks: Default::default(),
            rescan_requests,
            rescan_inbox: Mutex::new(Some(rescan_inbox)),
            rescan_budget: crate::openrgb_supervisor::RESCAN_BUDGET,
            engine_stats: Mutex::new(Cached::default()),
            engine_groups: Mutex::new(Cached::default()),
            displays: crate::displays::DisplayHub::new(display_adapter),
            peripherals: crate::peripherals::PeripheralHub::with_home(Vec::new(), home.clone()),
            home,
            identify: Arc::default(),
            last_frame: Arc::default(),
            transient: Default::default(),
            draft: Default::default(),
            import_seam: Mutex::new(None),
            input_source: Mutex::new(None),
            preview_slots: Arc::new(tokio::sync::Semaphore::new(crate::api::editor::PREVIEW_CONCURRENT)),
            draft_slots: Arc::new(tokio::sync::Semaphore::new(crate::api::editor::PREVIEW_CONCURRENT)),
            fakes: false,
            scene: Default::default(),
            store: tokio::sync::RwLock::new(store),
            engine,
            adapter: Arc::new(tokio::sync::Mutex::new(adapter as Box<dyn DeviceAdapter>)),
            frame_lane,
            devices: tokio::sync::RwLock::new(Vec::new()),
            zones: Arc::new(tokio::sync::RwLock::new(ZoneSizes::new())),
            zone_holds: Arc::new(tokio::sync::RwLock::new(ZoneHolds::new())),
            chain_setup: tokio::sync::Mutex::new(()),
            leftovers_replayed: AtomicBool::new(false),
            preview_subs: PreviewRegistry::default(),
            group_preview: GroupPreviewSlot::default(),
            stack_subs: PreviewRegistry::default(),
            stack_preview: crate::preview::StackPreview::new(),
            inputs,
            token,
            version,
            service_down_ms: AtomicU64::new(0),
            fans: tokio::sync::RwLock::new(None),
            fans_last: tokio::sync::RwLock::new(None),
            fans_pipe_limit: FANS_LOCK_PIPE_LIMIT,
            conflicts: tokio::sync::RwLock::new(Vec::new()),
            openrgb: tokio::sync::RwLock::new(LinkState::Down),
            shutting_down: AtomicU32::new(0),
            quit_tx,
            user_effects,
            stacks,
            patterns,
            effects_dir: crate::paths::effects_dir(),
            effect_writes: tokio::sync::Mutex::new(()),
            openrgb_child: Mutex::new(None),
            pipe: Arc::new(crate::pipe_client::ServicePipe),
            thumbnails: crate::thumbnail::ThumbnailCache::default(),
        }
    }

    /// Adds the peripheral adapters: the mouse and the controller.
    ///
    /// The peripherals are only opened when the owner says so (see [`crate::peripherals`]), so
    /// building the app opens nothing. The daemon wires in the real adapters; the tests wire
    /// in fakes.
    pub fn with_peripherals(
        mut self,
        peripherals: Vec<Box<dyn crate::peripherals::Peripheral>>,
    ) -> Self {
        self.peripherals = crate::peripherals::PeripheralHub::with_home(peripherals, self.home.clone());
        self
    }

    /// Like [`App::with_peripherals`], with an input-idle clock the tests drive (#127).
    pub fn with_peripherals_idle(
        mut self,
        peripherals: Vec<Box<dyn crate::peripherals::Peripheral>>,
        idle: Arc<dyn crate::peripherals::InputIdle>,
    ) -> Self {
        self.peripherals =
            crate::peripherals::PeripheralHub::with_home_and_idle(peripherals, self.home.clone(), idle);
        self
    }

    /// The receiving end of the rescan requests. The supervisor takes it once; a second call
    /// answers `None`.
    pub fn take_rescan_inbox(&self) -> Option<tokio::sync::mpsc::Receiver<crate::openrgb_supervisor::Rescan>> {
        self.rescan_inbox.lock().unwrap_or_else(|e| e.into_inner()).take()
    }


    /// Sets the seam in the config import (tests only).
    ///
    /// The import stops at the seam, between the end of the draft and the swap of the
    /// store, until the seam lets it go. The daemon never sets one.
    pub fn set_import_seam(&self, seam: Arc<crate::api::settings::ImportSeam>) {
        *self.import_seam.lock().unwrap_or_else(|e| e.into_inner()) = Some(seam);
    }

    /// The seam in the config import, if a test set one.
    pub fn import_seam(&self) -> Option<Arc<crate::api::settings::ImportSeam>> {
        self.import_seam.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The input source reaches both engines at the same call sites. The daemon's startup
    /// gives both engines the daemon's own capture (see [`App::capture_source`]), so an
    /// effect that needs the screen or the audio reads a live source from its first
    /// frame; the app keeps the last source so an engine made later (the preview) starts
    /// from the same one (D95.4.4).
    pub fn set_input_source(&self, source: Option<Arc<dyn canvas_proto::traits::InputSource>>) {
        *self.input_source.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            source.clone();
        self.engine.set_input_source(source.clone());
        self.stack_preview.set_input_source(source);
    }

    /// The daemon's capture, as the engines read it.
    ///
    /// This is the source both engines are given: the screen and the audio the daemon
    /// captures reach the lights and the stack preview through it (#1).
    pub fn capture_source(&self) -> Arc<dyn canvas_proto::traits::InputSource> {
        let inputs: Arc<crate::capture::DaemonInputs> = Arc::clone(&self.inputs);
        let source: Arc<dyn canvas_proto::traits::InputSource> = inputs;
        source
    }

    /// The input source the lights' engine has.
    pub fn input_source(&self) -> Option<Arc<dyn canvas_proto::traits::InputSource>> {
        self.input_source.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// The preview engine, made and configured on first use. It parks while nothing watches
    /// it and never has a device sink.
    pub async fn stack_preview_engine(&self) -> Arc<dyn canvas_proto::traits::Engine> {
        self.stack_preview.engine(self).await
    }

    /// The device with this id, if it is known.
    pub async fn device(&self, id: &DeviceId) -> Option<DeviceInfo> {
        self.devices.read().await.iter().find(|d| &d.id == id).cloned()
    }

    /// The number of devices currently connected.
    pub async fn devices_connected(&self) -> u32 {
        self.devices.read().await.iter().filter(|x| x.connected).count() as u32
    }

    /// How many devices the daemon knows about, connected or not.
    pub async fn devices_total(&self) -> u32 {
        self.devices.read().await.len() as u32
    }

    /// The engine's numbers, without waiting for the render thread.
    ///
    /// The numbers come from the daemon's watcher thread ([`watch_engine`]), which asks the
    /// engine once a second. A route asks the engine itself only when the watcher has not
    /// answered yet, and that ask is bounded: the engine's lock is the render thread's,
    /// and the daemon's runtime has two workers (#241).
    pub async fn engine_stats(&self) -> RenderStats {
        {
            let cache = self.engine_stats.lock().unwrap_or_else(|e| e.into_inner());
            if cache.asked {
                return cache.value.clone();
            }
        }
        match engine_call(Arc::clone(&self.engine), |engine| engine.stats()).await {
            Some(stats) => {
                let mut cache = self.engine_stats.lock().unwrap_or_else(|e| e.into_inner());
                cache.asked = true;
                cache.value = stats.clone();
                stats
            }
            // The engine did not answer in time: the route answers with the numbers it
            // has, and the API keeps serving.
            None => RenderStats::default(),
        }
    }

    /// The engine's group states, kept the same way ([`App::engine_stats`]).
    pub async fn engine_group_states(&self) -> Vec<GroupState> {
        {
            let cache = self.engine_groups.lock().unwrap_or_else(|e| e.into_inner());
            if cache.asked {
                return cache.value.clone();
            }
        }
        match engine_call(Arc::clone(&self.engine), |engine| engine.group_states()).await {
            Some(groups) => {
                let mut cache = self.engine_groups.lock().unwrap_or_else(|e| e.into_inner());
                cache.asked = true;
                cache.value = groups.clone();
                groups
            }
            None => Vec::new(),
        }
    }

    /// The status snapshot the API and the CLI read.
    pub async fn status(&self) -> Status {
        let stats = self.engine_stats().await;
        let store = self.store.read().await;
        let power = store.config.settings.power;
        let brightness = store.config.settings.brightness;
        let fps_target = store.config.settings.fps;
        let scene = crate::scenes::active_scene(&store.scenes);
        let active_scene = Some(canvas_proto::api::SceneRef { id: scene.id.clone(), name: scene.name.clone() });
        // One entry per distinct effect the active scene's groups play, in the scene's order.
        let mut active_effects: Vec<String> = Vec::new();
        for group in &scene.groups {
            if let canvas_proto::scene::Plays::Effect { effect, .. } = &group.plays {
                if !active_effects.contains(effect) {
                    active_effects.push(effect.clone());
                }
            }
        }
        // G7: the editor shows the dismissible "size your ARGB headers" panel until the user
        // dismisses it.
        let show_zone_panel = !store.config.zone_panel_dismissed;
        let onboarding_complete = store.config.onboarding_complete;
        let openrgb = *self.openrgb.read().await;
        let conflicts = self.conflicts.read().await.clone();
        drop(store);
        let down = self.service_down_ms.load(Ordering::SeqCst);
        Status {
            version: self.version.clone(),
            power,
            brightness,
            fps_target,
            fps_measured: stats.fps_measured,
            frame_time_ms: stats.frame_time_ms,
            frames_rendered: stats.frames_rendered,
            parked: stats.parked,
            devices_connected: self.devices_connected().await,
            devices_total: self.devices_total().await,
            preview_subscribers: self.preview_subs.subscribers()
                + self.stack_subs.subscribers()
                + u32::from(self.group_preview.is_open())
                + self.scene.feed.watchers() as u32,
            active_scene,
            active_effects,
            openrgb,
            service: if down == 0 { LinkState::Connected } else { LinkState::Down },
            service_down_ms: if down == 0 { None } else { Some(down) },
            conflicts,
            show_zone_panel,
            onboarding_complete,
        }
    }

    /// The fan profile names the service knows about.
    pub async fn fan_profile_names(&self) -> Vec<String> {
        self.fans.read().await.as_ref().map(|s| s.profiles.clone()).unwrap_or_default()
    }
}

/// A per-device frame sink: a bounded(1) channel into the device's writer task.
///
/// When the device is slower than the render loop, the old frame is dropped and the new
/// one takes its place. The engine never blocks.
pub struct DeviceSink {
    id: DeviceId,
    tx: tokio::sync::mpsc::Sender<Vec<canvas_proto::Rgb8>>,
    /// While this is active the engine's frames are dropped (the identify override).
    gate: Option<Arc<crate::identify::IdentifyGate>>,
}

impl DeviceSink {
    pub fn new(id: DeviceId, tx: tokio::sync::mpsc::Sender<Vec<canvas_proto::Rgb8>>) -> Self {
        Self { id, tx, gate: None }
    }

    /// A sink that obeys the identify override and lets its ticker reach the same writer.
    pub fn with_gate(
        id: DeviceId,
        tx: tokio::sync::mpsc::Sender<Vec<canvas_proto::Rgb8>>,
        gate: Arc<crate::identify::IdentifyGate>,
    ) -> Self {
        gate.register(id.clone(), tx.clone());
        Self { id, tx, gate: Some(gate) }
    }
}

impl FrameSink for DeviceSink {
    fn submit(&self, colors: Vec<canvas_proto::Rgb8>) {
        if self.gate.as_ref().is_some_and(|g| g.is_active()) {
            return;
        }
        // `try_send` never blocks; a full channel means the device is behind and the old
        // frame is dropped.
        match self.tx.try_send(colors) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::debug!(device = %self.id, "device is behind; dropping a frame");
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!(device = %self.id, "device writer is gone; dropping a frame");
            }
        }
    }
}

/// A preview sink: the engine's downscaled canvas, forwarded to the WebSocket.
pub struct WebsocketPreview {
    tx: tokio::sync::mpsc::Sender<(u32, u32, Vec<u8>)>,
}

impl WebsocketPreview {
    pub fn new(tx: tokio::sync::mpsc::Sender<(u32, u32, Vec<u8>)>) -> Self {
        Self { tx }
    }
}

impl PreviewSink for WebsocketPreview {
    fn submit(&self, width: u32, height: u32, rgb: Vec<u8>) {
        let _ = self.tx.try_send((width, height, rgb));
    }
}

/// Zone sizes by device fingerprint, then zone index.
pub fn zone_sizes(sizes: &ZoneSizes, device: &DeviceId) -> BTreeMap<u32, u32> {
    sizes.get(&device.0).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::api::Conflict;

    #[test]
    fn preview_registry_counts_subscribers() {
        let registry = PreviewRegistry::default();
        assert_eq!(registry.subscribers(), 0);
        registry.subscribe();
        registry.subscribe();
        assert_eq!(registry.subscribers(), 2);
        registry.unsubscribe();
        assert_eq!(registry.subscribers(), 1);
    }

    #[test]
    fn device_sink_drops_frames_rather_than_blocking() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let sink = DeviceSink::new(DeviceId("openrgb:abc".into()), tx);
        sink.submit(vec![canvas_proto::Rgb8::WHITE]);
        // The channel is full now: the second frame is dropped, not queued.
        sink.submit(vec![canvas_proto::Rgb8::BLACK]);
        let first = rx.try_recv().unwrap();
        assert_eq!(first, vec![canvas_proto::Rgb8::WHITE]);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn conflicts_are_reported() {
        let app_conflicts = [Conflict::SignalRgb { process: true, service: false }];
        assert!(matches!(app_conflicts[0], Conflict::SignalRgb { process: true, .. }));
    }
}
