//! The render loop: one thread, a fixed-step timer, and the state the loop owns.
//!
//! Threading model:
//! - The public `Engine` handle owns a `Mutex<State>` and a `Condvar`.
//! - A single background thread does the rendering. It waits on the condvar when it
//!   has nothing to do (parked or paused) and sleeps until the next tick when it is
//!   running.
//! - Callers (`set_fps`, `set_layout`, ...) mutate the state under the lock and nudge
//!   the condvar so the thread reacts immediately.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use canvas_proto::api::Power;
use canvas_proto::color::Rgb8;
use canvas_proto::device::{DeviceId, DeviceInfo};
use canvas_proto::layout::{Layout, SamplingMode};
use canvas_proto::props::{InputNeeds, Props};
use canvas_proto::traits::{
    Canvas, Effect, Engine, EngineSettings, FrameSink, InputSource, PreviewSink, RenderStats,
};
use tracing::{info, warn};

use crate::geometry::{self, LayoutPlan};
use crate::preview;
use crate::sampling;
use crate::timer::FrameTimer;

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
    /// The active layout's precomputed sample plan.
    plan: Option<LayoutPlan>,
    /// Bumped whenever `plan` is replaced, so the render thread knows to pick it up.
    plan_generation: u64,
    /// Device topology, so the plan can be rebuilt when a zone changes size.
    devices: Vec<DeviceInfo>,
    /// Sinks, in the order their devices appear in the plan.
    sinks: Vec<SinkEntry>,
    preview: Option<Arc<dyn PreviewSink>>,
    input_source: Option<Arc<dyn InputSource>>,
    /// The active effect. `None` means "paint nothing" (a black canvas).
    effect: Option<Box<dyn Effect>>,
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
            plan: None,
            plan_generation: 0,
            devices: Vec::new(),
            sinks: Vec::new(),
            preview: None,
            input_source: None,
            effect: None,
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
            wake_generation: 0,
            render_thread_id: None,
        }
    }

    /// Whether anything consumes frames.
    fn has_consumers(&self) -> bool {
        !self.sinks.is_empty() || self.preview.is_some()
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
}

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
}

impl CoreEngine {
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
}

impl Engine for CoreEngine {
    fn set_layout(&self, layout: Layout, devices: Vec<DeviceInfo>) {
        let sampling = layout.sampling;
        let scale_percent = layout.render_scale.percent();
        let plan = geometry::build_plan(&layout, &devices);
        let summary = plan.summary();
        let device_count = plan.device_count();
        self.mutate(|state| {
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
        });
        info!(
            devices = device_count,
            ?sampling,
            scale = scale_percent,
            "layout applied; sample regions recomputed: {summary}"
        );
    }

    fn set_effect(&self, effect: Box<dyn Effect>, props: &Props) {
        let mut effect = effect;
        effect.init(props);
        self.mutate(|state| {
            state.effect = Some(effect);
            state.effect_generation = state.effect_generation.wrapping_add(1);
            state.effect_started = Some(Instant::now());
            state.props = props.clone();
            state.pending_props = props.clone();
            state.props_dirty = false;
        });
        info!("effect activated");
    }

    fn set_props(&self, props: &Props) {
        // The effect may be in the render thread's hands right now, so the new props
        // are queued and applied at the start of the next frame.
        self.mutate(|state| {
            state.pending_props = props.clone();
            state.props_dirty = true;
            // If the effect is idle in the state, initialise it right away; there is
            // no frame in flight to race with.
            if let Some(effect) = state.effect.as_mut() {
                effect.init(props);
                state.props = props.clone();
                state.props_dirty = false;
            }
        });
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
        self.mutate(|state| {
            if state.power == power {
                return;
            }
            state.power = power;
            if power == Power::Paused {
                // "Paused writes black once, then stops the render loop."
                state.pending_black = true;
            }
        });
        info!(?power, "power changed");
    }

    fn set_brightness(&self, brightness: u8) {
        let clamped = brightness.min(100);
        self.mutate(|state| state.brightness = clamped);
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
    let engine = Arc::new(CoreEngine {
        state: Mutex::new(State::new(&settings)),
        wake: Condvar::new(),
        thread: Mutex::new(None),
        park_wakeups: std::sync::atomic::AtomicU64::new(0),
    });

    let thread_engine = Arc::clone(&engine);
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

    engine
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

    loop {
        // ---- read the state --------------------------------------------------
        let (running, fps, resumed) = {
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
            (running, state.fps, resumed)
        };

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
            // between the check above and the wait below: if a change landed in
            // between, the counter moved and the wait returns immediately.
            let seen = state.wake_generation;
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

        // Take the effect out of the state for the duration of the frame, so the API
        // thread is never blocked by a long effect update.
        let (effect, effect_generation, pending_props, brightness, preview_on, needs, input_source, effect_start) = {
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
            }
            let needs = state.effect.as_ref().map(|e| e.schema().needs).unwrap_or_default();
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
                pending_props,
                state.brightness,
                state.preview.is_some(),
                needs,
                state.input_source.clone(),
                state.effect_started,
            )
        };

        // ---- draw ------------------------------------------------------------
        let frame_start = Instant::now();
        let dt = frame_start.duration_since(last_frame_at).as_secs_f32();
        last_frame_at = frame_start;

        let inputs = build_inputs(needs, &input_source, effect_start);

        if let Some(mut e) = effect {
            if let Some(props) = pending_props {
                e.init(&props);
            }
            e.update(dt, &mut buffers.canvas, &inputs);
            // Put the effect back, unless a newer one was activated while this one
            // was drawing; in that case the newer one wins.
            let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.effect.is_none() && state.effect_generation == effect_generation {
                state.effect = Some(e);
            }
        }

        // ---- sample ----------------------------------------------------------
        let brightness_factor = f32::from(brightness) / 100.0;

        buffers.device_colors.clear();
        buffers.device_colors.resize(plan.devices.len(), Vec::new());
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
            // Apply the brightness to the sampled colors.
            if brightness_factor < 1.0 {
                for c in device_colors.iter_mut() {
                    let f = c.to_f32();
                    *c = Rgb8::from_f32([
                        f[0] * brightness_factor,
                        f[1] * brightness_factor,
                        f[2] * brightness_factor,
                    ]);
                }
            }
        }

        // ---- deliver ---------------------------------------------------------
        {
            let state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
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
                sink.sink.submit(colors);
            }
        }

        // ---- preview ---------------------------------------------------------
        if preview_on {
            let state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(sink) = state.preview.clone() {
                // Rate-limit the preview to PREVIEW_MAX_FPS.
                let interval = Duration::from_nanos(1_000_000_000 / u64::from(PREVIEW_MAX_FPS));
                if frame_start.duration_since(last_preview_at) >= interval {
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

        // ---- bookkeeping -----------------------------------------------------
        {
            let mut state = engine.state.lock().unwrap_or_else(|e| e.into_inner());
            state.frames_rendered += 1;
            state.frame_time_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
            state.record_frame(frame_start);
        }

        // ---- sleep to the next tick ------------------------------------------
        timer.sleep_until_next(fps);
    }

    info!("render thread exiting");
}

/// Writes one black frame to every sink.
fn write_black(state: &State) {
    for sink in state.sinks.iter().filter(|s| s.led_count > 0) {
        sink.sink.submit(vec![Rgb8::BLACK; sink.led_count as usize]);
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
        State::new(&EngineSettings { fps: 60, power: Power::Running, brightness: 100 })
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
