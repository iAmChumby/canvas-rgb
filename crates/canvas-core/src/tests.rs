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
use canvas_proto::layout::{Layout, PlacedZone, RenderScale, SamplingMode, ZoneShape};
use canvas_proto::props::{InputNeeds, PropKind, PropertyDef, PropertySchema, Props};
use canvas_proto::traits::{Canvas, Effect, EngineSettings, FrameSink, PreviewSink};
use canvas_proto::CANVAS_HEIGHT;
use canvas_proto::CANVAS_WIDTH;

use crate::spawn;

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
    }
}

fn settings(fps: u32) -> EngineSettings {
    EngineSettings {
        fps,
        power: Power::Running,
        brightness: 100,
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

    assert!(wait_until(Duration::from_secs(3), || sink.frame_count() > 0));

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

    assert!(wait_until(Duration::from_secs(3), || sink.frame_count() > 0));
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
    assert!(wait_until(Duration::from_secs(3), || sink.frame_count() > 0));
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
    assert!(wait_until(Duration::from_secs(3), || sink.frame_count() > 0));

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
    assert!(wait_until(Duration::from_secs(3), || sink.frame_count() > 0));

    engine.set_power(Power::Paused);
    assert!(wait_until(Duration::from_secs(2), || sink
        .last()
        .map(|f| f.iter().all(|c| *c == Rgb8::BLACK))
        .unwrap_or(false)));

    engine.set_power(Power::Running);
    let frames_at_resume = sink.frame_count();
    assert!(
        wait_until(Duration::from_secs(3), || sink.frame_count() > frames_at_resume),
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
    assert!(wait_until(Duration::from_secs(3), || sink.frame_count() > 0));

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
    assert!(wait_until(Duration::from_secs(3), || sink.last().is_some_and(|f| f.len() == 8)));
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
    assert!(wait_until(Duration::from_secs(3), || {
        let fps = engine.stats().fps_measured;
        (fps - 30.0).abs() <= 1.0
    }));

    engine.set_fps(20);
    assert!(
        wait_until(Duration::from_secs(3), || {
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
    assert!(wait_until(Duration::from_secs(3), || {
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
    assert!(wait_until(Duration::from_secs(3), || {
        let fps = engine.stats().fps_measured;
        (fps - 10.0).abs() <= 1.0
    }), "measured {}", engine.stats().fps_measured);

    // Above the maximum: clamped to 60.
    engine.set_fps(500);
    assert!(wait_until(Duration::from_secs(3), || {
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
    assert!(wait_until(Duration::from_secs(3), || sink.frame_count() > 0));
    assert_eq!(sink.last().expect("a frame")[0], Rgb8::new(255, 255, 255));

    engine.set_brightness(50);
    assert!(wait_until(Duration::from_secs(3), || sink
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

    assert!(wait_until(Duration::from_secs(3), || {
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

    assert!(wait_until(Duration::from_secs(3), || preview.count() > 0));
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
    assert!(wait_until(Duration::from_secs(3), || preview.count() > 0));

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
    assert!(wait_until(Duration::from_secs(3), || sink.frame_count() > 0));
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
