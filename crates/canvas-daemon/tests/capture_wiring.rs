//! The daemon's capture reaches the engines (#1).
//!
//! The capture threads started, but nothing ever gave the capture to the engines, so
//! Audio Spectrum and Screen Ambience drew black. These tests run on the fakes: no
//! device, no capture thread, and no port.

mod common;

use canvas_daemon::state::App;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::{Layout, PlacedZone, RenderScale, SamplingMode, ZoneShape};
use canvas_proto::props::Props;
use canvas_proto::traits::{
    AudioSpectrum, Engine, EngineSettings, FrameSink, InputSource, ScreenFrame,
};
use canvas_proto::Rgb8;
use common::{FakeAdapter, FakeEngine};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The source of the daemon's main, the startup wiring included.
///
/// The scan reads the source of the daemon's entry point, so a startup that forgets to
/// wire the capture is caught here and not only on the lights.
const MAIN: &str = include_str!("../src/main.rs");

// ---------------------------------------------------------------- the harness

/// The daemon's state and the two engines it talks to.
struct Harness {
    app: Arc<App>,
    /// The lights' engine.
    engine: Arc<FakeEngine>,
    /// The stack preview's engine.
    preview_engine: Arc<FakeEngine>,
}

/// The daemon on a temporary home, with the two engines as fakes.
fn build(tag: &str) -> Harness {
    let home =
        std::env::temp_dir().join(format!("canvas-rgb-capture-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp home is created");
    let store = canvas_daemon::storage::Store::at(&home);
    let engine = Arc::new(FakeEngine::default());
    let preview_engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    // The preview engine is a fake, so a test reads what it was given.
    app.stack_preview.install_engine(preview_engine.clone());
    Harness {
        app: Arc::new(app),
        engine,
        preview_engine,
    }
}

/// The daemon's startup wiring: the engines read the source the app has.
///
/// The daemon does this once, before the saved effect is restored (see `main.rs`).
fn wire_like_the_daemon(app: &Arc<App>) {
    app.set_input_source(Some(app.capture_source()));
}

// ---------------------------------------------------------------- the tests

/// The daemon's startup must give the engines the capture (#1).
///
/// The call lives in the daemon's entry point, so the scan reads it there: a startup
/// that stops wiring the capture fails this test instead of leaving the effects dark.
#[test]
fn the_daemon_wires_the_capture_at_startup() {
    assert!(
        MAIN.contains("set_input_source(Some(app.capture_source()))"),
        "the daemon's startup must give the engines the capture"
    );
}

/// The lights' engine reads the daemon's own capture after the startup wiring.
#[tokio::test]
async fn the_lights_engine_has_the_capture_after_the_startup_wiring() {
    let harness = build("lights");
    wire_like_the_daemon(&harness.app);

    let reached = harness
        .engine
        .input_source
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("the lights' engine was given the capture");
    assert!(
        Arc::ptr_eq(&reached, &harness.app.capture_source()),
        "the lights' engine reads the daemon's own capture"
    );
    assert!(
        harness.app.input_source().is_some(),
        "the app keeps the source it gave the engines"
    );
}

/// The stack preview's engine reads the same capture when it is made.
#[tokio::test]
async fn the_stack_preview_engine_has_the_capture_after_the_startup_wiring() {
    let harness = build("preview");
    wire_like_the_daemon(&harness.app);

    // The preview engine is configured the first time the builder asks for it.
    let preview = harness.app.stack_preview.engine(&harness.app).await;
    let fake: Arc<dyn Engine> = harness.preview_engine.clone();
    assert!(
        Arc::ptr_eq(&preview, &fake),
        "the preview engine is the fake one"
    );

    let reached = harness
        .preview_engine
        .input_source
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("the preview engine was given the capture");
    assert!(
        Arc::ptr_eq(&reached, &harness.app.capture_source()),
        "the preview engine reads the daemon's own capture"
    );
}

/// An effect that needs nothing, so an apply never starts a capture.
fn solid_effect() -> Box<dyn canvas_proto::traits::Effect> {
    canvas_effects::create("solid").expect("the built-in effect exists")
}

/// The capture survives an effect switch: the apply path never takes it away.
#[tokio::test]
async fn the_capture_survives_an_effect_switch() {
    let harness = build("switch");
    wire_like_the_daemon(&harness.app);
    let before = harness
        .engine
        .input_source
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("the lights' engine was given the capture");

    // Two effects in a row, the way the daemon applies them.
    canvas_daemon::apply_built(&harness.app, solid_effect(), &Props::new())
        .await
        .expect("the first effect is applied");
    canvas_daemon::apply_built(&harness.app, solid_effect(), &Props::new())
        .await
        .expect("the second effect is applied");

    let after = harness
        .engine
        .input_source
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("the lights' engine still has the capture");
    assert!(
        Arc::ptr_eq(&before, &after),
        "the effect switch did not take the capture away"
    );
}

// ---------------------------------------------------------------- the frame

/// An input source that publishes a known spectrum: one loud band.
struct SpectrumSource {
    audio: Arc<AudioSpectrum>,
}

impl SpectrumSource {
    fn new() -> Self {
        let centers: Vec<f32> = (0..32).map(|i| 60.0 * 2f32.powf(i as f32 / 4.0)).collect();
        let magnitudes: Vec<f32> = (0..32).map(|i| if i == 16 { 1.0 } else { 0.0 }).collect();
        Self {
            audio: Arc::new(AudioSpectrum {
                centers_hz: centers,
                magnitudes,
            }),
        }
    }
}

impl InputSource for SpectrumSource {
    fn screen(&self) -> Option<Arc<ScreenFrame>> {
        None
    }

    fn audio(&self) -> Option<Arc<AudioSpectrum>> {
        Some(Arc::clone(&self.audio))
    }
}

/// A sink that keeps the frames it is given.
#[derive(Default)]
struct RecordingSink {
    frames: Mutex<Vec<Vec<Rgb8>>>,
}

impl RecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Whether any frame carried a colour that is not black.
    fn lit(&self) -> bool {
        self.frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|frame| frame.iter().any(|color| *color != Rgb8::BLACK))
    }
}

impl FrameSink for RecordingSink {
    fn submit(&self, colors: Vec<Rgb8>) {
        let mut frames = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        frames.push(colors);
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

fn zone(index: u32, leds: u32) -> ZoneInfo {
    ZoneInfo {
        index,
        name: format!("zone{index}"),
        kind: ZoneKind::Linear,
        led_count: leds,
        min_leds: leds,
        max_leds: leds,
        resizable: false,
    }
}

fn device(id: &str, leds: u32) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.to_string()),
        name: id.to_string(),
        vendor: "test".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones: vec![zone(0, leds)],
        connected: true,
    }
}

fn layout() -> Layout {
    Layout {
        id: "test".into(),
        name: "test".into(),
        render_scale: RenderScale::P100,
        sampling: SamplingMode::AreaAverage,
        zones: vec![PlacedZone {
            device_id: DeviceId("d0".to_string()),
            zone_index: 0,
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 200.0,
            rotation_deg: 0.0,
            reversed: false,
            visible: true,
            shape: ZoneShape::Auto,
            label: None,
            leds: None,
            part: None,
            locked: false,
        }],
        effect: None,
        case_form: canvas_proto::layout::CaseForm::None,
        landmarks: Vec::new(),
    }
}

/// The effect that needs the audio draws a frame that is not black when the engine
/// reads a spectrum from its input source.
///
/// This is the chain the daemon got wrong: the engine asks the source for the audio, and
/// the effect draws from it. With no source the canvas stays black (#1).
#[tokio::test]
async fn an_audio_effect_reads_the_source_and_lights_the_sink() {
    let home =
        std::env::temp_dir().join(format!("canvas-rgb-capture-frame-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp home is created");
    let store = canvas_daemon::storage::Store::at(&home);
    let engine: Arc<dyn Engine> = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
    });
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));

    // The daemon's wiring, with the fake source the test can drive.
    app.set_input_source(Some(Arc::new(SpectrumSource::new())));

    engine.set_layout(layout(), vec![device("d0", 12)]);
    let sink = RecordingSink::new();
    engine.add_sink(DeviceId("d0".into()), sink.clone());

    // The effect that needs the audio.
    let effect = canvas_effects::create("audio_spectrum").expect("the built-in effect exists");
    engine.set_effect(effect, &Props::new());

    assert!(
        wait_until(Duration::from_secs(10), || sink.lit()),
        "the effect drew from the spectrum the source published"
    );
    engine.shutdown();
}
