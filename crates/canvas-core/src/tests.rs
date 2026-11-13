//! Integration tests for the render loop.
//!
//! These use their own tiny test effects (never the `canvas-effects` crate) and a
//! fake sink, so they never touch hardware.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canvas_proto::api::Power;
use canvas_proto::color::Rgb8;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::{Layout, LedSlice, PlacedZone, RenderScale, SamplingMode, ZoneShape};
use canvas_proto::props::{InputNeeds, PropKind, PropertyDef, PropertySchema, Props};
use canvas_proto::traits::{Canvas, Effect, EngineSettings, FrameSink, PreviewSink};
use canvas_proto::CANVAS_HEIGHT;
use canvas_proto::CANVAS_WIDTH;

use crate::spawn;

/// The deadline for a first frame to arrive (#143, class `cause:timing-test`; same shape as
/// #59, which raised the web waits to 10 s). The waits stay event-based: `wait_until`
/// returns on the first frame, so a passing run costs nothing; 20 s is only the ceiling a
/// starved runner gets. No assertion here measures time, so this never widens a measured
/// budget.
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(20);

/// A sink that records every frame it receives.
#[derive(Default)]
struct RecordingSink {
    frames: Mutex<Vec<Vec<Rgb8>>>,
    count: AtomicU32,
}

impl RecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn last(&self) -> Option<Vec<Rgb8>> {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).last().cloned()
    }

    /// Every frame the sink received, oldest first.
    fn all_frames(&self) -> Vec<Vec<Rgb8>> {
        self.frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn frame_count(&self) -> u32 {
        self.count.load(Ordering::Acquire)
    }
}

impl FrameSink for RecordingSink {
    fn submit(&self, colors: Vec<Rgb8>) {
        self.count.fetch_add(1, Ordering::AcqRel);
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).push(colors);
    }
}

/// A preview sink that records every frame.
#[derive(Default)]
struct RecordingPreview {
    frames: Mutex<Vec<(u32, u32, usize)>>,
}

impl RecordingPreview {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn count(&self) -> usize {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

impl PreviewSink for RecordingPreview {
    fn submit(&self, width: u32, height: u32, rgb: Vec<u8>) {
        self.frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((width, height, rgb.len()));
    }
}

/// An effect that paints the whole canvas one color.
struct SolidEffect {
    color: [f32; 3],
    initialized: Arc<AtomicBool>,
}

impl SolidEffect {
    fn new(color: [f32; 3], initialized: Arc<AtomicBool>) -> Self {
        SolidEffect { color, initialized }
    }
}

impl Effect for SolidEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_solid".into(),
            name: "Test Solid".into(),
            description: "Paints the canvas one color.".into(),
            needs: InputNeeds::default(),
            properties: vec![PropertyDef {
                key: "color".into(),
                label: "Color".into(),
                kind: PropKind::Color { default: Rgb8::WHITE },
            }],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {
        self.initialized.store(true, Ordering::Release);
    }

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        canvas.fill(self.color);
    }
}

/// An effect that reports the `dt` and `time` it was given.
struct TimingEffect {
    dts: Arc<Mutex<Vec<f32>>>,
    times: Arc<Mutex<Vec<f64>>>,
}

impl TimingEffect {
    fn new(dts: Arc<Mutex<Vec<f32>>>, times: Arc<Mutex<Vec<f64>>>) -> Self {
        TimingEffect { dts, times }
    }
}

impl Effect for TimingEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_timing".into(),
            name: "Test Timing".into(),
            description: "Records dt and time.".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, dt: f32, _canvas: &mut Canvas, inputs: &canvas_proto::traits::Inputs) {
        self.dts.lock().unwrap_or_else(|e| e.into_inner()).push(dt);
        self.times.lock().unwrap_or_else(|e| e.into_inner()).push(inputs.time);
    }
}

/// An effect that counts how many times it was initialised.
struct CountingEffect {
    inits: Arc<AtomicU32>,
}

impl Effect for CountingEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_counting".into(),
            name: "Test Counting".into(),
            description: "Counts init calls.".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {
        self.inits.fetch_add(1, Ordering::AcqRel);
    }

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        canvas.fill([0.0, 0.0, 0.0]);
    }
}

/// An effect that paints a horizontal gradient: red at x=0, blue at the right edge.
struct GradientEffect;

impl Effect for GradientEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_gradient".into(),
            name: "Test Gradient".into(),
            description: "A horizontal red-to-blue gradient.".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        let w = canvas.width;
        for y in 0..canvas.height {
            for x in 0..w {
                let t = if w > 1 { x as f32 / (w - 1) as f32 } else { 0.0 };
                canvas.set(x, y, [1.0 - t, 0.0, t]);
            }
        }
    }
}

/// An input source that counts how often it was asked for capture data.
struct CountingInputSource {
    screen_calls: Arc<AtomicU32>,
    audio_calls: Arc<AtomicU32>,
}

impl CountingInputSource {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            screen_calls: Arc::new(AtomicU32::new(0)),
            audio_calls: Arc::new(AtomicU32::new(0)),
        })
    }
}

impl canvas_proto::traits::InputSource for CountingInputSource {
    fn screen(&self) -> Option<Arc<canvas_proto::traits::ScreenFrame>> {
        self.screen_calls.fetch_add(1, Ordering::AcqRel);
        None
    }

    fn audio(&self) -> Option<Arc<canvas_proto::traits::AudioSpectrum>> {
        self.audio_calls.fetch_add(1, Ordering::AcqRel);
        None
    }
}

/// An effect that declares that it needs screen and audio capture.
struct NeedyEffect;

impl Effect for NeedyEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_needy".into(),
            name: "Test Needy".into(),
            description: "Wants screen and audio.".into(),
            needs: InputNeeds {
                screen: true,
                audio: true,
            },
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        canvas.fill([0.0, 0.0, 0.0]);
    }
}

fn zone(index: u32, kind: ZoneKind, led_count: u32) -> ZoneInfo {
    ZoneInfo {
        index,
        name: format!("zone{index}"),
        kind,
        led_count,
        min_leds: led_count,
        max_leds: led_count,
        resizable: false,
    }
}

fn device(id: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.to_string()),
        name: id.to_string(),
        vendor: "test".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones,
        connected: true,
    }
}

fn placed(device_id: &str, zone_index: u32, x: f32, y: f32, w: f32, h: f32) -> PlacedZone {
    PlacedZone {
        device_id: DeviceId(device_id.to_string()),
        zone_index,
        x,
        y,
        width: w,
        height: h,
        rotation_deg: 0.0,
        reversed: false,
        visible: true,
        shape: ZoneShape::Auto,
        label: None,
        leds: None,
        part: None,
        locked: false,
    }
}

fn layout(zones: Vec<PlacedZone>, sampling: SamplingMode, scale: RenderScale) -> Layout {
    Layout {
        id: "test".into(),
        name: "test".into(),
        render_scale: scale,
        sampling,
        zones,
        effect: None,
        case_form: canvas_proto::layout::CaseForm::None,
        landmarks: Vec::new(),
    }
}

fn settings(fps: u32) -> EngineSettings {
    EngineSettings {
        fps,
        power: Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    }
}

/// Waits until `check` is true, or the timeout expires. Returns whether it became true.
fn wait_until(timeout: Duration, check: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    check()
}

/// A layout that knows device `d0` (4 LEDs), so a sink on it gets real frames. Frames for a
/// device the engine has never heard of are skipped, not sent empty.
fn give_d0_a_layout(engine: &Arc<dyn canvas_proto::traits::Engine>) {
    let dev = device("d0", vec![zone(0, ZoneKind::Linear, 4)]);
    let lay = layout(
        vec![placed("d0", 0, 0.0, 0.0, 320.0, 200.0)],
        SamplingMode::AreaAverage,
        RenderScale::P100,
    );
    engine.set_layout(lay, vec![dev]);
}

// ---------------------------------------------------------------------------
// Frames reach the hardware
// ---------------------------------------------------------------------------

#[test]
fn a_solid_red_effect_yields_all_red_frames_for_all_placed_leds() {
    let engine = spawn(settings(30));
    let dev = device("d0", vec![zone(0, ZoneKind::Linear, 12)]);
    let lay = layout(
        vec![placed("d0", 0, 0.0, 0.0, 320.0, 200.0)],
        SamplingMode::AreaAverage,
        RenderScale::P100,
    );
    engine.set_layout(lay, vec![dev]);

    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let initialized = Arc::new(AtomicBool::new(false));
    engine.set_effect(
        Box::new(SolidEffect::new([1.0, 0.0, 0.0], initialized.clone())),
        &Props::new(),
    );

    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));

    let frame = sink.last().expect("a frame was submitted");
    assert_eq!(frame.len(), 12, "a frame has one color per LED");
    for (i, c) in frame.iter().enumerate() {
        assert_eq!(*c, Rgb8::new(255, 0, 0), "LED {i} should be red");
    }
    assert!(initialized.load(Ordering::Acquire), "init was called");
    engine.shutdown();
}

#[test]
fn unplaced_zones_get_black_but_placed_ones_do_not() {
    let engine = spawn(settings(30));
    // Two zones on one device; only zone 0 is placed.
    let dev = device(
        "d0",
        vec![zone(0, ZoneKind::Linear, 4), zone(1, ZoneKind::Linear, 6)],
    );
    let lay = layout(
        vec![placed("d0", 0, 0.0, 0.0, 320.0, 100.0)],
        SamplingMode::Nearest,
        RenderScale::P100,
    );
    engine.set_layout(lay, vec![dev]);

    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let initialized = Arc::new(AtomicBool::new(false));
    engine.set_effect(
        Box::new(SolidEffect::new([1.0, 0.0, 0.0], initialized)),
        &Props::new(),
    );

    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));
    let frame = sink.last().expect("a frame was submitted");
    assert_eq!(frame.len(), 10, "the frame covers every LED of the device");
    for c in frame.iter().take(4) {
        assert_eq!(*c, Rgb8::new(255, 0, 0));
    }
    for c in frame.iter().skip(4) {
        assert_eq!(*c, Rgb8::BLACK, "unplaced LEDs are black");
    }
    engine.shutdown();
}

/// #250: a consumer that arrives while the render thread is between deciding to park and
/// waiting is not slept through.
///
/// The thread decides "nothing consumes frames, park", then takes the lock again to wait. A
/// change in that gap (a sink, a layout, an effect) bumps the engine's change counter. The wait
/// used to read the counter after the gap, so it counted the change as already seen and slept
/// until the next unrelated change: a `capture_wiring` run on CI waited 10 s for a frame that
/// the thread never drew. The hook parks the thread inside the gap, the test makes the change,
/// and the thread must draw.
#[test]
fn a_consumer_that_arrives_between_the_park_decision_and_the_wait_is_not_slept_through() {
    use std::sync::mpsc;

    let (reached_tx, reached_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let reached_tx = Mutex::new(reached_tx);
    let release_rx = Mutex::new(release_rx);
    // Only the first decision is held: that is the one made with no consumer.
    let held = AtomicBool::new(false);
    let hook: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if !held.swap(true, Ordering::AcqRel) {
            let _ = reached_tx.lock().unwrap_or_else(|e| e.into_inner()).send(());
            let _ = release_rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
        }
    });
    let core = crate::engine::new_core_with_decided_hook(&settings(30), hook);
    let engine: Arc<dyn canvas_proto::traits::Engine> = core.clone();
    crate::engine::start_core(&core);

    // The thread has decided to park, and has not waited yet.
    reached_rx
        .recv_timeout(FIRST_FRAME_WAIT)
        .expect("the render thread made its first decision");

    // The consumer arrives in the gap.
    give_d0_a_layout(&engine);
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    release_tx.send(()).expect("the thread is waiting at the hook");

    assert!(
        wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0),
        "the sink that arrived between the decision and the wait was never given a frame"
    );
    engine.shutdown();
}

#[test]
fn a_frame_has_one_color_per_led_in_zone_order() {
    let engine = spawn(settings(30));
    let dev = device(
        "d0",
        vec![zone(0, ZoneKind::Linear, 3), zone(1, ZoneKind::Linear, 5)],
    );
    let lay = layout(
        vec![
            placed("d0", 0, 0.0, 0.0, 100.0, 100.0),
            placed("d0", 1, 100.0, 0.0, 100.0, 100.0),
        ],
        SamplingMode::Nearest,
        RenderScale::P100,
    );
    engine.set_layout(lay, vec![dev]);
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    engine.set_effect(
        Box::new(SolidEffect::new(
            [0.0, 1.0, 0.0],
            Arc::new(AtomicBool::new(false)),
        )),
        &Props::new(),
    );
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));
    let frame = sink.last().expect("a frame was submitted");
    assert_eq!(frame.len(), 8);
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// Parking
// ---------------------------------------------------------------------------

#[test]
fn the_loop_parks_with_no_devices_and_no_preview() {
    let engine = spawn(settings(30));
    // Give the thread time to settle into the parked state.
    std::thread::sleep(Duration::from_millis(300));

    let stats = engine.stats();
    assert!(stats.parked, "the loop should be parked");
    assert_eq!(stats.frames_rendered, 0, "no frames were rendered");
    assert_eq!(stats.sinks, 0);
    assert!(!stats.preview);

    // The frame counter must not grow.
    let before = engine.stats().frames_rendered;
    std::thread::sleep(Duration::from_secs(2));
    let after = engine.stats().frames_rendered;
    assert_eq!(before, after, "no frames are rendered while parked");
    engine.shutdown();
}

#[test]
fn adding_a_sink_unparks_the_loop_immediately() {
    let engine = spawn(settings(30));
    std::thread::sleep(Duration::from_millis(300));
    assert!(engine.stats().parked);

    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    assert!(
        wait_until(Duration::from_secs(2), || engine.stats().frames_rendered > 0),
        "the loop resumed and rendered frames"
    );
    assert!(!engine.stats().parked, "the loop is no longer parked");
    engine.shutdown();
}

#[test]
fn setting_a_preview_unparks_the_loop_immediately() {
    let engine = spawn(settings(30));
    std::thread::sleep(Duration::from_millis(300));
    assert!(engine.stats().parked);

    engine.set_preview(Some(RecordingPreview::new()));
    assert!(
        wait_until(Duration::from_secs(2), || engine.stats().frames_rendered > 0),
        "the loop resumed and rendered frames"
    );
    engine.shutdown();
}

#[test]
fn removing_the_last_sink_parks_the_loop_again() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    assert!(wait_until(Duration::from_secs(2), || engine.stats().frames_rendered > 0));

    engine.remove_sink(&DeviceId("d0".into()));
    assert!(wait_until(Duration::from_secs(2), || engine.stats().parked));
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// Pause
// ---------------------------------------------------------------------------

#[test]
fn pausing_writes_one_black_frame_and_stops_the_loop() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    engine.set_effect(
        Box::new(SolidEffect::new(
            [1.0, 0.0, 0.0],
            Arc::new(AtomicBool::new(false)),
        )),
        &Props::new(),
    );
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));

    engine.set_power(Power::Paused);

    // The black frame arrives.
    assert!(
        wait_until(Duration::from_secs(2), || sink
            .last()
            .map(|f| f.iter().all(|c| *c == Rgb8::BLACK))
            .unwrap_or(false)),
        "a black frame was written"
    );

    // And then the frame counter stops growing.
    let after_black = engine.stats().frames_rendered;
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        engine.stats().frames_rendered,
        after_black,
        "no frames are rendered while paused"
    );
    engine.shutdown();
}

#[test]
fn running_resumes_the_loop_after_a_pause() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    engine.set_effect(
        Box::new(SolidEffect::new(
            [1.0, 0.0, 0.0],
            Arc::new(AtomicBool::new(false)),
        )),
        &Props::new(),
    );
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));

    engine.set_power(Power::Paused);
    assert!(wait_until(Duration::from_secs(2), || sink
        .last()
        .map(|f| f.iter().all(|c| *c == Rgb8::BLACK))
        .unwrap_or(false)));

    engine.set_power(Power::Running);
    let frames_at_resume = sink.frame_count();
    assert!(
        wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > frames_at_resume),
        "frames are submitted again after resuming"
    );
    engine.shutdown();
}

#[test]
fn resuming_from_a_pause_does_not_replay_the_missed_ticks() {
    let engine = spawn(settings(60));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    engine.set_effect(
        Box::new(SolidEffect::new([1.0, 0.0, 0.0], Arc::new(AtomicBool::new(false)))),
        &Props::new(),
    );
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));

    engine.set_power(Power::Paused);
    assert!(wait_until(Duration::from_secs(2), || sink
        .last()
        .map(|f| f.iter().all(|c| *c == Rgb8::BLACK))
        .unwrap_or(false)));
    // About 90 ticks of 60 fps pass while paused.
    std::thread::sleep(Duration::from_millis(1500));

    let before = engine.stats().frames_rendered;
    engine.set_power(Power::Running);
    std::thread::sleep(Duration::from_millis(150));
    let burst = engine.stats().frames_rendered - before;
    // 150 ms at 60 fps is 9 frames; allow slack for scheduling, far below the ~90 replayed.
    assert!(burst <= 15, "{burst} frames rendered in 150 ms after the resume");
    engine.shutdown();
}

#[test]
fn a_sink_added_before_its_layout_never_receives_an_empty_frame() {
    let engine = spawn(settings(60));
    let sink = RecordingSink::new();
    // The device is not in any layout yet, so the engine does not know its LED count.
    engine.add_sink(DeviceId("late".into()), sink.clone());
    engine.set_effect(
        Box::new(SolidEffect::new([1.0, 0.0, 0.0], Arc::new(AtomicBool::new(false)))),
        &Props::new(),
    );
    // Let it render a while with the sink unknown to the plan.
    std::thread::sleep(Duration::from_millis(300));
    let dev = device("late", vec![zone(0, ZoneKind::Linear, 8)]);
    let lay = layout(
        vec![placed("late", 0, 0.0, 0.0, 320.0, 200.0)],
        SamplingMode::AreaAverage,
        RenderScale::P100,
    );
    engine.set_layout(lay, vec![dev]);
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| f.len() == 8)));
    let empties = sink.frames.lock().unwrap().iter().filter(|f| f.is_empty()).count();
    assert_eq!(empties, 0, "{empties} zero-length frames were submitted");
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// FPS
// ---------------------------------------------------------------------------

#[test]
fn set_fps_takes_effect_without_a_restart() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    // Let the measured fps settle at 30.
    assert!(wait_until(FIRST_FRAME_WAIT, || {
        let fps = engine.stats().fps_measured;
        (fps - 30.0).abs() <= 1.0
    }));

    engine.set_fps(20);
    assert!(
        wait_until(FIRST_FRAME_WAIT, || {
            let fps = engine.stats().fps_measured;
            (fps - 20.0).abs() <= 1.0
        }),
        "the measured fps followed the change to 20, got {}",
        engine.stats().fps_measured
    );
    engine.shutdown();
}

#[test]
fn the_measured_fps_is_zero_while_parked() {
    let engine = spawn(settings(30));
    std::thread::sleep(Duration::from_millis(300));
    assert!(engine.stats().parked);
    assert_eq!(engine.stats().fps_measured, 0.0, "no frames, no rate");
    engine.shutdown();
}

#[test]
fn the_measured_fps_is_zero_while_paused() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    assert!(wait_until(Duration::from_secs(2), || engine.stats().fps_measured > 0.0));

    engine.set_power(Power::Paused);
    assert!(wait_until(Duration::from_secs(2), || {
        engine.stats().fps_measured == 0.0
    }));
    engine.shutdown();
}

#[test]
fn the_measured_fps_is_within_one_of_the_target() {
    let engine = spawn(settings(60));
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    assert!(wait_until(FIRST_FRAME_WAIT, || {
        let fps = engine.stats().fps_measured;
        (fps - 60.0).abs() <= 1.0
    }), "measured {}", engine.stats().fps_measured);
    engine.shutdown();
}

#[test]
fn fps_is_clamped_to_the_supported_range() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    // Below the minimum: clamped to 10.
    engine.set_fps(1);
    assert!(wait_until(FIRST_FRAME_WAIT, || {
        let fps = engine.stats().fps_measured;
        (fps - 10.0).abs() <= 1.0
    }), "measured {}", engine.stats().fps_measured);

    // Above the maximum: clamped to 60.
    engine.set_fps(500);
    assert!(wait_until(FIRST_FRAME_WAIT, || {
        let fps = engine.stats().fps_measured;
        (fps - 60.0).abs() <= 1.0
    }), "measured {}", engine.stats().fps_measured);
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// Brightness
// ---------------------------------------------------------------------------

#[test]
fn brightness_scales_the_sampled_colors() {
    let engine = spawn(settings(30));
    let dev = device("d0", vec![zone(0, ZoneKind::Linear, 4)]);
    let lay = layout(
        vec![placed("d0", 0, 0.0, 0.0, 320.0, 200.0)],
        SamplingMode::Nearest,
        RenderScale::P100,
    );
    engine.set_layout(lay, vec![dev]);
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    engine.set_effect(
        Box::new(SolidEffect::new(
            [1.0, 1.0, 1.0],
            Arc::new(AtomicBool::new(false)),
        )),
        &Props::new(),
    );
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));
    assert_eq!(sink.last().expect("a frame")[0], Rgb8::new(255, 255, 255));

    engine.set_brightness(50);
    assert!(wait_until(FIRST_FRAME_WAIT, || sink
        .last()
        .map(|f| f[0] == Rgb8::new(128, 128, 128))
        .unwrap_or(false)));
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// write_solid
// ---------------------------------------------------------------------------

#[test]
fn write_solid_submits_one_solid_frame_to_every_sink() {
    let engine = spawn(settings(30));
    let dev = device("d0", vec![zone(0, ZoneKind::Linear, 5)]);
    let lay = layout(
        vec![placed("d0", 0, 0.0, 0.0, 320.0, 200.0)],
        SamplingMode::Nearest,
        RenderScale::P100,
    );
    engine.set_layout(lay, vec![dev]);
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let before = sink.frame_count();
    engine.write_solid(Rgb8::new(0, 0, 255));
    assert!(wait_until(Duration::from_secs(2), || sink.frame_count() > before));
    let frame = sink.last().expect("a frame");
    assert_eq!(frame.len(), 5);
    assert!(frame.iter().all(|c| *c == Rgb8::new(0, 0, 255)));
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// Effect contract
// ---------------------------------------------------------------------------

#[test]
fn capture_data_is_only_pulled_when_the_effect_needs_it() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let source = CountingInputSource::new();
    engine.set_input_source(Some(source.clone()));

    // The solid effect declares no input needs, so the source is never asked.
    engine.set_effect(
        Box::new(SolidEffect::new(
            [1.0, 0.0, 0.0],
            Arc::new(AtomicBool::new(false)),
        )),
        &Props::new(),
    );
    assert!(wait_until(Duration::from_secs(2), || sink.frame_count() > 5));
    assert_eq!(
        source.screen_calls.load(Ordering::Acquire),
        0,
        "the screen is not pulled for an effect that does not need it"
    );
    assert_eq!(source.audio_calls.load(Ordering::Acquire), 0);

    // The needy effect asks for both.
    engine.set_effect(Box::new(NeedyEffect), &Props::new());
    assert!(wait_until(Duration::from_secs(2), || {
        source.screen_calls.load(Ordering::Acquire) > 3
    }));
    assert!(source.audio_calls.load(Ordering::Acquire) > 3);
    engine.shutdown();
}

#[test]
fn set_props_reinitialises_the_active_effect_while_the_loop_runs() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let inits = Arc::new(AtomicU32::new(0));
    engine.set_effect(
        Box::new(CountingEffect {
            inits: inits.clone(),
        }),
        &Props::new(),
    );
    assert_eq!(inits.load(Ordering::Acquire), 1, "set_effect calls init");

    // The loop is running, so the effect is in the render thread's hands while it
    // draws. The props change must still reach it.
    assert!(wait_until(Duration::from_secs(2), || sink.frame_count() > 2));
    engine.set_props(&Props::new());
    assert!(
        wait_until(Duration::from_secs(2), || inits.load(Ordering::Acquire) >= 2),
        "set_props calls init again, got {}",
        inits.load(Ordering::Acquire)
    );
    engine.shutdown();
}

#[test]
fn set_effect_and_set_props_both_call_init() {
    let engine = spawn(settings(30));
    let inits = Arc::new(AtomicU32::new(0));
    engine.set_effect(
        Box::new(CountingEffect {
            inits: inits.clone(),
        }),
        &Props::new(),
    );
    assert_eq!(inits.load(Ordering::Acquire), 1, "set_effect calls init");

    engine.set_props(&Props::new());
    assert_eq!(inits.load(Ordering::Acquire), 2, "set_props calls init again");
    engine.shutdown();
}

#[test]
fn the_effect_receives_the_real_dt_and_a_monotonic_time() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let dts = Arc::new(Mutex::new(Vec::new()));
    let times = Arc::new(Mutex::new(Vec::new()));
    engine.set_effect(
        Box::new(TimingEffect::new(dts.clone(), times.clone())),
        &Props::new(),
    );

    assert!(wait_until(FIRST_FRAME_WAIT, || {
        dts.lock().unwrap_or_else(|e| e.into_inner()).len() >= 20
    }));

    let dts = dts.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let times = times.lock().unwrap_or_else(|e| e.into_inner()).clone();

    // The first frame after a start or a resume has no predecessor, so its dt is
    // near zero. Every other frame is one interval apart.
    let measured = &dts[1..];
    let mean = measured.iter().sum::<f32>() / measured.len() as f32;
    let expected = 1.0 / 30.0;
    assert!(
        (mean - expected).abs() < 0.01,
        "the mean dt was {mean}, expected about {expected}"
    );
    for dt in measured {
        assert!(*dt > 0.005 && *dt < 0.2, "dt {dt} is out of range");
    }
    // Time is monotonic.
    for pair in times.windows(2) {
        assert!(pair[1] >= pair[0], "time went backwards");
    }
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

#[test]
fn the_preview_is_downscaled_and_rate_limited() {
    let engine = spawn(settings(60));
    let preview = RecordingPreview::new();
    engine.set_preview(Some(preview.clone()));

    assert!(wait_until(FIRST_FRAME_WAIT, || preview.count() > 0));
    let frames = preview.frames.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let (w, h, len) = frames[0];
    assert!(w <= 160 && h <= 100, "the preview is {w}x{h}");
    assert_eq!(len, (w * h * 3) as usize, "the preview carries w*h*3 bytes");

    // At 60 fps for ~2 s the preview must not exceed 15 fps.
    std::thread::sleep(Duration::from_secs(2));
    let count = preview.count();
    assert!(count <= 35, "the preview sent {count} frames in ~2 s, too many");
    engine.shutdown();
}

#[test]
fn the_preview_stops_when_the_subscriber_goes_away() {
    let engine = spawn(settings(30));
    let preview = RecordingPreview::new();
    engine.set_preview(Some(preview.clone()));
    assert!(wait_until(FIRST_FRAME_WAIT, || preview.count() > 0));

    engine.set_preview(None);
    let count = preview.count();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(preview.count(), count, "no more preview frames arrive");
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// Sampling modes and render scale
// ---------------------------------------------------------------------------

/// Runs the engine with one zone of `led_count` LEDs across the full canvas width,
/// a gradient effect, and returns the last frame.
fn gradient_frame(
    sampling: SamplingMode,
    scale: RenderScale,
    led_count: u32,
) -> Vec<Rgb8> {
    let engine = spawn(settings(30));
    let dev = device("d0", vec![zone(0, ZoneKind::Linear, led_count)]);
    let lay = layout(
        vec![placed("d0", 0, 0.0, 0.0, 320.0, 200.0)],
        sampling,
        scale,
    );
    engine.set_layout(lay, vec![dev]);
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    engine.set_effect(
        Box::new(GradientEffect),
        &Props::new(),
    );
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));
    let frame = sink.last().expect("a frame");
    engine.shutdown();
    frame
}

/// The red channel the gradient effect paints at render pixel `x` of a 320-wide
/// canvas.
fn gradient_red(x: f32) -> u8 {
    ((1.0 - x / 319.0) * 255.0 + 0.5) as u8
}

#[test]
fn nearest_sampling_takes_the_pixel_at_the_led_center() {
    // Two LEDs across the full canvas width: the cells are 160 wide, so the LED
    // centres sit at x = 80 and x = 240.
    let frame = gradient_frame(SamplingMode::Nearest, RenderScale::P100, 2);
    assert_eq!(frame.len(), 2);
    assert_eq!(frame[0].r, gradient_red(80.0), "LED 0 red channel");
    assert_eq!(frame[0].b, 255 - gradient_red(80.0), "LED 0 blue channel");
    assert_eq!(frame[1].r, gradient_red(240.0));
    assert_eq!(frame[1].b, 255 - gradient_red(240.0));
}

#[test]
fn bilinear_sampling_interpolates_at_the_led_center() {
    // One LED at the very centre of the canvas, x = 160.
    let frame = gradient_frame(SamplingMode::Bilinear, RenderScale::P100, 1);
    assert_eq!(frame.len(), 1);
    // Bilinear at x=160 interpolates between pixels 160 and 161, which are nearly
    // identical, so the result is within a step of the exact gradient.
    let exact = (1.0 - 160.0 / 319.0) * 255.0;
    let got = f32::from(frame[0].r);
    assert!((got - exact).abs() < 2.0, "bilinear {got} vs {exact}");
}

#[test]
fn area_average_sampling_averages_the_cell() {
    // The first cell of a two-LED zone covers render pixels x in 0..160.
    let frame = gradient_frame(SamplingMode::AreaAverage, RenderScale::P100, 2);
    assert_eq!(frame.len(), 2);
    // The mean of the gradient over that cell.
    let mean_x = (0..160u32).map(|x| x as f32).sum::<f32>() / 160.0;
    let expected = ((1.0 - mean_x / 319.0) * 255.0 + 0.5) as u8;
    assert_eq!(frame[0].r, expected, "area average red channel");
}

#[test]
fn the_three_sampling_modes_give_different_results() {
    // Three LEDs across the canvas, so the cell centres land on x = 53.33, 160 and
    // 266.67 - fractional positions, where nearest, bilinear and area average differ.
    let nearest = gradient_frame(SamplingMode::Nearest, RenderScale::P100, 3);
    let bilinear = gradient_frame(SamplingMode::Bilinear, RenderScale::P100, 3);
    let area = gradient_frame(SamplingMode::AreaAverage, RenderScale::P100, 3);
    assert_ne!(nearest, bilinear, "nearest and bilinear differ");
    assert_ne!(nearest, area, "nearest and area average differ");
    assert_ne!(bilinear, area, "bilinear and area average differ");
}

#[test]
fn the_render_scale_changes_the_sampled_colors() {
    // At 100% the LED centres land on exact pixel positions; at 25% the canvas is
    // 80x50 and the LED centres land on quarter positions, so the sampled colors
    // differ.
    let full = gradient_frame(SamplingMode::Nearest, RenderScale::P100, 8);
    let quarter = gradient_frame(SamplingMode::Nearest, RenderScale::P25, 8);
    assert_eq!(full.len(), 8);
    assert_eq!(quarter.len(), 8);
    assert_ne!(full, quarter, "the render scale changes the result");
}

// ---------------------------------------------------------------------------
// A zone split into parts
// ---------------------------------------------------------------------------

/// A placement that owns a run of its zone's LEDs.
fn run(mut p: PlacedZone, start: u32, count: u32) -> PlacedZone {
    p.leds = Some(LedSlice { start, count });
    p
}

#[test]
fn the_leds_of_a_split_zone_that_nobody_owns_are_black() {
    let engine = spawn(settings(30));
    // One 40-LED header split into three runs of 12, so the last 4 LEDs belong to
    // nobody. They must be black, not the effect's top-left pixel.
    let dev = device("d0", vec![zone(0, ZoneKind::Linear, 40)]);
    let lay = layout(
        vec![
            run(placed("d0", 0, 0.0, 0.0, 120.0, 20.0), 0, 12),
            run(placed("d0", 0, 0.0, 30.0, 120.0, 20.0), 12, 12),
            run(placed("d0", 0, 0.0, 60.0, 120.0, 20.0), 24, 12),
        ],
        SamplingMode::Nearest,
        RenderScale::P100,
    );
    engine.set_layout(lay, vec![dev]);

    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let initialized = Arc::new(AtomicBool::new(false));
    engine.set_effect(
        Box::new(SolidEffect::new([1.0, 0.0, 0.0], initialized)),
        &Props::new(),
    );

    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));
    let frame = sink.last().expect("a frame was submitted");
    assert_eq!(frame.len(), 40, "a frame has one color per LED");
    for (i, c) in frame.iter().enumerate() {
        if i < 36 {
            assert_eq!(*c, Rgb8::new(255, 0, 0), "LED {i} belongs to a part");
        } else {
            assert_eq!(*c, Rgb8::BLACK, "LED {i} belongs to nobody: it is black");
        }
    }
    engine.shutdown();
}

#[test]
fn a_zone_that_is_split_still_gets_one_color_per_led() {
    let engine = spawn(settings(30));
    let dev = device("d0", vec![zone(0, ZoneKind::Linear, 40)]);
    let lay = layout(
        vec![
            run(placed("d0", 0, 0.0, 0.0, 120.0, 20.0), 0, 12),
            run(placed("d0", 0, 0.0, 30.0, 120.0, 20.0), 12, 12),
        ],
        SamplingMode::AreaAverage,
        RenderScale::P50,
    );
    engine.set_layout(lay, vec![dev]);
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    let initialized = Arc::new(AtomicBool::new(false));
    engine.set_effect(
        Box::new(SolidEffect::new([0.0, 1.0, 0.0], initialized)),
        &Props::new(),
    );
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));
    let frame = sink.last().expect("a frame was submitted");
    assert_eq!(frame.len(), 40, "the frame still covers every LED of the zone");
    // The two runs cover LEDs 0..24; the other 16 belong to nobody.
    for (i, c) in frame.iter().enumerate() {
        if i < 24 {
            assert_eq!(*c, Rgb8::new(0, 255, 0), "LED {i} belongs to a part");
        } else {
            assert_eq!(*c, Rgb8::BLACK, "LED {i} belongs to nobody: it is black");
        }
    }
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// Canvas size
// ---------------------------------------------------------------------------

#[test]
fn the_canvas_is_the_logical_size_at_full_scale() {
    // The logical canvas is SignalRGB's, so an effect ported from it shares its
    // coordinates.
    assert_eq!(CANVAS_WIDTH, 320);
    assert_eq!(CANVAS_HEIGHT, 200);
    let canvas = Canvas::with_scale(1.0);
    assert_eq!(canvas.width, 320);
    assert_eq!(canvas.height, 200);
    assert_eq!(canvas.pixels.len(), 320 * 200);
}

// ---------------------------------------------------------------------------
// The golden plan
// ---------------------------------------------------------------------------

/// The devices of the golden layout: a wide linear zone, a tall linear zone, a
/// matrix zone with a gap in its map, and a zone nobody placed.
fn golden_devices() -> Vec<DeviceInfo> {
    let map = vec![Some(0), Some(1), None, Some(3)];
    vec![device(
        "d0",
        vec![
            zone(0, ZoneKind::Linear, 4),
            zone(1, ZoneKind::Linear, 3),
            zone(2, ZoneKind::Matrix { width: 2, height: 2, map }, 4),
            zone(3, ZoneKind::Linear, 5),
        ],
    )]
}

/// The golden layout: a layout saved before parts and slices existed, so no
/// placement carries a `leds` run or a `part`.
fn golden_layout() -> Layout {
    layout(
        vec![
            placed("d0", 0, 0.0, 0.0, 100.0, 20.0),
            placed("d0", 1, 10.0, 30.0, 60.0, 90.0),
            placed("d0", 2, 150.0, 0.0, 100.0, 100.0),
        ],
        SamplingMode::AreaAverage,
        RenderScale::P100,
    )
}

/// The plan the golden layout builds, and the frame that plan produces on a fixed
/// canvas, as text.
///
/// The text is the golden fixture: it was captured before the slice work, so a
/// layout saved before parts existed has to build the same plan, byte for byte.
fn golden_dump() -> String {
    let devices = golden_devices();
    let lay = golden_layout();
    let plan = crate::geometry::build_plan(&lay, &devices);
    let mut out = String::new();
    out.push_str(&format!("scale {:?} sampling {:?}\n", plan.scale, plan.sampling));
    for device in &plan.devices {
        out.push_str(&format!("device {} leds {}\n", device.device_id.0, device.led_count));
        for zone in &device.zones {
            out.push_str(&format!(
                "zone {} placed {} leds {}\n",
                zone.zone_index,
                zone.placed,
                zone.leds.len()
            ));
            // An unplaced zone is black: it has no region to print.
            if !zone.placed {
                continue;
            }
            for led in &zone.leds {
                match led.region() {
                    Some((cx, cy, bounds)) => {
                        out.push_str(&format!("  {cx:?} {cy:?} {bounds:?}\n"))
                    }
                    None => out.push_str("  black\n"),
                }
            }
        }
    }

    // The frame the plan produces on a fixed canvas: the unplaced zone is black
    // and the placed zones carry the canvas's colours. This is the engine's own
    // rule, so the frame is what the hardware would see.
    let mut canvas = Canvas::with_scale(plan.scale);
    for y in 0..canvas.height {
        for x in 0..canvas.width {
            canvas.set(
                x,
                y,
                [
                    x as f32 / 255.0,
                    y as f32 / 255.0,
                    ((x + y) % 256) as f32 / 255.0,
                ],
            );
        }
    }
    let mut frame: Vec<Rgb8> = Vec::new();
    for device in &plan.devices {
        for zone in &device.zones {
            if !zone.placed {
                frame.extend(std::iter::repeat_n(Rgb8::BLACK, zone.leds.len()));
                continue;
            }
            let mut colors = Vec::new();
            crate::sampling::sample_leds(
                &canvas.pixels,
                canvas.width,
                canvas.height,
                &zone.leds,
                plan.sampling,
                &mut colors,
            );
            frame.extend(colors);
        }
    }
    for c in &frame {
        out.push_str(&format!("{:02x}{:02x}{:02x} ", c.r, c.g, c.b));
    }
    out.push('\n');
    out
}

/// A layout saved before parts and slices existed builds the same plan, and the
/// same frame, as it did before the slice work.
#[test]
fn a_legacy_layout_builds_the_plan_it_built_before_parts_existed() {
    // A Windows checkout may turn the fixture's line endings into CRLF.
    let expected = include_str!("fixtures/legacy_plan.txt").replace("\r\n", "\n");
    assert_eq!(golden_dump(), expected, "the golden plan changed");
}

/// Writes the golden plan to stdout between two markers, so it can be captured
/// into the fixture file. Ignored: it is a capture, not a correctness test.
#[test]
#[ignore = "captures the golden fixture"]
fn capture_the_golden_plan() {
    println!("<<<GOLDEN>>>");
    print!("{}", golden_dump());
    println!("<<<END>>>");
}

// ---------------------------------------------------------------------------
// #99: a panicking effect must not kill the render thread
//
// These count calls on the fakes instead of reading a new engine flag, so they
// compile and run against the unguarded engine too: on the old code the render
// thread or the test thread panics and the frames stop, while the fix keeps
// the thread running, stops calling the effect, and lets a later effect run.
// The call counts are the "one record" of the fix: one caught panic, then no
// further calls until a new effect is set.
// ---------------------------------------------------------------------------

/// An effect whose `update` always panics. `updates` counts update calls.
struct PanicUpdateEffect {
    updates: Arc<AtomicU32>,
}

impl Effect for PanicUpdateEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_panic_update".into(),
            name: "Test Panic Update".into(),
            description: "Panics in update (#99).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        self.updates.fetch_add(1, Ordering::AcqRel);
        panic!("update boom");
    }
}

/// An effect whose `init` panics on every call but the first (the `set_effect`
/// one). `inits` counts init calls.
struct PanicInitEffect {
    inits: Arc<AtomicU32>,
}

impl Effect for PanicInitEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_panic_init".into(),
            name: "Test Panic Init".into(),
            description: "Panics in init after the first call (#99).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {
        let n = self.inits.fetch_add(1, Ordering::AcqRel) + 1;
        if n >= 2 {
            panic!("init boom");
        }
    }

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        canvas.fill([0.0, 0.0, 0.0]);
    }
}

/// An effect whose `update` paints the left half of the canvas red and then panics
/// mid-draw (#99). `updates` counts update calls.
struct HalfPanickedEffect {
    updates: Arc<AtomicU32>,
}

impl Effect for HalfPanickedEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_half_panicked".into(),
            name: "Test Half Panicked".into(),
            description: "Paints half the canvas then panics in update (#99).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        self.updates.fetch_add(1, Ordering::AcqRel);
        for y in 0..canvas.height {
            for x in 0..canvas.width / 2 {
                canvas.set(x, y, [1.0, 0.0, 0.0]);
            }
        }
        panic!("half-drawn boom");
    }
}

/// An effect whose `Drop` panics (#99). Replacement must not let that panic out.
/// `dropped_on` reports the thread the drop ran on, so the test can prove the caller
/// thread's drop really happened there (finding 5).
struct DropPanicsEffect {
    dropped_on: Option<Arc<Mutex<Option<std::thread::ThreadId>>>>,
}

impl Effect for DropPanicsEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_drop_panics".into(),
            name: "Test Drop Panics".into(),
            description: "Panics when it is dropped (#99).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        canvas.fill([0.0, 0.0, 1.0]);
    }
}

impl Drop for DropPanicsEffect {
    fn drop(&mut self) {
        if let Some(dropped_on) = &self.dropped_on {
            *dropped_on.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current().id());
        }
        panic!("drop boom");
    }
}

/// An effect whose `schema` panics every time; its `update` would paint green.
struct PanicSchemaEffect {
    schemas: Arc<AtomicU32>,
    updates: Arc<AtomicU32>,
}

impl Effect for PanicSchemaEffect {
    fn schema(&self) -> PropertySchema {
        self.schemas.fetch_add(1, Ordering::AcqRel);
        panic!("schema boom");
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        self.updates.fetch_add(1, Ordering::AcqRel);
        canvas.fill([0.0, 1.0, 0.0]);
    }
}

#[test]
fn a_panicking_update_does_not_kill_the_engine() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(Box::new(PanicUpdateEffect { updates: updates.clone() }), &Props::new());

    // The first update panics, but the loop keeps delivering frames.
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));
    assert!(wait_until(FIRST_FRAME_WAIT, || updates.load(Ordering::Acquire) >= 1));
    let frames_after_panic = sink.frame_count();
    assert!(
        wait_until(FIRST_FRAME_WAIT, || sink.frame_count() >= frames_after_panic + 10),
        "frames keep arriving after the update panic (got {})",
        sink.frame_count()
    );
    // One panic, then the effect is not called again until it is replaced.
    assert_eq!(updates.load(Ordering::Acquire), 1, "update ran once, then stopped");

    // A later effect set after it renders frames.
    let initialized = Arc::new(AtomicBool::new(false));
    engine.set_effect(
        Box::new(SolidEffect::new([0.0, 1.0, 0.0], initialized.clone())),
        &Props::new(),
    );
    assert!(wait_until(FIRST_FRAME_WAIT, || sink
        .last()
        .map(|f| f.iter().all(|c| *c == Rgb8::new(0, 255, 0)))
        .unwrap_or(false)));
    engine.shutdown();
}

#[test]
fn a_panicking_init_does_not_kill_the_engine() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let inits = Arc::new(AtomicU32::new(0));
    engine.set_effect(Box::new(PanicInitEffect { inits: inits.clone() }), &Props::new());
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));

    // The second init panics, on the calling thread when the effect is idle or
    // on the render thread when it is queued as pending props. Either way the
    // engine must survive it.
    engine.set_props(&Props::new());
    assert!(wait_until(FIRST_FRAME_WAIT, || {
        inits.load(Ordering::Acquire) >= 2 && engine.stats().frames_rendered > 5
    }));
    let frames_after_panic = sink.frame_count();
    let rendered_after_panic = engine.stats().frames_rendered;
    assert!(
        wait_until(FIRST_FRAME_WAIT, || engine.stats().frames_rendered >= rendered_after_panic + 10),
        "the loop keeps ticking after the init panic (got {})",
        engine.stats().frames_rendered
    );
    // A stopped effect is not initialised again.
    engine.set_props(&Props::new());
    assert!(wait_until(FIRST_FRAME_WAIT, || {
        engine.stats().frames_rendered >= rendered_after_panic + 15
    }));
    // Nothing was delivered on panicked ticks after the last good black frame.
    assert_eq!(sink.frame_count(), frames_after_panic, "no frames are delivered while the effect is stopped");
    assert_eq!(inits.load(Ordering::Acquire), 2, "init ran twice, then stopped");

    // A later effect set after it renders frames.
    let initialized = Arc::new(AtomicBool::new(false));
    engine.set_effect(
        Box::new(SolidEffect::new([0.0, 0.0, 1.0], initialized.clone())),
        &Props::new(),
    );
    assert!(wait_until(FIRST_FRAME_WAIT, || sink
        .last()
        .map(|f| f.iter().all(|c| *c == Rgb8::new(0, 0, 255)))
        .unwrap_or(false)));
    engine.shutdown();
}

#[test]
fn a_panicking_effect_logs_once_and_is_never_called_again() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(Box::new(PanicUpdateEffect { updates: updates.clone() }), &Props::new());
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));
    assert!(wait_until(FIRST_FRAME_WAIT, || updates.load(Ordering::Acquire) >= 1));

    // More frames and more prop changes would each panic again if the engine
    // kept calling the effect; the effect must stay stopped.
    let frames_after_panic = sink.frame_count();
    engine.set_props(&Props::new());
    engine.set_props(&Props::new());
    assert!(
        wait_until(FIRST_FRAME_WAIT, || sink.frame_count() >= frames_after_panic + 20),
        "frames keep arriving (got {})",
        sink.frame_count()
    );
    assert_eq!(
        updates.load(Ordering::Acquire),
        1,
        "a second panic in a row did not call the effect again"
    );
    engine.shutdown();
}

#[test]
fn the_lights_never_receive_the_half_drawn_frame_after_an_update_panic() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let green = Rgb8::new(0, 255, 0);
    let initialized = Arc::new(AtomicBool::new(false));
    engine.set_effect(
        Box::new(SolidEffect::new([0.0, 1.0, 0.0], initialized.clone())),
        &Props::new(),
    );
    assert!(wait_until(
        FIRST_FRAME_WAIT,
        || sink.last().is_some_and(|f| f.iter().all(|c| *c == green)),
    ));

    // The half-panicked effect paints the left half red, then panics. The frame
    // sampled from that torn canvas must never reach the sink.
    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(Box::new(HalfPanickedEffect { updates: updates.clone() }), &Props::new());
    assert!(wait_until(FIRST_FRAME_WAIT, || updates.load(Ordering::Acquire) >= 1));
    let rendered_after_panic = engine.stats().frames_rendered;
    assert!(
        wait_until(
            FIRST_FRAME_WAIT,
            || engine.stats().frames_rendered >= rendered_after_panic + 10
        ),
        "the loop keeps ticking after the update panic (got {})",
        engine.stats().frames_rendered
    );
    assert_eq!(updates.load(Ordering::Acquire), 1, "update ran once, then stopped");
    {
        let frames = sink.frames.lock().unwrap_or_else(|e| e.into_inner());
        let red = Rgb8::new(255, 0, 0);
        assert!(
            frames.iter().all(|f| f.iter().all(|c| *c != red)),
            "no frame the sink got is red from the torn half-drawn canvas"
        );
        assert!(
            frames.iter().all(|f| f.iter().all(|c| *c == green || *c == Rgb8::BLACK)),
            "every frame the sink got stayed the last good green frame or black before a good frame"
        );
    }
    assert_eq!(
        sink.last(),
        Some(vec![Rgb8::BLACK; 4]),
        "the last frame is black before the half-panicked effect's first good frame, never the half-red one"
    );
    engine.shutdown();
}

#[test]
fn replacing_an_effect_whose_drop_panics_does_not_panic_the_caller() {
    let engine = spawn(settings(30));
    // No sink yet: the loop is parked, so the old effect is still in the state when
    // the replacement happens and its drop runs on this calling thread (#99).
    let dropped_on = Arc::new(Mutex::new(None));
    let caller_thread = std::thread::current().id();
    engine.set_effect(Box::new(DropPanicsEffect { dropped_on: Some(dropped_on.clone()) }), &Props::new());
    engine.set_effect(
        Box::new(SolidEffect::new(
            [0.0, 1.0, 0.0],
            Arc::new(AtomicBool::new(false)),
        )),
        &Props::new(),
    );

    // Deterministic: the drop really ran on this thread, and its panic stayed
    // guarded (with the `set_effect` drop guard removed, this panic reaches the
    // caller and the test fails when it unwinds).
    assert_eq!(
        *dropped_on.lock().unwrap_or_else(|e| e.into_inner()),
        Some(caller_thread),
        "the replaced effect was dropped on the caller thread"
    );

    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());
    let green = Rgb8::new(0, 255, 0);
    assert!(
        wait_until(
            FIRST_FRAME_WAIT,
            || sink.last().is_some_and(|f| f.iter().all(|c| *c == green)),
        ),
        "the engine runs the new effect after the drop panic"
    );
    engine.shutdown();
}

#[test]
fn a_panicking_schema_does_not_kill_the_engine() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let schemas = Arc::new(AtomicU32::new(0));
    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(
        Box::new(PanicSchemaEffect { schemas: schemas.clone(), updates: updates.clone() }),
        &Props::new(),
    );
    // No good frame ever: the sink gets black and keeps getting frames.
    assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0));
    let frames_after_panic = sink.frame_count();
    assert!(
        wait_until(
            FIRST_FRAME_WAIT,
            || sink.frame_count() >= frames_after_panic + 10
        ),
        "frames keep arriving after the schema panic (got {})",
        sink.frame_count()
    );
    assert_eq!(updates.load(Ordering::Acquire), 0, "update never ran for the stopped effect");
    assert!(
        sink.last().is_some_and(|f| f.iter().all(|c| *c == Rgb8::BLACK)),
        "black before the first good frame"
    );

    let initialized = Arc::new(AtomicBool::new(false));
    engine.set_effect(
        Box::new(SolidEffect::new([0.0, 0.0, 1.0], initialized.clone())),
        &Props::new(),
    );
    let blue = Rgb8::new(0, 0, 255);
    assert!(
        wait_until(
            FIRST_FRAME_WAIT,
            || sink.last().is_some_and(|f| f.iter().all(|c| *c == blue)),
        ),
        "a later effect renders after the schema panic"
    );
    engine.shutdown();
}

/// An effect that paints only the right half of the canvas green and never clears it.
/// A replacement effect starts from a cleared canvas, so the left half is black, never
/// the torn half the previous effect left behind (finding 3).
struct RightHalfEffect;

impl Effect for RightHalfEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_right_half".into(),
            name: "Test Right Half".into(),
            description: "Paints the right half green (finding 3).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        for y in 0..canvas.height {
            for x in canvas.width / 2..canvas.width {
                canvas.set(x, y, [0.0, 1.0, 0.0]);
            }
        }
    }
}

/// Finding 3: the last-good state is reset per generation, so a NEW effect that panics
/// on its first tick shows black, never the PREVIOUS effect's frame.
#[test]
fn a_new_effect_that_panics_before_a_good_frame_shows_black_not_the_previous_effect() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let green = Rgb8::new(0, 255, 0);
    engine.set_effect(
        Box::new(SolidEffect::new([0.0, 1.0, 0.0], Arc::new(AtomicBool::new(false)))),
        &Props::new(),
    );
    assert!(wait_until(
        FIRST_FRAME_WAIT,
        || sink.last().is_some_and(|f| f.iter().all(|c| *c == green)),
    ));

    // A new effect that panics on its first update. With the generation reset it
    // shows black; without it the previous good green frame is the last thing the
    // sink was sent and black never arrives.
    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(Box::new(PanicUpdateEffect { updates: updates.clone() }), &Props::new());
    assert!(wait_until(FIRST_FRAME_WAIT, || updates.load(Ordering::Acquire) >= 1));
    assert!(
        wait_until(FIRST_FRAME_WAIT, || sink
            .last()
            .is_some_and(|f| f.iter().all(|c| *c == Rgb8::BLACK))),
        "the panicking new effect shows black, not the previous green frame; last = {:?}",
        sink.last()
    );
    engine.shutdown();
}

/// Finding 3: clear the canvas on `set_effect`, so a replacement that paints only part
/// of the canvas does not start from a torn half-drawn canvas. The replacement paints
/// the right half green; the leftover left-half red from the torn effect must be gone.
#[test]
fn a_replacement_effect_starts_from_a_cleared_canvas_not_a_torn_one() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let green = Rgb8::new(0, 255, 0);
    engine.set_effect(
        Box::new(SolidEffect::new([0.0, 1.0, 0.0], Arc::new(AtomicBool::new(false)))),
        &Props::new(),
    );
    assert!(wait_until(
        FIRST_FRAME_WAIT,
        || sink.last().is_some_and(|f| f.iter().all(|c| *c == green)),
    ));

    // The torn effect paints the left half red, then panics.
    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(Box::new(HalfPanickedEffect { updates: updates.clone() }), &Props::new());
    assert!(wait_until(FIRST_FRAME_WAIT, || updates.load(Ordering::Acquire) >= 1));

    engine.set_effect(Box::new(RightHalfEffect), &Props::new());
    // The 4 LEDs of the test layout are spread across the canvas; after the clear,
    // the replacement's frame has green on the right and black on the left.
    assert!(wait_until(FIRST_FRAME_WAIT, || {
        sink.last().is_some_and(|f| {
            f.contains(&green) && f.contains(&Rgb8::BLACK)
        })
    }), "the replacement drew green on the right and black on the left; last = {:?}", sink.last());
    let red = Rgb8::new(255, 0, 0);
    assert!(
        sink.last().is_some_and(|f| f.iter().all(|c| *c != red)),
        "no red from the torn canvas remains: {:?}",
        sink.last()
    );
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// #145: after a panicked effect is skipped, the last good frame goes out again
//
// The skipped effect stops the delivery (#99), so a device that needs a frame again
// kept the frame it was last sent (or the black frame the pause wrote) until another
// effect was picked. The three cases: a resume from a pause, a resume from sleep
// (the daemon's power path, `crates/canvas-daemon/tests/power_resume_panic.rs`) and a
// device that arrives while the effect is stopped.
// ---------------------------------------------------------------------------

/// The green of the good frame the tests below keep and re-send.
const GOOD: Rgb8 = Rgb8 { r: 0, g: 255, b: 0 };

/// An effect that paints one good frame and then panics (#145).
///
/// The first update paints the canvas, so the engine has a good frame to keep. The
/// second one panics, and the engine stops delivering (#99).
struct PanicAfterGoodFrameEffect {
    updates: Arc<AtomicU32>,
}

impl Effect for PanicAfterGoodFrameEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_panic_after_good_frame".into(),
            name: "Test Panic After Good Frame".into(),
            description: "Paints one frame, then panics (#145).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        let n = self.updates.fetch_add(1, Ordering::AcqRel);
        if n >= 1 {
            panic!("update boom after a good frame");
        }
        canvas.fill([0.0, 1.0, 0.0]);
    }
}

/// An engine whose effect painted one good frame and then panicked.
///
/// The green frame is the last good frame: it is what a resume, a new sink and the
/// adapter's keepalive have to get again (#145).
fn engine_stopped_after_a_good_frame() -> (Arc<dyn canvas_proto::traits::Engine>, Arc<RecordingSink>)
{
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(
        Box::new(PanicAfterGoodFrameEffect {
            updates: updates.clone(),
        }),
        &Props::new(),
    );
    assert!(
        wait_until(FIRST_FRAME_WAIT, || sink
            .last()
            .is_some_and(|f| f.iter().all(|c| *c == GOOD))),
        "the good frame arrived"
    );
    assert!(
        wait_until(FIRST_FRAME_WAIT, || updates.load(Ordering::Acquire) >= 2),
        "the effect panicked"
    );
    // The loop keeps ticking and nothing is delivered while the effect is stopped.
    let frames_at_panic = sink.frame_count();
    let rendered = engine.stats().frames_rendered;
    assert!(wait_until(FIRST_FRAME_WAIT, || engine
        .stats()
        .frames_rendered
        >= rendered + 10));
    assert_eq!(
        sink.frame_count(),
        frames_at_panic,
        "nothing is delivered while the effect is stopped"
    );
    (engine, sink)
}

/// #145: a pause writes black, and the resume must send the last good frame again.
///
/// Without the fix the lights stay black until another effect is picked: the panicked
/// effect is skipped, so nothing is delivered on the ticks after the resume.
#[test]
fn a_resume_after_a_panic_sends_the_last_good_frame_again() {
    let (engine, sink) = engine_stopped_after_a_good_frame();

    // The pause writes one black frame.
    engine.set_power(Power::Paused);
    assert!(
        wait_until(FIRST_FRAME_WAIT, || sink
            .last()
            .is_some_and(|f| f.iter().all(|c| *c == Rgb8::BLACK))),
        "the pause wrote a black frame"
    );

    // The resume sends the last good frame again.
    let frames_at_pause = sink.frame_count();
    engine.set_power(Power::Running);
    assert!(
        wait_until(FIRST_FRAME_WAIT, || {
            sink.frame_count() > frames_at_pause
                && sink.last().is_some_and(|f| f.iter().all(|c| *c == GOOD))
        }),
        "the last good frame went out again after the resume; last = {:?}",
        sink.last()
    );
    engine.shutdown();
}

/// A layout with two devices, `d0` and `d1`, four LEDs each.
fn give_d0_and_d1_a_layout(engine: &Arc<dyn canvas_proto::traits::Engine>) {
    let devices = vec![
        device("d0", vec![zone(0, ZoneKind::Linear, 4)]),
        device("d1", vec![zone(0, ZoneKind::Linear, 4)]),
    ];
    let lay = layout(
        vec![
            placed("d0", 0, 0.0, 0.0, 160.0, 200.0),
            placed("d1", 0, 160.0, 0.0, 160.0, 200.0),
        ],
        SamplingMode::AreaAverage,
        RenderScale::P100,
    );
    engine.set_layout(lay, devices);
}

/// #145: a device that arrives while the effect is stopped gets the last good frame.
///
/// The device is in the layout, so the engine has a frame for it; the sink arrives
/// later. Without the fix the new sink gets nothing until another effect is picked.
#[test]
fn a_sink_added_after_a_panic_gets_the_last_good_frame() {
    let engine = spawn(settings(30));
    let sink = RecordingSink::new();
    give_d0_and_d1_a_layout(&engine);
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(
        Box::new(PanicAfterGoodFrameEffect {
            updates: updates.clone(),
        }),
        &Props::new(),
    );
    assert!(
        wait_until(FIRST_FRAME_WAIT, || sink
            .last()
            .is_some_and(|f| f.iter().all(|c| *c == GOOD))),
        "the good frame arrived"
    );
    assert!(
        wait_until(FIRST_FRAME_WAIT, || updates.load(Ordering::Acquire) >= 2),
        "the effect panicked"
    );

    // A device arrives while the effect is stopped.
    let late = RecordingSink::new();
    engine.add_sink(DeviceId("d1".into()), late.clone());
    assert!(
        wait_until(FIRST_FRAME_WAIT, || late
            .last()
            .is_some_and(|f| f.iter().all(|c| *c == GOOD))),
        "the new sink got the last good frame; last = {:?}",
        late.last()
    );
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// #145: the last good frame belongs to the device it was drawn for
//
// The saved frames were indexed by the position of the device in the plan, so a
// re-layout that listed the same devices in another order handed every device the
// other's frame. The frames are keyed by the device they were drawn for.
// ---------------------------------------------------------------------------

/// The red of the left half of the canvas.
const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };
/// The blue of the right half of the canvas.
const BLUE: Rgb8 = Rgb8 { r: 0, g: 0, b: 255 };

/// An effect that paints the left half of the canvas red, the right half blue, and
/// then panics (#145).
///
/// The first update paints both halves, so every device has a good frame of its own
/// colour to keep. The second one panics, and the engine stops delivering (#99).
struct TwoHalvesThenPanicEffect {
    updates: Arc<AtomicU32>,
}

impl Effect for TwoHalvesThenPanicEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_two_halves_then_panic".into(),
            name: "Test Two Halves Then Panic".into(),
            description: "Paints two halves, then panics (#145).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        let n = self.updates.fetch_add(1, Ordering::AcqRel);
        if n >= 1 {
            panic!("update boom after a good frame");
        }
        let half = canvas.width / 2;
        for y in 0..canvas.height {
            for x in 0..canvas.width {
                let color = if x < half {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 0.0, 1.0]
                };
                canvas.set(x, y, color);
            }
        }
    }
}

/// Two devices, `a` and `b`, eight LEDs each, side by side: `a` on the left half of
/// the canvas, `b` on the right half.
///
/// The devices are listed in the given order, and the plan lists them in that order
/// too (`geometry::build_plan` walks the device list), so the order of the list is
/// the order of the plan.
fn two_devices_in_order(order: &[&str]) -> (Layout, Vec<DeviceInfo>) {
    let devices = order
        .iter()
        .copied()
        .map(|id| device(id, vec![zone(0, ZoneKind::Linear, 8)]))
        .collect();
    let lay = layout(
        vec![
            placed("a", 0, 0.0, 0.0, 160.0, 200.0),
            placed("b", 0, 160.0, 0.0, 160.0, 200.0),
        ],
        SamplingMode::AreaAverage,
        RenderScale::P100,
    );
    (lay, devices)
}

/// #145: a re-layout that lists the same devices in another order keeps every
/// device's own last good frame.
///
/// The saved frames were indexed by the position of the device in the plan, so a
/// re-layout that swapped two devices of the same LED count handed each device the
/// other's frame: the lights showed swapped colours after the resume.
#[test]
fn a_re_layout_with_the_devices_in_another_order_keeps_each_device_s_own_frame() {
    let engine = spawn(settings(30));
    let (lay, devices) = two_devices_in_order(&["a", "b"]);
    engine.set_layout(lay, devices);

    let sink_a = RecordingSink::new();
    let sink_b = RecordingSink::new();
    engine.add_sink(DeviceId("a".into()), sink_a.clone());
    engine.add_sink(DeviceId("b".into()), sink_b.clone());

    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(
        Box::new(TwoHalvesThenPanicEffect {
            updates: updates.clone(),
        }),
        &Props::new(),
    );

    // The good frame: `a` is red, `b` is blue.
    assert!(
        wait_until(FIRST_FRAME_WAIT, || {
            sink_a.last().is_some_and(|f| f.iter().all(|c| *c == RED))
                && sink_b.last().is_some_and(|f| f.iter().all(|c| *c == BLUE))
        }),
        "the good frames arrived; a = {:?}, b = {:?}",
        sink_a.last(),
        sink_b.last()
    );
    assert!(
        wait_until(FIRST_FRAME_WAIT, || updates.load(Ordering::Acquire) >= 2),
        "the effect panicked"
    );

    // The same devices, listed in the other order: the plan follows the device list.
    let (lay, devices) = two_devices_in_order(&["b", "a"]);
    engine.set_layout(lay, devices);
    // The render thread picks the new plan up on its next tick, and the frames it
    // keeps are moved onto it.
    let rendered = engine.stats().frames_rendered;
    assert!(wait_until(FIRST_FRAME_WAIT, || engine
        .stats()
        .frames_rendered
        >= rendered + 5));

    // The pause writes black to every device.
    engine.set_power(Power::Paused);
    assert!(
        wait_until(FIRST_FRAME_WAIT, || {
            sink_a
                .last()
                .is_some_and(|f| f.iter().all(|c| *c == Rgb8::BLACK))
                && sink_b
                    .last()
                    .is_some_and(|f| f.iter().all(|c| *c == Rgb8::BLACK))
        }),
        "the pause wrote black; a = {:?}, b = {:?}",
        sink_a.last(),
        sink_b.last()
    );

    // The resume sends every device its own last good frame again.
    let frames_a = sink_a.frame_count();
    let frames_b = sink_b.frame_count();
    engine.set_power(Power::Running);
    assert!(
        wait_until(FIRST_FRAME_WAIT, || {
            sink_a.frame_count() > frames_a
                && sink_b.frame_count() > frames_b
                && sink_a.last().is_some_and(|f| f.iter().all(|c| *c == RED))
                && sink_b.last().is_some_and(|f| f.iter().all(|c| *c == BLUE))
        }),
        "every device got its own last good frame; a = {:?}, b = {:?}",
        sink_a.last(),
        sink_b.last()
    );

    // And nothing else: no device ever saw the other's colour.
    for frame in sink_a.all_frames() {
        assert!(
            frame.iter().all(|c| *c == RED || *c == Rgb8::BLACK),
            "device a saw a colour that is not its own: {frame:?}"
        );
    }
    for frame in sink_b.all_frames() {
        assert!(
            frame.iter().all(|c| *c == BLUE || *c == Rgb8::BLACK),
            "device b saw a colour that is not its own: {frame:?}"
        );
    }
    engine.shutdown();
}

// ---------------------------------------------------------------------------
// Scenes (0.14.0, #310): one effect per group, merged per device
// ---------------------------------------------------------------------------

mod scene {
    use super::*;
    use canvas_proto::scene::{zone_light_id, Arrangement, GroupMode, Light, SCENE_FADE_MS};
    use canvas_proto::traits::{Engine, EngineGroup, LightFrame, LightPreviewSink};

    const RED: Rgb8 = Rgb8::new(255, 0, 0);
    const BLUE: Rgb8 = Rgb8::new(0, 0, 255);
    const GREEN: Rgb8 = Rgb8::new(0, 255, 0);

    /// A recording light-preview sink.
    #[derive(Default)]
    struct RecordingLights {
        frames: Mutex<Vec<LightFrame>>,
    }

    impl RecordingLights {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn last(&self) -> Option<LightFrame> {
            self.frames.lock().unwrap_or_else(|e| e.into_inner()).last().cloned()
        }
        fn count(&self) -> usize {
            self.frames.lock().unwrap_or_else(|e| e.into_inner()).len()
        }
    }

    impl LightPreviewSink for RecordingLights {
        fn submit(&self, frame: LightFrame) {
            self.frames.lock().unwrap_or_else(|e| e.into_inner()).push(frame);
        }
    }

    /// A device with one linear zone of `leds` LEDs.
    fn strip_device(id: &str, leds: u32) -> DeviceInfo {
        device(id, vec![zone(0, ZoneKind::Linear, leds)])
    }

    /// The light that is the whole of zone 0 of device `id`.
    fn light_of(id: &str) -> Light {
        let device_id = DeviceId(id.to_string());
        Light {
            id: zone_light_id(&device_id, 0),
            device_id,
            zone_index: 0,
            leds: None,
            name: None,
            arrangement: Arrangement::Strip,
            reversed: false,
        }
    }

    /// A light that is a run of zone `zone` of device `id`.
    fn slice_light(id: &str, zone: u32, start: u32, count: u32) -> Light {
        let device_id = DeviceId(id.to_string());
        Light {
            id: format!("{id}#z{zone}s{start}"),
            device_id,
            zone_index: zone,
            leds: Some(LedSlice { start, count }),
            name: None,
            arrangement: Arrangement::Strip,
            reversed: false,
        }
    }

    fn group(id: &str, members: &[&str], effect: Option<Box<dyn Effect>>) -> EngineGroup {
        EngineGroup {
            id: id.to_string(),
            members: members.iter().map(|m| m.to_string()).collect(),
            mode: GroupMode::FlowAcross,
            brightness: 100,
            effect,
            props: Props::new(),
        }
    }

    fn solid(color: [f32; 3]) -> Option<Box<dyn Effect>> {
        Some(Box::new(SolidEffect::new(color, Arc::new(AtomicBool::new(false)))))
    }

    /// Two devices `d1`, `d2` of 12 LEDs, each with its own sink.
    fn two_devices() -> (Arc<dyn Engine>, Vec<DeviceInfo>, Arc<RecordingSink>, Arc<RecordingSink>) {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12), strip_device("d2", 12)];
        let a = RecordingSink::new();
        let b = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), a.clone());
        engine.add_sink(DeviceId("d2".into()), b.clone());
        (engine, devices, a, b)
    }

    fn lights_d1_d2() -> Vec<Light> {
        vec![light_of("d1"), light_of("d2")]
    }

    fn all(frame: &[Rgb8], color: Rgb8) -> bool {
        frame.iter().all(|c| *c == color)
    }

    /// An effect that panics in `update` and again in `Drop` (#310 review 1).
    ///
    /// The `update` panic marks the group failed, and a failed group keeps its effect in
    /// the scene's state (the render thread does not take it out again). The effect is
    /// therefore in the state when the scene ends, which is what `set_layout` drops.
    struct FailingEffectThatPanicsInDrop;

    impl Effect for FailingEffectThatPanicsInDrop {
        fn schema(&self) -> PropertySchema {
            PropertySchema {
                id: "test_fail_and_panic_in_drop".into(),
                name: "Test Fail And Panic In Drop".into(),
                description: "Panics in update and in Drop.".into(),
                needs: InputNeeds::default(),
                properties: vec![],
                presets: vec![],
                thumbnail: None,
            }
        }

        fn init(&mut self, _props: &Props) {}

        fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
            panic!("update boom");
        }
    }

    impl Drop for FailingEffectThatPanicsInDrop {
        fn drop(&mut self) {
            panic!("drop boom");
        }
    }

    #[test]
    fn two_groups_play_two_effects_at_once() {
        let (engine, devices, a, b) = two_devices();
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![
                group("a", &["d1#z0"], solid([1.0, 0.0, 0.0])),
                group("b", &["d2#z0"], solid([0.0, 0.0, 1.0])),
            ],
            false,
        );
        assert!(
            wait_until(FIRST_FRAME_WAIT, || {
                a.last().is_some_and(|f| all(&f, RED)) && b.last().is_some_and(|f| all(&f, BLUE))
            }),
            "a = {:?}, b = {:?}",
            a.last(),
            b.last()
        );
        engine.shutdown();
    }

    #[test]
    fn lights_of_two_groups_on_one_device_merge_into_one_frame() {
        let engine = spawn(settings(60));
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 24)])];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d0".into()), sink.clone());
        engine.set_scene(
            vec![slice_light("d0", 0, 0, 12), slice_light("d0", 0, 12, 12)],
            devices,
            vec![
                group("a", &["d0#z0s0"], solid([1.0, 0.0, 0.0])),
                group("b", &["d0#z0s12"], solid([0.0, 0.0, 1.0])),
            ],
            false,
        );
        let mut want = vec![RED; 12];
        want.extend(vec![BLUE; 12]);
        assert!(
            wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| f == want)),
            "last = {:?}",
            sink.last()
        );
        engine.shutdown();
    }

    #[test]
    fn leds_no_group_owns_are_black() {
        let engine = spawn(settings(60));
        // Zone 0 has 8 LEDs nobody owns; the light is the first 12 of zone 1's 20.
        let devices = vec![device(
            "d0",
            vec![zone(0, ZoneKind::Linear, 8), zone(1, ZoneKind::Linear, 20)],
        )];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d0".into()), sink.clone());
        engine.set_scene(
            vec![slice_light("d0", 1, 0, 12)],
            devices,
            vec![group("a", &["d0#z1s0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || {
            sink.last().is_some_and(|f| f.len() == 28 && f[8] == RED)
        }));
        let frame = sink.last().expect("a frame");
        assert!(all(&frame[..8], Rgb8::BLACK), "zone 0 is nobody's: {frame:?}");
        assert!(all(&frame[8..20], RED), "the light's 12 LEDs: {frame:?}");
        assert!(all(&frame[20..], Rgb8::BLACK), "the rest of zone 1 is nobody's: {frame:?}");
        engine.shutdown();
    }

    #[test]
    fn a_group_set_to_off_is_black() {
        let (engine, devices, a, b) = two_devices();
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![group("a", &["d1#z0"], None), group("b", &["d2#z0"], solid([0.0, 0.0, 1.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || b.last().is_some_and(|f| all(&f, BLUE))));
        let frame = a.last().expect("device a still gets frames");
        assert_eq!(frame.len(), 12);
        assert!(all(&frame, Rgb8::BLACK));
        engine.shutdown();
    }

    #[test]
    fn group_brightness_multiplies_with_global() {
        let engine = spawn(settings(60));
        engine.set_brightness(50);
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        let mut g = group("a", &["d1#z0"], solid([200.0 / 255.0; 3]));
        g.brightness = 50;
        engine.set_scene(vec![light_of("d1")], devices, vec![g], false);
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 3));
        let frame = sink.last().expect("a frame");
        for c in &frame {
            for channel in [c.r, c.g, c.b] {
                assert!((49..=51).contains(&channel), "200 x 50% x 50% is 50, got {frame:?}");
            }
        }
        engine.shutdown();
    }

    #[test]
    fn a_panicking_group_goes_dark_and_reports_while_the_other_keeps_running() {
        let (engine, devices, a, b) = two_devices();
        let updates = Arc::new(AtomicU32::new(0));
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![
                group("a", &["d1#z0"], Some(Box::new(PanicUpdateEffect { updates: updates.clone() }))),
                group("b", &["d2#z0"], solid([0.0, 0.0, 1.0])),
            ],
            false,
        );
        assert!(
            wait_until(FIRST_FRAME_WAIT, || {
                engine.group_states().iter().any(|s| s.id == "a" && s.failed.is_some())
            }),
            "the panic is reported: {:?}",
            engine.group_states()
        );
        let states = engine.group_states();
        let failed = states.iter().find(|s| s.id == "a").and_then(|s| s.failed.clone());
        assert!(
            failed.is_some_and(|t| t.contains("update boom")),
            "the text of the panic: {states:?}"
        );
        assert!(states.iter().find(|s| s.id == "b").is_some_and(|s| s.failed.is_none()));
        // The other group keeps running; the failed one is dark and never called again.
        let frames_b = b.frame_count();
        assert!(wait_until(FIRST_FRAME_WAIT, || b.frame_count() > frames_b + 5));
        assert!(b.last().is_some_and(|f| all(&f, BLUE)));
        assert!(a.last().is_some_and(|f| all(&f, Rgb8::BLACK)));
        assert_eq!(updates.load(Ordering::Acquire), 1, "a failed group is not called again");
        engine.shutdown();
    }

    #[test]
    fn replacing_a_failed_groups_effect_clears_the_failure() {
        let (engine, devices, a, _b) = two_devices();
        let updates = Arc::new(AtomicU32::new(0));
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![
                group("a", &["d1#z0"], Some(Box::new(PanicUpdateEffect { updates }))),
                group("b", &["d2#z0"], solid([0.0, 0.0, 1.0])),
            ],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || {
            engine.group_states().iter().any(|s| s.id == "a" && s.failed.is_some())
        }));
        engine.set_group_effect("a", solid([1.0, 0.0, 0.0]), &Props::new());
        assert!(
            wait_until(FIRST_FRAME_WAIT, || a.last().is_some_and(|f| all(&f, RED))),
            "the new effect plays: {:?}",
            a.last()
        );
        assert!(
            engine.group_states().iter().all(|s| s.failed.is_none()),
            "{:?}",
            engine.group_states()
        );
        engine.shutdown();
    }

    #[test]
    fn set_group_props_reaches_only_that_group() {
        let (engine, devices, a, _b) = two_devices();
        let inits_a = Arc::new(AtomicU32::new(0));
        let inits_b = Arc::new(AtomicU32::new(0));
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![
                group("a", &["d1#z0"], Some(Box::new(CountingEffect { inits: inits_a.clone() }))),
                group("b", &["d2#z0"], Some(Box::new(CountingEffect { inits: inits_b.clone() }))),
            ],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || a.frame_count() > 2));
        let (before_a, before_b) = (inits_a.load(Ordering::Acquire), inits_b.load(Ordering::Acquire));
        engine.set_group_props("a", &Props::new());
        let frames = a.frame_count();
        assert!(wait_until(FIRST_FRAME_WAIT, || a.frame_count() > frames + 3));
        assert_eq!(inits_a.load(Ordering::Acquire), before_a + 1, "group a re-initialised");
        assert_eq!(inits_b.load(Ordering::Acquire), before_b, "group b did not");
        engine.shutdown();
    }

    #[test]
    fn set_scene_with_fade_blends_then_settles() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, RED))));
        let before = sink.frame_count() as usize;
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("a", &["d1#z0"], solid([0.0, 0.0, 1.0]))],
            true,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, BLUE))));
        let frames = sink.all_frames();
        let mid = frames[before..]
            .iter()
            .any(|f| f[0].r > 0 && f[0].r < 255 && f[0].b > 0 && f[0].b < 255);
        assert!(mid, "a frame between the old red and the new blue: {:?}", &frames[before..]);
        // The fade ends on the new scene and stays there: from the first all-new frame on, every
        // frame is all new (the fade is SCENE_FADE_MS long and the frames below outlast it).
        let settled = frames.len();
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() as usize > settled + 10));
        let frames = sink.all_frames();
        let first_blue = frames[before..].iter().position(|f| all(f, BLUE)).expect("settled") + before;
        assert!(
            frames[first_blue..].iter().all(|f| all(f, BLUE)),
            "after {SCENE_FADE_MS} ms the scene is the new one: {:?}",
            &frames[first_blue..]
        );
        engine.shutdown();
    }

    #[test]
    fn set_scene_without_fade_cuts() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, RED))));
        let before = sink.frame_count() as usize;
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("a", &["d1#z0"], solid([0.0, 0.0, 1.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, BLUE))));
        for frame in &sink.all_frames()[before..] {
            assert!(all(frame, RED) || all(frame, BLUE), "no blended frame on a cut: {frame:?}");
        }
        engine.shutdown();
    }

    #[test]
    fn light_preview_carries_each_lights_colours_before_global_brightness() {
        let engine = spawn(settings(60));
        engine.set_brightness(50);
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        let lights = RecordingLights::new();
        engine.set_light_preview(Some(lights.clone()));
        let g = group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]));
        engine.set_scene(vec![light_of("d1")], devices, vec![g], false);
        assert!(wait_until(FIRST_FRAME_WAIT, || lights.count() > 0 && sink.frame_count() > 0));
        let preview = lights.last().expect("a light frame");
        let (id, colours) = &preview.lights[0];
        assert_eq!(id, "d1#z0");
        assert_eq!(colours.len(), 12);
        assert!(colours.iter().all(|c| *c == RED), "before the global stage: {colours:?}");
        let device_frame = sink.last().expect("a frame");
        assert!(
            device_frame.iter().all(|c| (127..=129).contains(&c.r)),
            "after it: {device_frame:?}"
        );
        engine.shutdown();
    }

    #[test]
    fn light_preview_is_rate_limited() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        let lights = RecordingLights::new();
        engine.set_light_preview(Some(lights.clone()));
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || lights.count() > 0));
        let (l0, s0) = (lights.count(), sink.frame_count());
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > s0 + 60));
        let previews = lights.count() - l0;
        let frames = (sink.frame_count() - s0) as usize;
        // 60 fps against a 15 fps cap: about a quarter of the frames, never half.
        assert!(previews * 2 < frames, "{previews} previews for {frames} frames");
        engine.shutdown();
    }

    /// Review Focus 2: a zone's LED count changes while its light is in a group.
    #[test]
    fn resize_rebuilds_plan_without_stale_frame() {
        let engine = spawn(settings(60));
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d1")],
            vec![strip_device("d1", 12)],
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| f.len() == 12)));
        engine.set_scene(
            vec![light_of("d1")],
            vec![strip_device("d1", 20)],
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        let seen = sink.frame_count() as usize;
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() as usize > seen + 5));
        let frames = sink.all_frames();
        for frame in &frames[seen..] {
            assert_eq!(frame.len(), 20, "a frame after the resize has the new length");
            assert!(all(frame, RED), "and every LED of it plays: {frame:?}");
        }
        engine.shutdown();
    }

    #[test]
    fn flow_across_shows_one_gradient_over_two_lights_and_each_its_own_repeats_it() {
        for (mode, flows) in [(GroupMode::FlowAcross, true), (GroupMode::EachItsOwn, false)] {
            let (engine, devices, a, b) = two_devices();
            let mut g = group("a", &["d1#z0", "d2#z0"], Some(Box::new(GradientEffect)));
            g.mode = mode;
            engine.set_scene(lights_d1_d2(), devices, vec![g], false);
            // The first frames can predate the effect's first draw; wait for the gradient.
            assert!(wait_until(FIRST_FRAME_WAIT, || {
                a.frame_count() > 3
                    && b.frame_count() > 3
                    && a.last().is_some_and(|f| f[0].r > 200)
                    && b.last().is_some_and(|f| f[0].r > 0 || f[11].r > 0 || f[11].b > 0)
            }));
            let (fa, fb) = (a.last().unwrap(), b.last().unwrap());
            if flows {
                // One gradient: red falls along d1 and goes on falling along d2.
                assert!(fa[0].r > 230, "the left edge is red: {fa:?}");
                assert!(fb[11].r < 25, "the right edge is blue: {fb:?}");
                assert!(fa.iter().all(|c| c.r >= 115), "d1 holds the left half: {fa:?}");
                assert!(fb.iter().all(|c| c.r <= 140), "d2 holds the right half: {fb:?}");
                let seam = i32::from(fa[11].r) - i32::from(fb[0].r);
                assert!(seam.abs() < 40, "the seam is smooth: {fa:?} {fb:?}");
            } else {
                // Each its own: both lights show the whole gradient.
                assert!(fa[0].r > 230 && fb[0].r > 230, "{fa:?} {fb:?}");
                assert!(fa[11].r < 25 && fb[11].r < 25, "{fa:?} {fb:?}");
            }
            engine.shutdown();
        }
    }

    /// Task 4 gate: the planner's samples are in the logical 320x200 canvas, the group's canvas
    /// is `SCENE_RENDER_SCALE` of that (80x50). Without the scaling every LED past the first
    /// quarter of the run clamps to the last column and the gradient collapses to one colour.
    #[test]
    fn a_12_and_36_led_flow_across_group_spreads_the_gradient_over_all_48_leds() {
        let engine = spawn(settings(60));
        let devices = vec![device(
            "d0",
            vec![zone(0, ZoneKind::Linear, 12), zone(1, ZoneKind::Linear, 36)],
        )];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d0".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d0"), slice_light("d0", 1, 0, 36)],
            devices,
            vec![group("a", &["d0#z0", "d0#z1s0"], Some(Box::new(GradientEffect)))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || {
            sink.last().is_some_and(|f| f.len() == 48 && f[0].r > 200)
        }));
        let frame = sink.last().expect("a frame");
        // Red falls from the first LED to the last, one step at a time over the whole run.
        assert!(frame[0].r > 230, "the first LED is red: {frame:?}");
        assert!(frame[47].r < 25, "the last LED is blue: {frame:?}");
        assert!((90..=165).contains(&frame[24].r), "the middle LED is half way: {frame:?}");
        assert!(
            frame.windows(2).all(|w| w[1].r <= w[0].r),
            "red never rises along the run: {frame:?}"
        );
        let distinct = frame.iter().map(|c| c.r).collect::<std::collections::BTreeSet<_>>().len();
        assert!(distinct >= 30, "{distinct} distinct values over 48 LEDs: {frame:?}");
        engine.shutdown();
    }

    #[test]
    fn a_member_naming_no_light_or_an_absent_device_is_skipped() {
        let (engine, devices, a, _b) = two_devices();
        engine.set_scene(
            // d2's light is declared, but its device is not in the list.
            lights_d1_d2(),
            devices.into_iter().take(1).collect(),
            vec![group("a", &["nope", "d2#z0", "d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || a.last().is_some_and(|f| all(&f, RED))));
        engine.shutdown();
    }

    #[test]
    fn group_states_lists_every_group_in_scene_order_and_is_empty_outside_scene_mode() {
        let (engine, devices, _a, _b) = two_devices();
        assert!(engine.group_states().is_empty());
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![group("b", &["d2#z0"], solid([0.0, 0.0, 1.0])), group("a", &["d1#z0"], None)],
            false,
        );
        let ids: Vec<String> = engine.group_states().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["b".to_string(), "a".to_string()]);
        engine.shutdown();
    }

    #[test]
    fn set_layout_after_set_scene_switches_back_to_the_layout() {
        let (engine, devices, a, _b) = two_devices();
        engine.set_scene(
            lights_d1_d2(),
            devices.clone(),
            vec![group("a", &["d1#z0"], solid([0.0, 0.0, 1.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || a.last().is_some_and(|f| all(&f, BLUE))));
        let lay = layout(
            vec![placed("d1", 0, 0.0, 0.0, 320.0, 200.0)],
            SamplingMode::AreaAverage,
            RenderScale::P100,
        );
        engine.set_layout(lay, devices);
        engine.set_effect(
            Box::new(SolidEffect::new([1.0, 0.0, 0.0], Arc::new(AtomicBool::new(false)))),
            &Props::new(),
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || a.last().is_some_and(|f| all(&f, RED))));
        assert!(engine.group_states().is_empty(), "layout mode has no groups");
        engine.shutdown();
    }

    /// #310 review 1: `set_layout` ends the scene, and the scene's effects drop behind the
    /// guard, outside the state lock, exactly like `set_scene` does. An effect whose `Drop`
    /// panics must not take the caller down, and the engine must keep rendering.
    #[test]
    fn set_layout_drops_the_scene_effects_outside_the_lock_and_the_engine_keeps_rendering() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group(
                "a",
                &["d1#z0"],
                Some(Box::new(FailingEffectThatPanicsInDrop)),
            )],
            false,
        );
        // The group failed, so the effect stays in the scene's state: the render thread
        // does not take a failed group's effect out again.
        assert!(
            wait_until(FIRST_FRAME_WAIT, || {
                engine.group_states().iter().any(|s| s.id == "a" && s.failed.is_some())
            }),
            "the group failed: {:?}",
            engine.group_states()
        );
        let before = sink.frame_count();

        // The effect panics when it is dropped. The call must return.
        let lay = layout(
            vec![placed("d1", 0, 0.0, 0.0, 320.0, 200.0)],
            SamplingMode::AreaAverage,
            RenderScale::P100,
        );
        engine.set_layout(lay, devices);

        // And the engine keeps rendering: the layout path delivers frames.
        engine.set_effect(
            Box::new(SolidEffect::new([1.0, 0.0, 0.0], Arc::new(AtomicBool::new(false)))),
            &Props::new(),
        );
        assert!(
            wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > before + 5),
            "the engine kept rendering: {} frames",
            sink.frame_count()
        );
        assert!(sink.last().is_some_and(|f| all(&f, RED)), "the layout renders red");
        engine.shutdown();
    }

    /// #310 review 2: a sink whose device the scene does not hold gets a black frame of
    /// its own length, like layout mode does. It never keeps the colours it was last
    /// sent.
    #[test]
    fn a_sink_whose_device_is_not_in_the_scene_gets_a_black_frame() {
        let engine = spawn(settings(60));
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        // The device is known first, so the sink's frames have 12 LEDs.
        let devices = vec![strip_device("d1", 12)];
        engine.set_layout(
            layout(
                vec![placed("d1", 0, 0.0, 0.0, 320.0, 200.0)],
                SamplingMode::AreaAverage,
                RenderScale::P100,
            ),
            devices.clone(),
        );
        engine.set_effect(
            Box::new(SolidEffect::new([1.0, 0.0, 0.0], Arc::new(AtomicBool::new(false)))),
            &Props::new(),
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, RED))));
        let before = sink.frame_count();

        // Now a scene without that device: the sink stays, the device does not.
        engine.set_scene(vec![], vec![strip_device("d2", 12)], vec![], false);
        assert!(
            wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > before + 2),
            "the sink whose device is gone still gets frames: {}",
            sink.frame_count()
        );
        let frame = sink.last().expect("a frame");
        assert_eq!(frame.len(), 12, "the sink's own length");
        assert!(
            all(&frame, Rgb8::BLACK),
            "the device is gone: it goes dark, never lit: {frame:?}"
        );
        engine.shutdown();
    }

    /// #310 review 3: a fade blends from each device's last sent frame, whatever mode
    /// sent it. Scene, then layout mode, then a scene with a fade: the fade starts from
    /// the layout's frame, never from the old scene's.
    #[test]
    fn a_fade_after_layout_mode_blends_from_the_layouts_last_frame() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());

        // Scene A: blue.
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group("a", &["d1#z0"], solid([0.0, 0.0, 1.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, BLUE))));

        // Layout mode: red.
        engine.set_layout(
            layout(
                vec![placed("d1", 0, 0.0, 0.0, 320.0, 200.0)],
                SamplingMode::AreaAverage,
                RenderScale::P100,
            ),
            devices.clone(),
        );
        engine.set_effect(
            Box::new(SolidEffect::new([1.0, 0.0, 0.0], Arc::new(AtomicBool::new(false)))),
            &Props::new(),
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, RED))));
        let before = sink.frame_count() as usize;

        // Scene B with a fade: the first frames blend from the layout's red.
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("a", &["d1#z0"], solid([0.0, 1.0, 0.0]))],
            true,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, GREEN))));
        let frames = sink.all_frames();
        let after = &frames[before..];
        assert!(
            !after.iter().any(|f| all(f, BLUE)),
            "the fade never shows the old scene's blue: {after:?}"
        );
        assert!(
            after.iter().any(|f| f[0].r > 0 && f[0].g > 0),
            "a frame between the layout's red and the new green: {after:?}"
        );
        engine.shutdown();
    }
}
