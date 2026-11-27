//! The daemon's shared state.
//!
//! One `App` holds everything the API handlers, the render loop and the background
//! tasks share: the on-disk store, the engine handle, the OpenRGB adapter, the device
//! list, the preview subscribers and the pipe client.

use canvas_adapters::frame_lane::FrameLane;
use canvas_adapters::resume::ResumeBehavior;
use canvas_proto::api::{LastExit, LinkState, SafeStart, SafeStartMode, Status};
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
///
/// The bound runs from the start of the call into the engine, not from the moment a
/// request joins it: a request that joins a call that is already in flight waits for
/// what is left of it, and never longer.
pub const ENGINE_CALL_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);

/// What a route says when the render engine has never answered (#241).
///
/// The daemon does not make a number up: zeros look like a measurement, and a group
/// list that is empty looks like a group that is fine.
pub const ENGINE_NOT_ANSWERING: &str = "the render engine is not answering";

/// How old the daemon's copy of the engine's numbers may be before a route asks the
/// engine itself (#241).
///
/// The daemon's watcher threads keep the copy fresh. A watcher that stops (a quit that
/// was cancelled, a render thread that never hands the lock over) leaves the copy to
/// age, and a route that reads an aged copy asks the engine: the answer may be late,
/// but it is the engine's own.
pub const ENGINE_CACHE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(3);

/// How often the daemon's watcher threads ask the engine (#241).
///
/// Four times a second: a change on the engine shows in the API within a second, and
/// the engine's numbers are a lock and a few numbers, so the cost is small.
pub const ENGINE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// What one call into the engine is doing.
#[derive(Debug, Clone)]
enum FlightState<T> {
    /// The call is still running.
    Running,
    /// The engine answered.
    Answered(T),
    /// The call gave up: it panicked, so there is no answer.
    GaveUp,
}

/// One call into the engine, and the answer it will give.
#[derive(Clone)]
struct Flight<T> {
    /// The flight's number. It tells a flight from the one that took its place.
    id: u64,
    /// When the flight started. The bound runs from here, so a request that joins a
    /// flight late waits for less of it.
    started: std::time::Instant,
    /// The answer, and the ones waiting for it.
    state: tokio::sync::watch::Receiver<FlightState<T>>,
    /// The sender the answer goes through.
    sender: tokio::sync::watch::Sender<FlightState<T>>,
}

/// The one call into the engine of one kind that may be in flight at a time (#241).
///
/// The engine's handle is a `std::sync::Mutex` the render thread takes as well, so a
/// call into the engine holds a thread for as long as the engine takes to answer. The
/// timeout around the call gives the route its answer back, but it does not take the
/// thread back: the call stays inside the engine, and ten status requests left ten
/// threads inside the engine.
///
/// One flight at a time per kind is the bound. A request that arrives while a call is
/// outstanding joins that call instead of starting one of its own, and waits for its
/// answer. The number of threads inside the engine is one per kind, however many
/// requests arrive.
///
/// The call runs outside every runtime: on the watcher's own thread when the watcher
/// asks, and on a thread of its own when a route asks. A runtime that goes away does
/// not take the call with it, and the daemon's exit does not wait for it.
pub struct EngineGate<T> {
    inner: Mutex<GateInner<T>>,
}

struct GateInner<T> {
    /// The next flight's number.
    next: u64,
    /// The flight that is in flight, if there is one.
    flight: Option<Flight<T>>,
}

impl<T: Clone + Send + Sync + 'static> EngineGate<T> {
    /// A gate with no flight in it.
    fn new() -> Self {
        Self { inner: Mutex::new(GateInner { next: 0, flight: None }) }
    }

    /// The flight in flight, or a new one to make.
    fn take_flight(&self) -> (Flight<T>, bool) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match inner.flight.clone() {
            // A call is already in flight: this one joins it.
            Some(flight) => (flight, false),
            None => {
                let (sender, state) = tokio::sync::watch::channel(FlightState::Running);
                let flight = Flight {
                    id: inner.next,
                    started: std::time::Instant::now(),
                    state,
                    sender,
                };
                inner.next += 1;
                inner.flight = Some(flight.clone());
                (flight, true)
            }
        }
    }

    /// Takes the flight out of the gate, when it is the one that has just finished.
    fn finish(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.flight.as_ref().is_some_and(|f| f.id == id) {
            inner.flight = None;
        }
    }

    /// Asks the engine, or joins the call that is already in flight.
    ///
    /// The call runs on a thread of its own, so a runtime worker is never held while
    /// the engine's lock is waited for, and the call is not tied to the runtime that
    /// asked for it. `limit` is how long the caller waits for the answer: the bound
    /// runs from the flight's start, so a request that joins a flight late waits for
    /// less of it.
    ///
    /// `None` when the engine did not answer in time.
    async fn run<F>(
        self: &Arc<Self>,
        engine: Arc<dyn Engine>,
        call: F,
        limit: std::time::Duration,
    ) -> Option<T>
    where
        F: FnOnce(&dyn Engine) -> T + Send + 'static,
    {
        let (flight, ours) = self.take_flight();
        if ours {
            let sender = flight.sender.clone();
            let gate = Arc::clone(self);
            let id = flight.id;
            let started = std::thread::Builder::new()
                .name("canvas-rgb-engine-call".into())
                .spawn(move || {
                    let value = call_the_engine(call, &engine);
                    let state = match value {
                        Some(value) => FlightState::Answered(value),
                        // The call panicked: the waiters are let go.
                        None => FlightState::GaveUp,
                    };
                    let _ = sender.send_replace(state);
                    // The flight is over: the next request starts a new one.
                    gate.finish(id);
                });
            if started.is_err() {
                // The thread could not start: the flight is over and gave nothing.
                let _ = flight.sender.send_replace(FlightState::GaveUp);
                self.finish(flight.id);
            }
        }
        let mut state = flight.state.clone();
        loop {
            let answered = match &*state.borrow() {
                FlightState::Answered(value) => Some(value.clone()),
                FlightState::GaveUp => return None,
                FlightState::Running => None,
            };
            if let Some(value) = answered {
                return Some(value);
            }
            let left = limit.saturating_sub(flight.started.elapsed());
            if left.is_zero() {
                return None;
            }
            match tokio::time::timeout(left, state.changed()).await {
                Ok(Ok(())) => continue,
                // The flight is over and gave nothing.
                Ok(Err(_)) => return None,
                // The bound ran out.
                Err(_) => return None,
            }
        }
    }

    /// Asks the engine on this thread, or leaves the one call of this kind alone.
    ///
    /// The daemon's watcher thread uses this: it is an ordinary OS thread, and the
    /// engine's lock is waited for there, outside every runtime. `None` when a call of
    /// this kind is already in flight: that call's answer is kept by the caller that
    /// made it, and a second call would put a second thread inside the engine.
    fn run_here<F>(&self, engine: Arc<dyn Engine>, call: F) -> Option<T>
    where
        F: FnOnce(&dyn Engine) -> T + Send + 'static,
    {
        let (flight, ours) = self.take_flight();
        if !ours {
            return None;
        }
        let value = call_the_engine(call, &engine);
        let state = match &value {
            Some(value) => FlightState::Answered(value.clone()),
            // The call panicked: the waiters are let go.
            None => FlightState::GaveUp,
        };
        let _ = flight.sender.send_replace(state);
        self.finish(flight.id);
        value
    }
}

/// Calls the engine, and keeps the answer.
///
/// `None` when the call panicked: a panic in the engine must not take the waiters with
/// it, and the flight has to end so the next request starts a new call.
fn call_the_engine<T, F>(call: F, engine: &Arc<dyn Engine>) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce(&dyn Engine) -> T + Send + 'static,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(engine.as_ref()))).ok()
}

/// The daemon's copy of one engine value, and the one call of that kind that may be
/// in flight at a time (#241).
pub struct EngineSlot<T> {
    /// The last value the engine gave, and when it gave it.
    cache: Mutex<EngineCache<T>>,
    /// The one call of this kind that may be in flight.
    gate: Arc<EngineGate<T>>,
}

impl<T: Clone + Send + Sync + 'static> EngineSlot<T> {
    /// A slot with no value in it.
    fn new() -> Self {
        Self { cache: Mutex::new(EngineCache::new()), gate: Arc::new(EngineGate::new()) }
    }

    /// The last value the engine gave.
    fn known(&self) -> Option<T> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).value.clone()
    }

    /// Keeps `value` and the time it came.
    fn set(&self, value: T) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).set(value);
    }

    /// Whether the copy is younger than `max_age`.
    fn is_fresh(&self, max_age: std::time::Duration) -> bool {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).fresh(max_age).is_some()
    }
}

/// What the daemon last heard from the render engine, and when it heard it.
///
/// The value is `None` until the engine has answered once. A route that has no value
/// must not answer with a made-up one (#241).
#[derive(Debug)]
struct EngineCache<T> {
    /// The last value the engine gave.
    value: Option<T>,
    /// When the engine gave it.
    filled_at: Option<std::time::Instant>,
}

impl<T> EngineCache<T> {
    fn new() -> Self {
        Self { value: None, filled_at: None }
    }

    /// Keeps `value` and the time it came.
    fn set(&mut self, value: T) {
        self.value = Some(value);
        self.filled_at = Some(std::time::Instant::now());
    }

    /// The value, if it is younger than `max_age`.
    fn fresh(&self, max_age: std::time::Duration) -> Option<&T> {
        let filled_at = self.filled_at?;
        if filled_at.elapsed() > max_age {
            return None;
        }
        self.value.as_ref()
    }
}

/// The threads that keep the daemon's copy of the engine's numbers fresh.
///
/// Dropping it asks them to stop. The daemon holds it for as long as it runs.
pub struct EngineWatch {
    stop: Arc<AtomicBool>,
}

impl Drop for EngineWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Keeps the engine's numbers and its group states fresh, so a route never waits for the
/// render thread.
///
/// One thread per kind asks the engine every [`ENGINE_POLL_INTERVAL`] and keeps the last
/// answer with the time it came. The engine's lock is the render thread's: a route that asked
/// for the numbers on a runtime worker would wait for the render thread there, and the
/// daemon's runtime has two workers (#241). The watchers are ordinary OS threads, not tasks
/// on the blocking pool, and they ask the engine through the same gate the routes use: there
/// is one call into the engine at a time for each kind, so a render thread that never hands
/// the lock over costs one thread for that kind and not one per request. A watcher whose
/// poll finds a call of its kind already in flight leaves that call alone (#241 review).
///
/// The daemon's quit flag does **not** stop the watchers: a quit that could not reach
/// the main task puts the flag back to zero and the daemon keeps running
/// (`crates/canvas-daemon/src/api/status.rs`), and a watcher that stopped there would
/// freeze the numbers for the rest of the daemon's life.
pub fn watch_engine(app: Arc<App>) -> EngineWatch {
    let stop = Arc::new(AtomicBool::new(false));
    let for_stats = Arc::clone(&app);
    let stats_stop = Arc::clone(&stop);
    if let Err(error) = std::thread::Builder::new()
        .name("canvas-rgb-engine-stats".into())
        .spawn(move || watch_numbers(for_stats, stats_stop))
    {
        tracing::error!(error = %error, "the watcher for the engine's numbers did not start");
    }
    let for_groups = Arc::clone(&app);
    let groups_stop = Arc::clone(&stop);
    if let Err(error) = std::thread::Builder::new()
        .name("canvas-rgb-engine-groups".into())
        .spawn(move || watch_groups(for_groups, groups_stop))
    {
        tracing::error!(error = %error, "the watcher for the engine's group states did not start");
    }
    EngineWatch { stop }
}

/// Asks the engine for its numbers and keeps the answer, until the guard is dropped.
fn watch_numbers(app: Arc<App>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        // The one call of this kind: the watcher asks the engine on its own thread, and
        // a call that is already in flight is left to the caller that made it.
        if let Some(stats) = app.poll_engine_stats() {
            app.engine_stats.set(stats);
        }
        std::thread::sleep(ENGINE_POLL_INTERVAL);
    }
}

/// Asks the engine for its group states and keeps the answer, until the guard is dropped.
fn watch_groups(app: Arc<App>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        if let Some(groups) = app.poll_engine_group_states() {
            app.engine_groups.set(groups);
        }
        std::thread::sleep(ENGINE_POLL_INTERVAL);
    }
}

/// The state every handler sees.
pub struct App {
    /// The on-disk state. Written rarely (a settings change) and read often.
    ///
    /// This is a Tokio lock, not a std one, so the guard is `Send` and an API handler
    /// can hold it across an await.
    pub store: tokio::sync::RwLock<Store>,
    /// Serialises store writes so they land on disk in the order callers made them (#241).
    /// `store_io` takes this before the store lock and holds it while a write is on a blocking
    /// thread; the store lock itself is free while the disk is busy.
    pub(crate) store_writer: Arc<tokio::sync::Mutex<()>>,
    /// The render engine handle.
    pub engine: Arc<dyn Engine>,
    /// The engine's last numbers, kept by [`watch_engine`] so a route never waits for the
    /// render thread (#241).
    engine_stats: EngineSlot<RenderStats>,
    /// The engine's last group states, kept the same way (#241).
    engine_groups: EngineSlot<Vec<GroupState>>,
    /// How old the daemon's copy of the engine's numbers may be before a route asks the
    /// engine itself (#241). The daemon keeps [`ENGINE_CACHE_MAX_AGE`]; a test shortens
    /// it.
    pub engine_cache_max_age: Mutex<std::time::Duration>,
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
    /// The service's version from its pipe `Hello`. The pipe client writes it; `/about` reads it.
    /// `None` until the service has answered (or for ever in a fakes run).
    pub service_version: Mutex<Option<String>>,
    /// The one scrubber for every string that leaves the app (#574). Tests replace it with a
    /// fake profile so no test depends on the account that runs it.
    pub scrubber: Mutex<crate::scrub::Scrubber>,
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
    /// The mode from the first service `Hello` (#577), latched for this run. `None` until it
    /// arrives (or for ever when the pipe cannot be reached in time).
    pub start_mode: Mutex<Option<SafeStartMode>>,
    /// How the last daemon run ended, from the same reply. `About.last_exit` reads it.
    pub last_exit: Mutex<LastExit>,
    /// The status field every surface reads. Built once when the mode is latched, with the
    /// fan line updated from the first fan status in safe mode.
    pub safe_start: Mutex<Option<SafeStart>>,
    /// The effect safe mode has on the lights, or `None` while there is no light to play on.
    pub safe_effect: Mutex<Option<String>>,
    /// The built-in effect safe mode plays: Breathing. A test names one that cannot be built.
    pub safe_builtin: Mutex<String>,
    /// Set once Breathing could not be built: safe mode shows one steady colour from then on.
    pub safe_fallback: std::sync::atomic::AtomicBool,
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
    /// A seam in a scene PUT, for the tests (#497): the PUT stops between validation and the
    /// write so a test can put a light delete in that gap. `None` in the daemon.
    scene_put_seam: Mutex<Option<Arc<crate::api::scenes::ScenePutSeam>>>,
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
        let user_effects = Arc::new(crate::user_effects::UserEffects::with_stock_tiles_and_import_load(
            crate::user_effects::html_builder(),
            crate::user_effects::html_builder_seeded(crate::thumbnail::TILE_SEED),
            crate::user_effects::html_builder_with_import_load(),
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
            engine_stats: EngineSlot::new(),
            engine_groups: EngineSlot::new(),
            engine_cache_max_age: Mutex::new(ENGINE_CACHE_MAX_AGE),
            displays: crate::displays::DisplayHub::new(display_adapter),
            peripherals: crate::peripherals::PeripheralHub::with_home(Vec::new(), home.clone()),
            home,
            identify: Arc::default(),
            last_frame: Arc::default(),
            transient: Default::default(),
            draft: Default::default(),
            import_seam: Mutex::new(None),
            scene_put_seam: Mutex::new(None),
            input_source: Mutex::new(None),
            preview_slots: Arc::new(tokio::sync::Semaphore::new(crate::api::editor::PREVIEW_CONCURRENT)),
            draft_slots: Arc::new(tokio::sync::Semaphore::new(crate::api::editor::PREVIEW_CONCURRENT)),
            fakes: false,
            scene: Default::default(),
            store: tokio::sync::RwLock::new(store),
            store_writer: Arc::new(tokio::sync::Mutex::new(())),
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
            service_version: Mutex::new(None),
            scrubber: Mutex::new(crate::scrub::Scrubber::from_env()),
            service_down_ms: AtomicU64::new(0),
            fans: tokio::sync::RwLock::new(None),
            fans_last: tokio::sync::RwLock::new(None),
            fans_pipe_limit: FANS_LOCK_PIPE_LIMIT,
            start_mode: Mutex::new(None),
            last_exit: Mutex::new(LastExit::Unknown),
            safe_start: Mutex::new(None),
            safe_effect: Mutex::new(None),
            safe_builtin: Mutex::new("breathing".into()),
            safe_fallback: std::sync::atomic::AtomicBool::new(false),
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

    /// Keeps the service's version from its `Hello` reply (#574 R6). The pipe client used to log
    /// and drop it, so `/about` could not say what service was answering.
    pub fn set_service_version(&self, version: String) {
        *self.service_version.lock().unwrap_or_else(|e| e.into_inner()) = Some(version);
    }

    /// Forgets the last `Hello`: a disconnected service has no version to show (#574).
    pub fn clear_service_version(&self) {
        *self.service_version.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The service version, when the service has answered a `Hello`.
    pub fn service_version(&self) -> Option<String> {
        self.service_version.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The latched mode, or Normal when the first `Hello` never arrived (#577 R13).
    pub fn start_mode(&self) -> SafeStartMode {
        self.start_mode.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or_default()
    }

    /// The latched verdict for Help & About. `Unknown` until the first `Hello` arrives.
    pub fn last_exit(&self) -> LastExit {
        *self.last_exit.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Latches the first `Hello`'s decision with no record from the service.
    pub fn latch_start(&self, mode: SafeStartMode, last_exit: LastExit) {
        self.latch_start_from(mode, last_exit, None);
    }

    /// Latches the first `Hello`'s decision. Later replies never change it mid-run. The reason,
    /// the streak and since when are the service's record; with none, they read as unknown.
    pub fn latch_start_from(&self, mode: SafeStartMode, last_exit: LastExit, record: Option<SafeStart>) {
        let mut slot = self.start_mode.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return;
        }
        *slot = Some(mode);
        drop(slot);
        *self.last_exit.lock().unwrap_or_else(|e| e.into_inner()) = last_exit;
        *self.safe_start.lock().unwrap_or_else(|e| e.into_inner()) = (mode == SafeStartMode::Safe).then(|| {
            let record = record.unwrap_or(SafeStart {
                active: true,
                reason: None,
                unclean_streak: 0,
                since: None,
                fans_on_bios: None,
            });
            SafeStart { active: true, fans_on_bios: None, ..record }
        });
    }

    /// Whether every fan header is on BIOS control, from the latest fan status. Unknown while
    /// the service is not answering or has listed no header.
    async fn fans_on_bios(&self) -> Option<bool> {
        if self.service_down_ms.load(Ordering::SeqCst) != 0 {
            return None;
        }
        let fans = self.fans.read().await;
        let headers = &fans.as_ref()?.headers;
        (!headers.is_empty()).then(|| headers.iter().all(|h| h.on_bios))
    }

    /// The status snapshot a surface reads, with the fan line the service confirmed.
    pub fn safe_start_status(&self) -> Option<SafeStart> {
        self.safe_start.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replaces the scrubber. Tests pass a fake profile here; production uses the one built from
    /// the process's own environment.
    pub fn set_scrubber(&self, scrubber: crate::scrub::Scrubber) {
        *self.scrubber.lock().unwrap_or_else(|e| e.into_inner()) = scrubber;
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

    /// Sets the seam in a scene PUT (tests only). The PUT stops between validation and the write
    /// until the seam lets it go. The daemon never sets one.
    pub fn set_scene_put_seam(&self, seam: Arc<crate::api::scenes::ScenePutSeam>) {
        *self.scene_put_seam.lock().unwrap_or_else(|e| e.into_inner()) = Some(seam);
    }

    /// The seam in a scene PUT, if a test set one.
    pub fn scene_put_seam(&self) -> Option<Arc<crate::api::scenes::ScenePutSeam>> {
        self.scene_put_seam.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
    /// The numbers come from the daemon's watcher threads ([`watch_engine`]), which ask
    /// the engine every [`ENGINE_POLL_INTERVAL`]. A copy younger than
    /// [`App::engine_cache_max_age`] is answered as it is; an older one is asked of the
    /// engine itself, on the blocking pool and with a bound, so a watcher that stopped
    /// does not freeze the numbers (#241).
    ///
    /// `None` when the engine has never answered: the caller must not answer with
    /// zeros that look like a measurement.
    pub async fn engine_stats(&self) -> Option<RenderStats> {
        self.engine_value(&self.engine_stats, |engine| engine.stats()).await
    }

    /// The engine's group states, kept the same way ([`App::engine_stats`]).
    ///
    /// `None` when the engine has never answered: an empty list would say that no
    /// group failed, and a group can fail (#241).
    pub async fn engine_group_states(&self) -> Option<Vec<GroupState>> {
        self.engine_value(&self.engine_groups, |engine| engine.group_states()).await
    }

    /// The engine's numbers, asked of the engine on this thread.
    ///
    /// The daemon's watcher thread uses this: it wants the engine's own numbers, not
    /// the copy the routes read. It is the one call of its kind, so the engine is asked
    /// once at a time however many callers there are (#241).
    ///
    /// `None` when a call of this kind is already in flight: that call's answer is
    /// kept by the caller that made it.
    fn poll_engine_stats(&self) -> Option<RenderStats> {
        self.engine_stats.gate.run_here(Arc::clone(&self.engine), |engine| engine.stats())
    }

    /// The engine's group states, asked of the engine on this thread
    /// ([`App::poll_engine_stats`]).
    fn poll_engine_group_states(&self) -> Option<Vec<GroupState>> {
        self.engine_groups.gate.run_here(Arc::clone(&self.engine), |engine| engine.group_states())
    }

    /// The engine's value for `slot`, without waiting for the render thread.
    ///
    /// A copy younger than [`App::engine_cache_max_age`] is the answer. An older one is
    /// asked of the engine, bounded by [`ENGINE_CALL_LIMIT`]: the answer may be late,
    /// but it is the engine's own. When the engine does not answer, the last value it
    /// gave is the answer (it is true as of the time it gave it), and `None` when the
    /// engine has never answered at all.
    async fn engine_value<T, F>(&self, slot: &EngineSlot<T>, call: F) -> Option<T>
    where
        T: Clone + Send + Sync + 'static,
        F: FnOnce(&dyn Engine) -> T + Send + 'static,
    {
        let known = slot.known();
        let max_age = *self.engine_cache_max_age.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_fresh(max_age) {
            return known;
        }
        match slot.gate.run(Arc::clone(&self.engine), call, ENGINE_CALL_LIMIT).await {
            Some(value) => {
                slot.set(value.clone());
                Some(value)
            }
            // The engine did not answer in time: the route answers with the numbers it
            // has, and the API keeps serving.
            None => known,
        }
    }

    /// The status snapshot the API and the CLI read.
    ///
    /// `None` when the render engine has never answered: the caller says so instead of
    /// answering zeros that look like a measurement (#241).
    pub async fn status(&self) -> Option<Status> {
        let stats = self.engine_stats().await?;
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
        // #577: in safe start the synthetic scene is what is actually on the lights, and the
        // saved scene is not run. The status names what plays, not what would play after a
        // restart. Nothing here is written to the store.
        let mut safe = self.safe_start_status();
        if let Some(safe) = safe.as_mut() {
            safe.fans_on_bios = self.fans_on_bios().await;
        }
        let (active_scene, active_effects) = if safe.as_ref().is_some_and(|s| s.active) {
            let effect = self.safe_effect.lock().unwrap_or_else(|e| e.into_inner()).clone();
            (
                Some(canvas_proto::api::SceneRef {
                    id: crate::scenes::SAFE_SCENE.into(),
                    name: crate::scenes::safe_scene_name(self.safe_fallback.load(Ordering::SeqCst)).into(),
                }),
                effect.into_iter().collect::<Vec<_>>(),
            )
        } else {
            (active_scene, active_effects)
        };
        let openrgb = *self.openrgb.read().await;
        let conflicts = self.conflicts.read().await.clone();
        drop(store);
        let down = self.service_down_ms.load(Ordering::SeqCst);
        Some(Status {
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
            // #577: the value latched from the first Hello, with the service's fan answer.
            safe_start: safe,
        })
    }

    /// The fan profile names the service knows about.
    pub async fn fan_profile_names(&self) -> Vec<String> {
        self.fans.read().await.as_ref().map(|s| s.profiles.clone()).unwrap_or_default()
    }
}

/// A per-device frame sink: the newest frame, on its way to the device's writer task.
///
/// The frame sits in a watch channel and not in a queue: the device is often slower than the
/// render loop, and the frame that matters is the newest one. A queue of one keeps the
/// oldest frame and drops the newest, so the device would show colours the scene has left
/// behind, and the frame that brings the lights back after a resume (#145) could be the one
/// that is dropped (#308). The engine never blocks either way.
pub struct DeviceSink {
    id: DeviceId,
    tx: tokio::sync::watch::Sender<Vec<canvas_proto::Rgb8>>,
    /// While this is active the engine's frames are dropped (the identify override).
    gate: Option<Arc<crate::identify::IdentifyGate>>,
}

impl DeviceSink {
    pub fn new(id: DeviceId, tx: tokio::sync::watch::Sender<Vec<canvas_proto::Rgb8>>) -> Self {
        Self { id, tx, gate: None }
    }

    /// A sink that obeys the identify override and lets its ticker reach the same writer.
    pub fn with_gate(
        id: DeviceId,
        tx: tokio::sync::watch::Sender<Vec<canvas_proto::Rgb8>>,
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
        // A paint of every sink does not reach the controller rings (#376).
        if self.id.0.starts_with(canvas_adapters::eightbitdo_ultimate2::ring::RING_PREFIX)
            && self.gate.as_ref().is_some_and(|g| g.is_painting_solid())
        {
            return;
        }
        // No reader left: the device's writer is gone, and nobody will take the frame.
        if self.tx.receiver_count() == 0 {
            tracing::warn!(device = %self.id, "device writer is gone; dropping a frame");
            return;
        }
        // The newest frame wins. The writer takes the frame that is there when it is ready:
        // a frame that arrives while the device is behind replaces the older one instead of
        // being dropped, and the engine never blocks.
        self.tx.send_replace(colors);
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

    /// The sink keeps the newest frame and never blocks the engine (#308).
    ///
    /// The device is often slower than the render loop. The frame that matters is the newest
    /// one, so a frame that arrives while the device is behind replaces the older one
    /// instead of being dropped.
    #[test]
    fn device_sink_keeps_the_newest_frame() {
        let (tx, mut rx) = tokio::sync::watch::channel(vec![canvas_proto::Rgb8::BLACK]);
        let sink = DeviceSink::new(DeviceId("openrgb:abc".into()), tx);
        sink.submit(vec![canvas_proto::Rgb8::WHITE]);
        sink.submit(vec![canvas_proto::Rgb8::new(255, 0, 0)]);
        // The writer takes the newest frame, not the first one.
        let newest = rx.borrow_and_update().clone();
        assert_eq!(newest, vec![canvas_proto::Rgb8::new(255, 0, 0)]);
    }

    #[test]
    fn conflicts_are_reported() {
        let app_conflicts = [Conflict::SignalRgb { process: true, service: false }];
        assert!(matches!(app_conflicts[0], Conflict::SignalRgb { process: true, .. }));
    }
}
