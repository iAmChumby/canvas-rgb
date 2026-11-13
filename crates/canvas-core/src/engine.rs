//! The render loop: one thread, a fixed-step timer, and the state the loop owns.
//!
//! Threading model:
//! - The public `Engine` handle owns a `Mutex<State>` and a `Condvar`.
//! - A single background thread does the rendering. It waits on the condvar when it
//!   has nothing to do (parked or paused) and sleeps until the next tick when it is
//!   running.
//! - Callers (`set_fps`, `set_layout`, ...) mutate the state under the lock and nudge
//!   the condvar so the thread reacts immediately.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use canvas_proto::api::Power;
use canvas_proto::color::{warmth_gains, Rgb8, WhiteBalance};
use canvas_proto::device::{DeviceId, DeviceInfo};
use canvas_proto::layout::{Layout, SamplingMode};
use canvas_proto::props::{InputNeeds, Props};
use canvas_proto::scene::Light;
use canvas_proto::traits::{
    Canvas, Effect, Engine, EngineGroup, EngineSettings, FrameSink, GroupState, InputSource,
    LightPreviewSink, PreviewSink, RenderStats,
};
use tracing::{info, warn};

use crate::geometry::{self, LayoutPlan};
use crate::preview;
use crate::sampling;
use crate::scene_engine::{FrameParams, GroupSlot, SceneRunner, SceneState};
use crate::timer::FrameTimer;

use crate::effect_guard;

/// Runs effect code behind the guard: `catch_unwind` with the marker set, so the
/// daemon's panic hook knows a panic here is a caught effect panic, never the
/// daemon's own fatal panic (#99, finding 1).
pub(crate) fn guarded_call<R>(f: impl FnOnce() -> R) -> Result<R, Box<dyn std::any::Any + Send>> {
    catch_unwind(AssertUnwindSafe(|| effect_guard::run_guarded(f)))
}

/// Frame rate limits, from the spine.
const FPS_MIN: u32 = canvas_proto::consts::FPS_MIN;
const FPS_MAX: u32 = canvas_proto::consts::FPS_MAX;
/// The preview stream never exceeds this rate.
const PREVIEW_MAX_FPS: u32 = canvas_proto::consts::PREVIEW_MAX_FPS;

/// How long the parked thread waits before re-checking its state. It is woken
/// immediately by any state change, so this is only a safety net against a missed
/// notification.
pub(crate) const PARK_POLL: Duration = Duration::from_millis(100);

/// One device's sink, plus the LED count its frames must have.
struct SinkEntry {
    device_id: DeviceId,
    sink: Arc<dyn FrameSink>,
    /// LED count of the device; a frame has exactly this many colors.
    led_count: u32,
}

/// Everything the render thread reads and the API mutates.
struct State {
    fps: u32,
    power: Power,
    brightness: u8,
    /// #50: the global warmth stage's per-channel gains ([`warmth_gains`]).
    warmth: [f32; 3],
    /// #50: each device's white-balance gains. Behind an `Arc` so the render thread's
    /// per-frame snapshot is a pointer copy.
    white_balance: Arc<HashMap<DeviceId, [f32; 3]>>,
    /// The active layout's precomputed sample plan.
    plan: Option<LayoutPlan>,
    /// Bumped whenever `plan` is replaced, so the render thread knows to pick it up.
    plan_generation: u64,
    /// Device topology, so the plan can be rebuilt when a zone changes size.
    devices: Vec<DeviceInfo>,
    /// Sinks, in the order their devices appear in the plan.
    sinks: Vec<SinkEntry>,
    preview: Option<Arc<dyn PreviewSink>>,
    /// Scenes: where the editor's live LEDs go.
    light_preview: Option<Arc<dyn LightPreviewSink>>,
    /// Scenes: the running scene. `Some` is scene mode (after `set_scene`); `set_layout` makes
    /// it `None` again and the layout path runs.
    scene: Option<SceneState>,
    /// Scenes: the next generation to give an effect. Unique for the engine's whole life, so
    /// an effect that was in flight when its slot was replaced can never be put back.
    scene_effect_counter: u64,
    /// Scenes: the next scene generation.
    scene_counter: u64,
    input_source: Option<Arc<dyn InputSource>>,
    /// The active effect. `None` means "paint nothing" (a black canvas).
    effect: Option<Box<dyn Effect>>,
    /// True when the active effect panicked in `init`/`update`/`schema` and must not be
    /// called again until `set_effect` replaces it (#99). The canvas keeps the last good
    /// frame while the effect is skipped.
    effect_panicked: bool,
    /// Bumped whenever the effect is replaced, so the render thread knows that the
    /// effect it is drawing with has been superseded.
    effect_generation: u64,
    /// The props the active effect was last initialised with. When these differ from
    /// [`pending_props`], the render thread re-initialises the effect before the next
    /// frame.
    props: Props,
    /// The props the caller asked for, applied on the next frame.
    pending_props: Props,
    /// Whether `pending_props` still needs to be applied.
    props_dirty: bool,
    /// When the effect was activated; `Inputs.time` counts from here.
    effect_started: Option<Instant>,
    /// Frame counter.
    frames_rendered: u64,
    /// True while the loop is parked (running, but nothing consumes frames).
    parked: bool,
    /// Frame timestamps of the last second, for the measured fps.
    recent: VecDeque<Instant>,
    /// Measured frame time of the last completed frame.
    frame_time_ms: f32,
    /// Set to true to make the thread exit.
    stop: bool,
    /// Set when a black frame must be written to every sink (the pause transition).
    pending_black: bool,
    /// Set when a sink was added and has not been given a frame yet (#145).
    ///
    /// A panicked effect stops the delivery (#99), so a sink that arrives while the
    /// effect is stopped would wait for the next effect. The render thread re-sends the
    /// last good frame when it sees this.
    pending_sink: bool,
    /// Set when the power came back to running after a pause, and the devices have not
    /// been given a frame again yet (#145).
    ///
    /// The flag is set by `set_power`, not by the render thread noticing the pause: a
    /// pause and a resume that both land between two ticks would never be seen by the
    /// thread, and the devices would keep the black frame the pause wrote.
    pending_resume: bool,
    /// Bumped on every state change. The parked thread remembers the value it saw
    /// and only re-reads the state when it changes, so a notification that arrives
    /// between the check and the wait cannot be missed.
    wake_generation: u64,
    /// The operating-system thread id of the render thread, so a diagnostic can
    /// measure that thread's CPU time on its own.
    render_thread_id: Option<u32>,
}

impl State {
    fn new(settings: &EngineSettings) -> Self {
        Self {
            fps: settings.fps.clamp(FPS_MIN, FPS_MAX),
            power: settings.power,
            brightness: settings.brightness.min(100),
            warmth: warmth_gains(f32::from(settings.warmth)),
            white_balance: Arc::new(HashMap::new()),
            plan: None,
            plan_generation: 0,
            devices: Vec::new(),
            sinks: Vec::new(),
            preview: None,
            light_preview: None,
            scene: None,
            scene_effect_counter: 0,
            scene_counter: 0,
            input_source: None,
            effect: None,
            effect_panicked: false,
            effect_generation: 0,
            props: Props::new(),
            pending_props: Props::new(),
            props_dirty: false,
            effect_started: None,
            frames_rendered: 0,
            parked: false,
            recent: VecDeque::new(),
            frame_time_ms: 0.0,
            stop: false,
            pending_black: false,
            pending_sink: false,
            pending_resume: false,
            wake_generation: 0,
            render_thread_id: None,
        }
    }

    /// Whether anything consumes frames.
    fn has_consumers(&self) -> bool {
        !self.sinks.is_empty() || self.preview.is_some() || self.light_preview.is_some()
    }

    /// The measured fps over the last second.
    ///
    /// Zero while the loop is parked or paused: no frames are being rendered, so
    /// there is no rate to report.
    /// Records a rendered frame and drops the ones older than a second, so `measured_fps` is the
    /// rate of the last second and the window stays small. (It used to compare in the wrong
    /// order, `recent[0].duration_since(now)`, which is always zero: nothing was ever dropped, the
    /// window grew by one entry per frame forever and the rate was the average since start.)
    fn record_frame(&mut self, at: Instant) {
        self.recent.push_back(at);
        while self.recent.len() > 2 && at.duration_since(self.recent[0]) > Duration::from_secs(1) {
            self.recent.pop_front();
        }
    }

    fn measured_fps(&self) -> f32 {
        if self.parked || self.power == Power::Paused {
            return 0.0;
        }
        if self.recent.len() < 2 {
            return 0.0;
        }
        let first = self.recent[0];
        let last = self.recent[self.recent.len() - 1];
        let span = last.duration_since(first);
        if span.is_zero() {
            return 0.0;
        }
        let frames = (self.recent.len() - 1) as f32;
        (frames / span.as_secs_f32()).round()
    }

    /// Applies a power change.
    ///
    /// The pause asks for one black frame, and the resume asks for the last good
    /// frame again (#145). A pause and a resume that both land between two ticks
    /// never reach the render thread, so the black frame of that pause is never
    /// written: the resume drops the request, because a pause that never happened
    /// must not black out a device later.
    fn apply_power(&mut self, power: Power) {
        if self.power == power {
            return;
        }
        let was_paused = self.power == Power::Paused;
        self.power = power;
        if power == Power::Paused {
            // "Paused writes black once, then stops the render loop."
            self.pending_black = true;
        } else if was_paused {
            // The output came back. The devices need a frame again: the pause wrote
            // black, and a panicked effect stops the delivery (#99), so the render
            // thread re-sends the last good frame (#145).
            self.pending_resume = true;
            // The black frame of that pause was never written, so the request is
            // stale: it goes.
            self.pending_black = false;
        }
    }
}

/// A test's hook that sees each frame's measured time in ms.
#[cfg(test)]
pub(crate) type FrameTimeHook = Arc<dyn Fn(f32) + Send + Sync>;

/// The public handle.
pub struct CoreEngine {
    state: Mutex<State>,
    /// Signalled whenever the state changes in a way the thread cares about.
    wake: Condvar,
    /// The render thread's join handle.
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// How many times the parked loop woke up. A test counts them: a parked engine that
    /// spins shows thousands of wake-ups a second (#39, #47).
    park_wakeups: std::sync::atomic::AtomicU64,
    /// How many effect panics the engine has caught (one per effect instance, #99).
    /// A test reads it to check that a panicking effect logs once and is never called
    /// again until it is replaced.
    effect_panics: std::sync::atomic::AtomicU64,
    /// A test's hook, called on the render thread between its decision to run or park and
    /// the park wait (#250). Set before the thread starts, so it sees every iteration.
    #[cfg(test)]
    decided_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// A test's hook, called with every frame's measured time in ms: the same number `stats()`
    /// reports as `frame_time_ms`, for every frame instead of the last one.
    #[cfg(test)]
    frame_time_hook: Mutex<Option<FrameTimeHook>>,
}

impl CoreEngine {
    /// Runs the test's hook, if it set one. Called with no lock held.
    #[cfg(test)]
    fn decided_hook(&self) {
        let hook = self.decided_hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Wakes the render thread so it re-reads the state.
    fn wake(&self) {
        self.wake.notify_all();
    }

    /// Locks the state, mutating it, then wakes the thread.
    fn mutate<F: FnOnce(&mut State)>(&self, f: F) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut state);
            state.wake_generation = state.wake_generation.wrapping_add(1);
        }
        self.wake();
    }

    /// A group as the state holds it: its effect initialised behind the guard, its needs read
    /// once. An effect that panics in `init` or `schema` is installed as failed, with the text
    /// of the panic, and is not called again until it is replaced. The caller assigns the
    /// generation under the lock.
    fn group_slot(&self, group: EngineGroup) -> GroupSlot {
        let mut effect = group.effect;
        let mut failed = None;
        let mut needs = canvas_proto::props::InputNeeds::default();
        if let Some(e) = effect.as_mut() {
            let outcome = guarded_call(|| {
                e.init(&group.props);
                e.schema().needs
            });
            match outcome {
                Ok(n) => needs = n,
                Err(payload) => {
                    report_effect_panic(e.as_ref(), "init", &payload);
                    self.effect_panics.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    failed = Some(panic_text(&payload));
                }
            }
        }
        GroupSlot {
            id: group.id,
            members: group.members,
            mode: group.mode,
            brightness: group.brightness.min(100),
            effect,
            needs,
            effect_generation: 0,
            started: Instant::now(),
            pending_props: None,
            failed,
        }
    }

    /// Drops replaced effects behind the guard, outside the state lock: an effect's `Drop` is
    /// effect code, and a panic in it must not take the caller or the lock down.
    fn drop_effects(&self, effects: impl IntoIterator<Item = Box<dyn Effect>>) {
        for effect in effects {
            if guarded_call(|| drop(effect)).is_err() {
                self.effect_panics.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                warn!("a replaced effect panicked in Drop; it was already superseded");
            }
        }
    }

    /// Sets the test hook that sees every frame's measured time in ms.
    #[cfg(test)]
    pub(crate) fn set_frame_time_hook(&self, hook: Option<FrameTimeHook>) {
        *self.frame_time_hook.lock().unwrap_or_else(|e| e.into_inner()) = hook;
    }

    /// The render thread's operating-system thread id, if the thread has started.
    ///
    /// Used by the CPU measurement test to time that thread on its own.
    #[cfg(test)]
    pub(crate) fn render_thread_id(&self) -> Option<u32> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.render_thread_id
    }

    /// How many times the parked loop has woken up.
    ///
    /// The parked loop waits [`PARK_POLL`] between wake-ups, so this counts the wake-ups of a
    /// parked engine: a spinning loop shows thousands of them (#39, #47).
    #[cfg(test)]
    pub(crate) fn park_wakeups(&self) -> u64 {
        self.park_wakeups.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// How many effect panics the engine has caught (one per effect instance).
    ///
    /// Exposed for tests and diagnostics; the regression tests count calls on the
    /// fakes so they also compile against the unguarded engine.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn effect_panic_count(&self) -> u64 {
        self.effect_panics.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether the active effect must no longer be called (it panicked, #99).
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn effect_panicked(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .effect_panicked
    }
}

/// A caught panic's payload as text. Same rule as the daemon's `panic_message`
/// (`crates/canvas-daemon/src/exit_log.rs`): `panic!("x")` carries a `&str`,
/// `panic!("{x}")` a `String`.
pub(crate) fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic with a payload that is not text".to_string()
    }
}

/// An effect's id, for the panic log (#99), without ever panicking itself: a `schema`
/// that panics gives `"<unknown>"`.
fn effect_id_guarded(effect: &dyn Effect) -> String {
    match guarded_call(|| effect.schema().id) {
        Ok(id) => id,
        Err(_) => "<unknown effect id>".to_string(),
    }
}

/// Logs one caught effect panic: the effect id and the panic message, once per effect
/// instance. The caller marks the effect stopped, so this is the only line that effect
/// gives until it is replaced.
pub(crate) fn report_effect_panic(effect: &dyn Effect, phase: &str, payload: &Box<dyn std::any::Any + Send>) {
    let effect_id = effect_id_guarded(effect);
    let message = panic_text(payload);
    tracing::error!(effect_id = %effect_id, phase, message = %message, "an effect panicked; it will not be called again until a new effect is set");
}

impl Engine for CoreEngine {
    fn set_layout(&self, layout: Layout, devices: Vec<DeviceInfo>) {
        let sampling = layout.sampling;
        let scale_percent = layout.render_scale.percent();
        let plan = geometry::build_plan(&layout, &devices);
        let summary = plan.summary();
        let device_count = plan.device_count();
        // The scene ends here, and its effects are dropped behind the guard, outside the
        // lock, exactly like `set_scene` drops the scene it replaces: an effect's `Drop` is
        // effect code, and a panic in it must not take the caller or the lock down.
        let old = {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let state = &mut *guard;
            // The layout path runs again: scene mode ends.
            let old = state.scene.take();
            state.devices = devices;
            state.plan = Some(plan);
            state.plan_generation = state.plan_generation.wrapping_add(1);
            // Sinks may have arrived before the layout did; keep their LED counts in
            // sync with the now-known topology.
            for sink in state.sinks.iter_mut() {
                sink.led_count = state
                    .devices
                    .iter()
                    .find(|d| d.id == sink.device_id)
                    .map(|d| d.led_count())
                    .unwrap_or(sink.led_count);
            }
            state.wake_generation = state.wake_generation.wrapping_add(1);
            old
        };
        self.wake();
        if let Some(old) = old {
            self.drop_effects(old.groups.into_iter().filter_map(|g| g.effect));
        }
        info!(
            devices = device_count,
            ?sampling,
            scale = scale_percent,
            "layout applied; sample regions recomputed: {summary}"
        );
    }

    fn set_effect(&self, effect: Box<dyn Effect>, props: &Props) {
        let mut effect = effect;
        // #99: an initialising panic must not take the calling thread (or, when the new
        // effect is handed to the render thread, the render thread) down with it. The
        // effect is installed as stopped, so the loop skips it until it is replaced.
        let init_panicked = match guarded_call(|| effect.init(props)) {
            Ok(()) => false,
            Err(payload) => {
                report_effect_panic(effect.as_ref(), "init", &payload);
                self.effect_panics.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                true
            }
        };
        // Finding 3: the replaced effect's `Drop` is effect code too. Take it out
        // under the lock but drop it after the lock, behind the guard, so a panicking
        // drop can neither kill this thread nor hold the state lock while it unwinds.
        let old = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let old = state.effect.take();
            state.effect = Some(effect);
            state.effect_panicked = init_panicked;
            state.effect_generation = state.effect_generation.wrapping_add(1);
            state.effect_started = Some(Instant::now());
            state.props = props.clone();
            state.pending_props = props.clone();
            state.props_dirty = false;
            state.wake_generation = state.wake_generation.wrapping_add(1);
            old
        };
        self.wake();
        if let Some(old) = old {
            if guarded_call(|| drop(old)).is_err() {
                let _ = self.effect_panics.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                warn!("a replaced effect panicked in Drop; it was already superseded");
            }
        }
        info!("effect activated");
    }

    fn set_props(&self, props: &Props) {
        // The effect may be in the render thread's hands right now, so the new props
        // are queued and applied at the start of the next frame.
        let mut panicked: Option<(String, String)> = None;
        self.mutate(|state| {
            state.pending_props = props.clone();
            state.props_dirty = true;
            // If the effect is idle in the state, initialise it right away; there is
            // no frame in flight to race with. A stopped effect is never called again
            // until it is replaced (#99).
            if state.effect_panicked {
                return;
            }
            if let Some(effect) = state.effect.as_mut() {
                let outcome = guarded_call(|| effect.init(props));
                match outcome {
                    Ok(()) => {
                        state.props = props.clone();
                        state.props_dirty = false;
                    }
                    Err(payload) => {
                        // Leave `props_dirty` set: the render thread clears it without
                        // calling the stopped effect again.
                        state.effect_panicked = true;
                        let effect_id = effect_id_guarded(effect.as_ref());
                        panicked = Some((effect_id, panic_text(&payload)));
                    }
                }
            }
        });
        if let Some((effect_id, message)) = panicked {
            self.effect_panics.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracing::error!(effect_id = %effect_id, phase = "init", message = %message, "an effect panicked; it will not be called again until a new effect is set");
        }
    }

    fn set_fps(&self, fps: u32) {
        let clamped = fps.clamp(FPS_MIN, FPS_MAX);
        if clamped != fps {
            warn!(requested = fps, clamped, "fps out of range; clamped");
        }
        self.mutate(|state| state.fps = clamped);
        info!(fps = clamped, "fps changed");
    }

    fn set_power(&self, power: Power) {
        self.mutate(|state| state.apply_power(power));
        info!(?power, "power changed");
    }

    fn set_brightness(&self, brightness: u8) {
        let clamped = brightness.min(100);
        self.mutate(|state| state.brightness = clamped);
    }

    fn set_warmth(&self, kelvin: u16) {
        let gains = warmth_gains(f32::from(kelvin));
        self.mutate(|state| state.warmth = gains);
    }

    fn set_white_balance(&self, balances: HashMap<DeviceId, WhiteBalance>) {
        // An invalid entry is neutral: the API refuses one, so this only guards the trait.
        let gains: HashMap<DeviceId, [f32; 3]> = balances
            .into_iter()
            .filter(|(_, wb)| wb.is_valid() && *wb != WhiteBalance::NEUTRAL)
            .map(|(id, wb)| (id, wb.gains()))
            .collect();
        let gains = Arc::new(gains);
        self.mutate(|state| state.white_balance = gains);
    }

    fn add_sink(&self, device: DeviceId, sink: Arc<dyn FrameSink>) {
        let device_id = device.clone();
        self.mutate(|state| {
            let led_count = state
                .devices
                .iter()
                .find(|d| d.id == device_id)
                .map(|d| d.led_count())
                .unwrap_or(0);
            state.sinks.retain(|s| s.device_id != device_id);
            state.sinks.push(SinkEntry {
                device_id: device_id.clone(),
                sink,
                led_count,
            });
            // The new sink has no frame yet. A panicked effect stops the delivery, so the
            // render thread re-sends the last good frame when it sees this (#145).
            state.pending_sink = true;
        });
        info!(device = %device, "sink added");
    }

    fn remove_sink(&self, device: &DeviceId) {
        self.mutate(|state| state.sinks.retain(|s| &s.device_id != device));
        info!(device = %device, "sink removed");
    }

    fn set_preview(&self, sink: Option<Arc<dyn PreviewSink>>) {
        let enabled = sink.is_some();
        self.mutate(|state| state.preview = sink);
        info!(enabled, "preview changed");
    }

    fn set_input_source(&self, source: Option<Arc<dyn InputSource>>) {
        self.mutate(|state| state.input_source = source);
    }

    fn write_solid(&self, color: Rgb8) {
        self.mutate(|state| {
            for sink in state.sinks.iter().filter(|s| s.led_count > 0) {
                sink.sink.submit(vec![color; sink.led_count as usize]);
            }
        });
    }

    fn stats(&self) -> RenderStats {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        RenderStats {
            fps_measured: state.measured_fps(),
            frame_time_ms: state.frame_time_ms,
            frames_rendered: state.frames_rendered,
            parked: state.parked,
            sinks: state.sinks.len() as u32,
            preview: state.preview.is_some(),
        }
    }

    // Scenes (0.14.0, #310): one effect per group. The per-frame work is in `scene_engine`.
    fn set_scene(
        &self,
        lights: Vec<Light>,
        devices: Vec<DeviceInfo>,
        groups: Vec<EngineGroup>,
        fade: bool,
    ) {
        // Every effect is initialised here, on the caller's thread and behind the guard, like
        // `set_effect`: an initialising panic marks that group failed and nothing else.
        let mut slots: Vec<GroupSlot> = groups.into_iter().map(|g| self.group_slot(g)).collect();
        let (group_count, device_count) = (slots.len(), devices.len());
        let old = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            for slot in slots.iter_mut() {
                state.scene_effect_counter += 1;
                slot.effect_generation = state.scene_effect_counter;
            }
            state.scene_counter += 1;
            let generation = state.scene_counter;
            let old = state.scene.replace(SceneState {
                generation,
                lights,
                groups: slots,
                fade_pending: fade,
            });
            state.devices = devices;
            // Sinks may have arrived before the scene did; a sink's LED count follows the
            // topology, so a frame built for the old size of a zone is dropped at delivery.
            let known: Vec<(DeviceId, u32)> =
                state.devices.iter().map(|d| (d.id.clone(), d.led_count())).collect();
            for sink in state.sinks.iter_mut() {
                if let Some((_, count)) = known.iter().find(|(id, _)| *id == sink.device_id) {
                    sink.led_count = *count;
                }
            }
            state.wake_generation = state.wake_generation.wrapping_add(1);
            old
        };
        self.wake();
        // The replaced scene's effects drop behind the guard, outside the lock.
        if let Some(old) = old {
            self.drop_effects(old.groups.into_iter().filter_map(|g| g.effect));
        }
        info!(groups = group_count, devices = device_count, fade, "scene applied");
    }

    fn set_group_effect(&self, group: &str, effect: Option<Box<dyn Effect>>, props: &Props) {
        let mut replacement = self.group_slot(EngineGroup {
            id: group.to_string(),
            members: Vec::new(),
            mode: canvas_proto::scene::GroupMode::FlowAcross,
            brightness: 100,
            effect,
            props: props.clone(),
        });
        let superseded = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.scene_effect_counter += 1;
            let generation = state.scene_effect_counter;
            let slot = state
                .scene
                .as_mut()
                .and_then(|scene| scene.groups.iter_mut().find(|g| g.id == group));
            let superseded = match slot {
                Some(slot) => {
                    let old = slot.effect.take();
                    slot.effect = replacement.effect.take();
                    slot.needs = replacement.needs;
                    slot.effect_generation = generation;
                    slot.started = Instant::now();
                    slot.pending_props = None;
                    slot.failed = replacement.failed.take();
                    old
                }
                None => {
                    warn!(group, "set_group_effect: no such group in the running scene");
                    replacement.effect.take()
                }
            };
            state.wake_generation = state.wake_generation.wrapping_add(1);
            superseded
        };
        self.wake();
        self.drop_effects(superseded);
    }

    fn set_group_props(&self, group: &str, props: &Props) {
        // The effect may be in the render thread's hands, so the props are queued and applied
        // at the start of the group's next frame.
        self.mutate(|state| {
            if let Some(slot) = state
                .scene
                .as_mut()
                .and_then(|scene| scene.groups.iter_mut().find(|g| g.id == group))
            {
                slot.pending_props = Some(props.clone());
            }
        });
    }

    fn set_group_brightness(&self, group: &str, brightness: u8) {
        let clamped = brightness.min(100);
        self.mutate(|state| {
            if let Some(slot) = state
                .scene
                .as_mut()
                .and_then(|scene| scene.groups.iter_mut().find(|g| g.id == group))
            {
                slot.brightness = clamped;
            }
        });
    }

    fn set_light_preview(&self, sink: Option<Arc<dyn LightPreviewSink>>) {
        let enabled = sink.is_some();
        self.mutate(|state| state.light_preview = sink);
        info!(enabled, "light preview changed");
    }

    fn group_states(&self) -> Vec<GroupState> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .scene
            .as_ref()
            .map(|scene| {
                scene
                    .groups
                    .iter()
                    .map(|g| GroupState { id: g.id.clone(), failed: g.failed.clone() })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn shutdown(&self) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stop {
                return;
            }
            state.stop = true;
        }
        self.wake();
        let handle = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        info!("engine shut down");
    }
}

impl Drop for CoreEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Starts the render thread and returns the concrete engine.
///
/// The public entry point is [`crate::spawn`], which hands out the trait object.
pub fn spawn_core(settings: EngineSettings) -> Arc<CoreEngine> {
    let engine = new_core(&settings);
    start_render_thread(&engine);
    engine
}

/// The engine with no render thread yet. A test sets its hook on it, then starts the thread.
fn new_core(settings: &EngineSettings) -> Arc<CoreEngine> {
    Arc::new(CoreEngine {
        state: Mutex::new(State::new(settings)),
        wake: Condvar::new(),
        thread: Mutex::new(None),
        park_wakeups: std::sync::atomic::AtomicU64::new(0),
        effect_panics: std::sync::atomic::AtomicU64::new(0),
        #[cfg(test)]
        decided_hook: Mutex::new(None),
        #[cfg(test)]
        frame_time_hook: Mutex::new(None),
    })
}

/// Starts the render thread of an engine made by [`new_core`].
fn start_render_thread(engine: &Arc<CoreEngine>) {
    let thread_engine = Arc::clone(engine);
    let handle = std::thread::Builder::new()
        .name("canvas-rgb-render".into())
        .spawn(move || render_loop(thread_engine))
        .ok();

    {
        let mut thread_slot = engine.thread.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(h) = handle {
            *thread_slot = Some(h);
        } else {
            warn!("could not spawn the render thread");
        }
    }
}

/// An engine whose render thread has not started, with a hook on its park decision (#250).
#[cfg(test)]
pub(crate) fn new_core_with_decided_hook(
    settings: &EngineSettings,
    hook: Arc<dyn Fn() + Send + Sync>,
) -> Arc<CoreEngine> {
    let engine = new_core(settings);
    *engine.decided_hook.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    engine
}

/// Starts the render thread of an engine made by [`new_core_with_decided_hook`].
#[cfg(test)]
pub(crate) fn start_core(engine: &Arc<CoreEngine>) {
    start_render_thread(engine);
}

/// Starts the render thread and returns its handle.
pub fn spawn(settings: EngineSettings) -> Arc<dyn Engine> {
    Arc::clone(&spawn_core(settings)) as Arc<dyn Engine>
}

/// The state the render thread keeps between frames, so the hot path allocates
/// nothing per frame.
struct LoopBuffers {
    canvas: Canvas,
    /// One `Vec<Rgb8>` per device in the plan.
    device_colors: Vec<Vec<Rgb8>>,
    /// Scratch buffer for one zone's LED colors.
    zone_colors: Vec<Rgb8>,
    /// Scratch buffer for the preview's packed RGB.
    preview_bytes: Vec<u8>,
}

/// The render thread.
fn render_loop(engine: Arc<CoreEngine>) {
    {
        let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
        state.render_thread_id = Some(platform::current_thread_id());
    }
    let mut timer = FrameTimer::new();
    let mut buffers = LoopBuffers {
        canvas: Canvas::with_scale(1.0),
        device_colors: Vec::new(),
        zone_colors: Vec::new(),
        preview_bytes: Vec::new(),
    };
    let mut plan = LayoutPlan::default();
    let mut plan_generation = u64::MAX;
    let mut sampling_mode = SamplingMode::AreaAverage;
    // Device id -> index into `plan.devices`, rebuilt when the plan changes.
    let mut device_index: std::collections::HashMap<DeviceId, usize> =
        std::collections::HashMap::new();
    let mut last_frame_at = Instant::now();
    let mut last_preview_at = Instant::now();
    // The power state the thread last saw, so the resume can be logged.
    let mut last_power = Power::Running;
    // The effect generation the render thread last ran with, and whether that effect
    // has delivered a good frame. A panicked tick after a good frame delivers nothing:
    // `FrameSink` is a one-way channel into a bounded writer task
    // (`crates/canvas-daemon/src/state.rs` `DeviceSink`), so skipping a submit leaves the
    // device showing what it was last sent (OpenRGB custom mode persists;
    // `crates/canvas-adapters/src/openrgb.rs`; Royuan holds `last_color`,
    // `crates/canvas-adapters/src/royuan.rs`), and the preview's `try_send` leaves the
    // client showing its last frame (`state.rs` `WebsocketPreview`). That makes the check
    // free when a panic never happens: no snapshot buffer, no per-tick copy, only a flag
    // the loop already reads. A panicked tick before this generation's first good frame
    // delivers black instead (#99, findings 1-3).
    //
    // On `set_effect` the generation bumps and this render thread clears the canvas when
    // it picks up the new generation: a new effect never starts from the previous
    // effect's frame (trails accumulate, see findings 3), and a new effect that panics
    // before a good frame shows black, never the previous effect's colours.
    let mut seen_generation = u64::MAX;
    let mut delivered_good_frame = false;
    // The last good colours of every device, kept so a device that needs a frame again
    // gets it (#145): a resume from a pause, a resume from sleep, a sink that was added
    // while the effect was stopped. Without it the delivery stays stopped until another
    // effect is picked, and the lights keep the frame they had (or black, after the
    // pause) while the adapter's keepalive and resume re-send never run
    // (`crates/canvas-adapters/src/openrgb.rs`).
    //
    // The copy is refreshed on every good frame, so it is the last good frame and not an
    // old one. `clone_from` reuses the destination's capacity, so the good path stays
    // allocation-free; the copy is a memcpy of the device's colours, next to nothing
    // compared with the sampling of the whole canvas.
    //
    // It is emptied when the effect is replaced: a new effect never starts from the
    // previous effect's frame (#99, finding 3), and its first frame is the one that is
    // kept.
    let mut last_good: Vec<Vec<Rgb8>> = Vec::new();
    // The device every saved frame belongs to, in the same order as `last_good`. The
    // frames are keyed by the device and not by the position of the device in the
    // plan, so a plan that lists the same devices in another order keeps every
    // device's own frame.
    let mut last_good_ids: Vec<DeviceId> = Vec::new();
    // Scene mode (Scenes, #310): the render thread's canvases, plans and buffers.
    let mut scene_runner = SceneRunner::new();

    loop {
        // ---- read the state --------------------------------------------------
        // `decided_at` is the state's change counter, read under the same lock as the
        // decision below. The park wait compares against it, so a change that lands after
        // the decision (a consumer arriving) is seen by the wait and not slept through.
        let (running, fps, resumed, decided_at) = {
            let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stop {
                break;
            }
            let running = state.power == Power::Running && state.has_consumers();
            let mut resumed = false;
            if running && state.parked {
                state.parked = false;
                resumed = true;
                info!("unparked: a consumer connected");
            } else if !running && !state.parked && state.power == Power::Running {
                state.parked = true;
                info!("parked: no device sinks and no preview");
            }
            // A resume from pause is not a park, so it is logged separately.
            if state.power != last_power {
                if state.power == Power::Paused {
                    info!("paused: the render loop stops after one black frame");
                } else {
                    info!("resumed: the render loop runs again");
                    // Leaving a pause never sets `parked`, so flag the re-anchor here:
                    // the schedule and `dt` restart instead of replaying the pause.
                    if last_power == Power::Paused {
                        resumed = true;
                    }
                }
                last_power = state.power;
            }
            (running, state.fps, resumed, state.wake_generation)
        };

        // A test holds the thread here, between the decision and the park wait, to land a
        // change in that gap (#250).
        #[cfg(test)]
        engine.decided_hook();

        if !running {
            // Parked or paused: sleep until something changes.
            timer.release_resolution();
            let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stop {
                break;
            }
            // The black frame on the pause transition is written once, here.
            if state.pending_black {
                state.pending_black = false;
                write_black(&state);
                info!("paused: wrote one black frame to every sink");
            }
            // Wait until the state changes. The generation counter closes the race
            // between the decision above and the wait below: `decided_at` was read with
            // the decision, so a change that landed in between has moved the counter past
            // it and the wait returns at once. (It used to read the counter here, after
            // the gap: a consumer that arrived in the gap was already counted, the wait
            // saw no change, and the loop slept until the next unrelated change, #250.)
            let seen = decided_at;
            while state.wake_generation == seen && !state.stop {
                let (guard, _timeout) = engine
                    .wake
                    .wait_timeout(state, PARK_POLL)
                    .unwrap_or_else(|e| e.into_inner());
                state = guard;
                // The count of wake-ups is what a parked engine is measured by (#39, #47).
                engine
                    .park_wakeups
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            drop(state);
            continue;
        }

        // ---- running ---------------------------------------------------------
        timer.acquire_resolution();

        if resumed {
            // The schedule restarts at the resume, so the first frame after a resume
            // is due one interval from now rather than catching up on the ticks that
            // were missed while parked.
            timer.reset_schedule(Instant::now());
            last_frame_at = Instant::now();
        }

        // Scene mode: one effect per group. The layout path below does not run.
        if render_scene_frame(&engine, &mut scene_runner, &mut last_frame_at) {
            timer.sleep_until_next(fps);
            continue;
        }

        // Take the effect out of the state for the duration of the frame, so the API
        // thread is never blocked by a long effect update.
        let (
            effect,
            effect_generation,
            effect_panicked,
            pending_props,
            brightness,
            warmth,
            white_balance,
            preview_on,
            input_source,
            effect_start,
            pending_sink,
            pending_resume,
        ) = {
            let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
            // Pick up a new plan only when the layout actually changed.
            if state.plan_generation != plan_generation {
                plan_generation = state.plan_generation;
                if let Some(p) = state.plan.clone() {
                    sampling_mode = p.sampling;
                    buffers.canvas = Canvas::with_scale(p.scale);
                    device_index.clear();
                    for (i, d) in p.devices.iter().enumerate() {
                        device_index.insert(d.device_id.clone(), i);
                    }
                    plan = p;
                }
                // The saved frames belong to the old plan (#145): they are keyed by
                // the device they were drawn for, so a plan that lists the same
                // devices in another order keeps every device's own frame. A frame
                // of a device that changed its LED count is the wrong length, so it
                // goes.
                remap_last_good(&plan, &mut last_good, &mut last_good_ids);
            }
            // `schema` used to be read here under the lock; it is effect code and
            // runs unguarded below (#99), so it moved below, outside the lock.
            // Props requested while this frame's effect was in flight are applied
            // before the next update.
            let pending_props = if state.props_dirty {
                state.props_dirty = false;
                state.props = state.pending_props.clone();
                Some(state.pending_props.clone())
            } else {
                None
            };
            (
                state.effect.take(),
                state.effect_generation,
                state.effect_panicked,
                pending_props,
                state.brightness,
                state.warmth,
                Arc::clone(&state.white_balance),
                state.preview.is_some(),
                state.input_source.clone(),
                state.effect_started,
                state.pending_sink,
                state.pending_resume,
            )
        };

        // A new effect starts from a black canvas, never the previous effect's frame
        // or a half-drawn torn canvas (finding 3). The generation only changes in
        // `set_effect`, which also bumps it for an effect that panics in `init` on the
        // caller thread, so a replacement that panics before a good frame shows black.
        if effect_generation != seen_generation {
            seen_generation = effect_generation;
            delivered_good_frame = false;
            // The saved frames belong to the previous effect (#145): the new effect's
            // first good frame is the one that is kept, never the old one.
            for saved in last_good.iter_mut() {
                saved.clear();
            }
            for pixel in buffers.canvas.pixels.iter_mut() {
                *pixel = [0.0, 0.0, 0.0];
            }
        }

        // ---- draw ------------------------------------------------------------
        let frame_start = Instant::now();
        let dt = frame_start.duration_since(last_frame_at).as_secs_f32();
        last_frame_at = frame_start;

        // Every effect call runs behind a guard (#99): a panicking effect is logged
        // once per instance and skipped until `set_effect` replaces it, and the
        // canvas below keeps the last good frame (black before the first one).
        let mut panicked = effect_panicked;
        let mut needs = InputNeeds::default();
        if let Some(e) = effect.as_ref() {
            if !panicked {
                let outcome = guarded_call(|| e.schema().needs);
                match outcome {
                    Ok(needs_now) => needs = needs_now,
                    Err(payload) => {
                        panicked = true;
                        report_effect_panic(e.as_ref(), "schema", &payload);
                        engine
                            .effect_panics
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        }

        let inputs = build_inputs(needs, &input_source, effect_start);

        if let Some(mut e) = effect {
            if !panicked {
                let mut step_panicked = false;
                if let Some(props) = pending_props {
                    let outcome = guarded_call(|| e.init(&props));
                    if let Err(payload) = outcome {
                        step_panicked = true;
                        report_effect_panic(e.as_ref(), "init", &payload);
                        engine
                            .effect_panics
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                if !step_panicked {
                    let canvas = &mut buffers.canvas;
                    let inputs_ref = &inputs;
                    let outcome = guarded_call(|| {
                        e.update(dt, &mut *canvas, inputs_ref);
                    });
                    if let Err(payload) = outcome {
                        step_panicked = true;
                        report_effect_panic(e.as_ref(), "update", &payload);
                        engine
                            .effect_panics
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                panicked = step_panicked || panicked;
            }
            // Put the effect back, unless a newer one was activated while this one
            // was drawing; in that case the newer one wins. The drop of a superseded
            // effect is effect code too: guard it so a panicking `drop` cannot kill
            // the render thread.
            let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.effect.is_none() && state.effect_generation == effect_generation {
                state.effect = Some(e);
                state.effect_panicked = panicked;
            } else {
                drop(state);
                let _ = guarded_call(|| drop(e));
            }
        }

        // ---- sample ----------------------------------------------------------
        // A panicked tick samples nothing: the canvas is half-drawn. It re-delivers
        // the last good colours per device (black before the first good frame).
        let brightness_factor = f32::from(brightness) / 100.0;

        buffers.device_colors.clear();
        buffers.device_colors.resize(plan.devices.len(), Vec::new());
        if panicked {
            if !delivered_good_frame {
                // No good frame yet for this generation: black, never a leftover.
                for (device_index, device) in plan.devices.iter().enumerate() {
                    let device_colors = &mut buffers.device_colors[device_index];
                    device_colors.clear();
                    device_colors.resize(device.led_count as usize, Rgb8::BLACK);
                }
            }
        } else {
        for (device_index, device) in plan.devices.iter().enumerate() {
            let device_colors = &mut buffers.device_colors[device_index];
            device_colors.clear();
            for zone in &device.zones {
                if !zone.placed {
                    // An unplaced or invisible zone is black, never sampled.
                    device_colors.resize(device_colors.len() + zone.leds.len(), Rgb8::BLACK);
                    continue;
                }
                sampling::sample_leds(
                    &buffers.canvas.pixels,
                    buffers.canvas.width,
                    buffers.canvas.height,
                    &zone.leds,
                    sampling_mode,
                    &mut buffers.zone_colors,
                );
                device_colors.extend_from_slice(&buffers.zone_colors);
            }
            // A frame has exactly one color per LED of the device. The plan's LED
            // count comes from the device topology, so a zone that changed size
            // between the layout and the device list cannot shorten the frame.
            device_colors.resize(device.led_count as usize, Rgb8::BLACK);
            // Apply the brightness, the warmth stage and this device's white balance
            // (#50) to the sampled colors, as one gain per channel.
            let balance = white_balance.get(&device.device_id).copied().unwrap_or([1.0; 3]);
            let gain = [0, 1, 2].map(|i| brightness_factor * warmth[i] * balance[i]);
            if gain.iter().any(|g| *g < 1.0) {
                for c in device_colors.iter_mut() {
                    let f = c.to_f32();
                    *c = Rgb8::from_f32([f[0] * gain[0], f[1] * gain[1], f[2] * gain[2]]);
                }
            }
            // Keep this device's last good colours (#145): a device that needs a frame
            // again (a resume, a sink that was added) gets them. `clone_from` reuses the
            // destination's capacity, so the good path allocates nothing.
            last_good[device_index].clone_from(device_colors);
        }
        delivered_good_frame = true;
        }

        // A panicked tick after a good frame delivers nothing (see `delivered_good_frame`
        // at the top of the loop): the lights keep the last frame they were sent, and the
        // preview client keeps the last frame it received. Black before a good frame is
        // the only panicked delivery.
        let deliver = !panicked || !delivered_good_frame;
        // Something needs a frame again while the delivery is stopped (#145): a resume
        // from a pause, a resume from sleep, or a sink that was added. The last good
        // frame goes out then, so a device is never left on a stale or black frame, and
        // the adapter's keepalive and resume re-send run again
        // (`crates/canvas-adapters/src/openrgb.rs`).
        let redeliver = !deliver && (pending_resume || pending_sink);
        if !deliver {
            // Skip to bookkeeping: no sink submit, no preview submit.
            if redeliver {
                let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
                redeliver_last_good(&state, &last_good, &device_index);
                state.pending_sink = false;
                state.pending_resume = false;
            }
        } else {
        // ---- deliver ---------------------------------------------------------
        {
            let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
            for sink in &state.sinks {
                // The sink takes ownership of the colors, so the buffer is moved out
                // rather than copied. `FrameSink::submit` takes a `Vec`, so one
                // allocation per device per frame is inherent to the trait.
                let colors = match device_index.get(&sink.device_id).copied() {
                    Some(i) => std::mem::replace(
                        &mut buffers.device_colors[i],
                        Vec::with_capacity(sink.led_count as usize),
                    ),
                    // A sink can arrive a frame before the plan that knows its device:
                    // it gets black, at its full length, never an empty frame.
                    None => vec![Rgb8::BLACK; sink.led_count as usize],
                };
                // A device with no known LEDs (its layout has not arrived) gets nothing: an empty
                // frame is rejected by every adapter and only fills the log.
                if colors.is_empty() {
                    continue;
                }
                // The frame the device is sent now is the frame a fade starts from, in
                // layout mode too: a fade that follows layout mode blends from what the
                // strip is showing, not from the scene that ended before it.
                scene_runner.record_sent(&sink.device_id, &colors);
                sink.sink.submit(colors);
            }
            // Every sink has this frame now, the new ones included.
            state.pending_sink = false;
            state.pending_resume = false;
        }

        // ---- preview ---------------------------------------------------------
        if preview_on {
            let state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(sink) = state.preview.clone() {
                // Rate-limit the preview to PREVIEW_MAX_FPS.
                let interval = Duration::from_nanos(1_000_000_000 / u64::from(PREVIEW_MAX_FPS));
                if frame_start.duration_since(last_preview_at) >= interval {
                    // A panicked tick reaches here only before a good frame, and then
                    // there is nothing to show: skip the preview rather than post a
                    // black downscale.
                    if !panicked {
                        let (w, h) = preview::downscale_rgb(
                            &buffers.canvas.pixels,
                            buffers.canvas.width,
                            buffers.canvas.height,
                            &mut buffers.preview_bytes,
                        );
                        sink.submit(w, h, buffers.preview_bytes.clone());
                        last_preview_at = frame_start;
                    }
                }
            }
        }
        }

        // ---- bookkeeping -----------------------------------------------------
        finish_frame(&engine, frame_start);

        // ---- sleep to the next tick ------------------------------------------
        timer.sleep_until_next(fps);
    }

    info!("render thread exiting");
}

/// The bookkeeping that ends every frame, layout mode and scene mode alike.
fn finish_frame(engine: &CoreEngine, frame_start: Instant) {
    let frame_time_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
    {
        let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
        state.frames_rendered += 1;
        state.frame_time_ms = frame_time_ms;
        state.record_frame(frame_start);
    }
    #[cfg(test)]
    {
        let hook = engine.frame_time_hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(hook) = hook {
            hook(frame_time_ms);
        }
    }
}

/// One frame of scene mode (Scenes, #310). Returns false, having done nothing, when the engine
/// is in layout mode.
///
/// The lock is held to take each group's effect out and to put it back, never while an effect
/// runs: the API threads (`set_group_props`, `set_group_effect`) are never blocked by a long
/// update. The per-group work is in `scene_engine`.
fn render_scene_frame(
    engine: &CoreEngine,
    runner: &mut SceneRunner,
    last_frame_at: &mut Instant,
) -> bool {
    let frame_start = Instant::now();
    let (input, params) = {
        let mut guard = engine.state.lock().unwrap_or_else(|e| e.into_inner());
        let state = &mut *guard;
        let Some(scene) = state.scene.as_mut() else {
            return false;
        };
        let input = scene.take_input(&state.devices, runner.generation());
        let dt = frame_start.duration_since(*last_frame_at).as_secs_f32();
        *last_frame_at = frame_start;
        let params = FrameParams {
            dt,
            now: frame_start,
            brightness: f32::from(state.brightness) / 100.0,
            warmth: state.warmth,
            white_balance: Arc::clone(&state.white_balance),
            input_source: state.input_source.clone(),
            want_light_preview: state.light_preview.is_some(),
        };
        (input, params)
    };

    // The groups run, one after the other, with no lock held.
    let output = runner.render(input, &params);
    if output.panics > 0 {
        engine.effect_panics.fetch_add(output.panics, std::sync::atomic::Ordering::Relaxed);
    }

    // The effects go back, the frames go out.
    let (superseded, light_sink) = {
        let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
        let superseded = match state.scene.as_mut() {
            Some(scene) => scene.put_back(output.done),
            // The scene ended while the frame ran (`set_layout`): the effects have nowhere
            // to go back to. They are dropped behind the guard, outside the lock, like
            // every other replaced effect: an effect's `Drop` is effect code.
            None => output.done.into_iter().filter_map(|item| item.effect).collect(),
        };
        // `set_layout` ended scene mode during the frame: the frame is stale, nothing is sent.
        if state.scene.is_some() {
            for sink in &state.sinks {
                // A frame the runner built for another LED count than the sink has now is
                // dropped, never sent: a resize lands between two frames, not inside one.
                let Some(colors) = runner.take_frame(&sink.device_id, sink.led_count as usize)
                else {
                    // The device is not in the scene at all: the sink gets a black frame of
                    // its own length, never nothing, so a strip whose device dropped goes
                    // dark instead of holding the colours it was last sent. Layout mode
                    // does the same for a sink whose device the plan does not know.
                    if !runner.knows_device(&sink.device_id) && sink.led_count > 0 {
                        sink.sink.submit(vec![Rgb8::BLACK; sink.led_count as usize]);
                    }
                    continue;
                };
                if !colors.is_empty() {
                    // The frame the device is sent now is the frame a fade starts from.
                    runner.record_sent(&sink.device_id, &colors);
                    sink.sink.submit(colors);
                }
            }
            // Every sink has had a frame of the scene, the new ones included.
            state.pending_sink = false;
            state.pending_resume = false;
        }
        (superseded, state.light_preview.clone())
    };
    engine.drop_effects(superseded);
    if let (Some(sink), Some(frame)) = (light_sink, output.light_frame) {
        sink.submit(frame);
    }

    finish_frame(engine, frame_start);
    true
}

/// Moves the saved frames onto the new plan (#145).
///
/// The frames are keyed by the device they were drawn for, so a plan that lists the
/// same devices in another order keeps every device's own frame: without this, a
/// re-layout that swapped two devices of the same LED count sent each device the
/// other's frame.
///
/// A frame whose device left the plan is dropped, and so is one whose device changed
/// its LED count: it is the wrong length for the device now.
fn remap_last_good(
    plan: &LayoutPlan,
    last_good: &mut Vec<Vec<Rgb8>>,
    last_good_ids: &mut Vec<DeviceId>,
) {
    let mut ids: Vec<DeviceId> = Vec::with_capacity(plan.devices.len());
    let mut frames: Vec<Vec<Rgb8>> = Vec::with_capacity(plan.devices.len());
    for device in &plan.devices {
        let mut frame = Vec::new();
        let old = last_good_ids.iter().position(|id| id == &device.device_id);
        if let Some(old) = old {
            if let Some(saved) = last_good.get(old) {
                if saved.len() == device.led_count as usize {
                    frame.clone_from(saved);
                }
            }
        }
        ids.push(device.device_id.clone());
        frames.push(frame);
    }
    *last_good = frames;
    *last_good_ids = ids;
}

/// Writes one black frame to every sink.
fn write_black(state: &State) {
    for sink in state.sinks.iter().filter(|s| s.led_count > 0) {
        sink.sink.submit(vec![Rgb8::BLACK; sink.led_count as usize]);
    }
}

/// Sends the last good frame to every sink again (#145).
///
/// A panicked effect stops the delivery (#99), so a device that needs a frame again (a
/// resume from a pause, a resume from sleep, a sink that was added) would keep the frame
/// it was last sent, or the black frame the pause wrote, until another effect is picked.
/// The last good colours go out instead; black when the effect never drew one.
///
/// The frame is copied here, not moved: the saved copy stays for the next time.
fn redeliver_last_good(
    state: &State,
    last_good: &[Vec<Rgb8>],
    device_index: &std::collections::HashMap<DeviceId, usize>,
) {
    for sink in state.sinks.iter().filter(|s| s.led_count > 0) {
        let led_count = sink.led_count as usize;
        let saved = device_index
            .get(&sink.device_id)
            .copied()
            .and_then(|i| last_good.get(i))
            .filter(|frame| frame.len() == led_count);
        let colors = match saved {
            Some(frame) => frame.clone(),
            // The effect never drew a frame for this device, or the plan changed under
            // it: black, at the device's full length, never an empty frame.
            None => vec![Rgb8::BLACK; led_count],
        };
        sink.sink.submit(colors);
    }
}

/// Builds the effect's `Inputs`, pulling capture data only when the effect asks.
fn build_inputs(
    needs: InputNeeds,
    input_source: &Option<Arc<dyn InputSource>>,
    effect_started: Option<Instant>,
) -> canvas_proto::traits::Inputs {
    let screen = if needs.screen {
        input_source.as_ref().and_then(|s| s.screen())
    } else {
        None
    };
    let audio = if needs.audio {
        input_source.as_ref().and_then(|s| s.audio())
    } else {
        None
    };
    canvas_proto::traits::Inputs {
        time: effect_started
            .map(|s| s.elapsed().as_secs_f64())
            .unwrap_or(0.0),
        screen,
        audio,
    }
}

/// The platform bits the engine needs from the operating system.
mod platform {
    /// The operating-system id of the calling thread.
    pub fn current_thread_id() -> u32 {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::GetCurrentThreadId;
            // SAFETY: `GetCurrentThreadId` takes no arguments and cannot fail.
            unsafe { GetCurrentThreadId() }
        }
        #[cfg(not(windows))]
        {
            0
        }
    }
}

#[cfg(test)]
mod frame_window_tests {
    use super::*;


    fn state() -> State {
        State::new(&EngineSettings { fps: 60, power: Power::Running, brightness: 100, warmth: canvas_proto::consts::WARMTH_OFF })
    }

    #[test]
    fn the_window_keeps_only_the_last_second() {
        let mut s = state();
        let start = Instant::now();
        // Ten seconds of frames at 60 fps.
        for i in 0..600u32 {
            s.record_frame(start + Duration::from_micros(16_667 * u64::from(i)));
        }
        assert!(s.recent.len() <= 62, "window holds {} frames", s.recent.len());
    }

    #[test]
    fn the_measured_rate_follows_a_drop_in_fps() {
        let mut s = state();
        let start = Instant::now();
        let mut t = start;
        for _ in 0..300 {
            s.record_frame(t);
            t += Duration::from_micros(16_667);
        }
        // Then two seconds at 30 fps.
        for _ in 0..60 {
            s.record_frame(t);
            t += Duration::from_micros(33_333);
        }
        let fps = s.measured_fps();
        assert!((29.0..=31.0).contains(&fps), "measured {fps}");
    }
}

#[cfg(test)]
mod power_tests {
    use super::*;

    fn state() -> State {
        State::new(&EngineSettings {
            fps: 60,
            power: Power::Running,
            brightness: 100,
            warmth: canvas_proto::consts::WARMTH_OFF,
        })
    }

    /// #145: a pause and a resume that both land between two ticks leave no black
    /// request behind.
    ///
    /// The render thread never saw the pause, so the black frame of that pause was
    /// never written. The request is stale: it must not black out a device that
    /// arrives later.
    #[test]
    fn a_pause_and_a_resume_between_two_ticks_leave_no_black_request() {
        let mut s = state();
        s.apply_power(Power::Paused);
        assert!(s.pending_black, "the pause asks for one black frame");
        s.apply_power(Power::Running);
        assert!(
            !s.pending_black,
            "the resume drops the black request of that pause"
        );
        assert!(
            s.pending_resume,
            "the resume asks for the last good frame again"
        );
    }

    /// A pause the render thread did see keeps its black request until the frame is
    /// written, and the resume after that asks for the last good frame again.
    #[test]
    fn a_pause_keeps_its_black_request_until_the_frame_is_written() {
        let mut s = state();
        s.apply_power(Power::Paused);
        assert!(s.pending_black, "the pause asks for one black frame");
        // The render thread wrote the black frame.
        s.pending_black = false;
        s.apply_power(Power::Running);
        assert!(
            s.pending_resume,
            "the resume asks for the last good frame again"
        );
        assert!(!s.pending_black, "the resume asks for no black frame");
    }
}
