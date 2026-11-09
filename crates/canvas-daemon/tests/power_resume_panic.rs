//! #145: after an effect panics, a resume from sleep sends the last good frame again.
//!
//! The daemon's power path pauses the engine on a suspend and resumes it on a wake
//! (`crates/canvas-daemon/src/power.rs`). A panicked effect stops the delivery (#99), so
//! without the fix the lights stay black after the wake, the OpenRGB controllers never
//! re-enter custom mode (the #5 shape) and the adapter's keepalive never runs: the
//! frames that would carry them never go out.
//!
//! The fakes: the fake OpenRGB server (`canvas_adapters::fake`) and the real engine, so
//! the whole path is walked: the engine, the sink, the writer task, the adapter and the
//! server.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_core::spawn as spawn_engine;
use canvas_daemon::power::{self, PowerEvent, PowerWatcher};
use canvas_daemon::state::{App, DeviceSink};
use canvas_proto::api::Power;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::{CaseForm, Layout, PlacedZone, RenderScale, SamplingMode, ZoneShape};
use canvas_proto::props::{InputNeeds, PropertySchema, Props};
use canvas_proto::traits::{Canvas, DeviceAdapter, Effect, EngineSettings, Inputs};
use canvas_proto::Rgb8;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The green of the good frame the effect paints.
const GOOD: Rgb8 = Rgb8 { r: 0, g: 255, b: 0 };

fn stick() -> DeviceInfo {
    DeviceInfo {
        id: DeviceId("openrgb:dram".into()),
        name: "DDR5".into(),
        vendor: "fake".into(),
        kind: DeviceKind::Dram,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "dram".into(),
        location: "smbus".into(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "RGB".into(),
            kind: ZoneKind::Linear,
            led_count: 8,
            min_leds: 8,
            max_leds: 8,
            resizable: false,
        }],
        connected: true,
    }
}

/// A layout that places the device's zone over the whole canvas.
fn layout_for(device: &DeviceInfo) -> Layout {
    Layout {
        id: "test".into(),
        name: "test".into(),
        render_scale: RenderScale::P100,
        sampling: SamplingMode::AreaAverage,
        zones: vec![PlacedZone {
            device_id: device.id.clone(),
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
        case_form: CaseForm::None,
        landmarks: Vec::new(),
    }
}

/// An effect that paints one good frame and then panics (#145).
///
/// The first update paints the canvas, so the engine has a good frame to keep. The
/// second one panics, and the engine stops delivering (#99).
struct OneFrameThenPanic {
    updates: Arc<AtomicU32>,
}

impl Effect for OneFrameThenPanic {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_one_frame_then_panic".into(),
            name: "Test One Frame Then Panic".into(),
            description: "Paints one frame, then panics (#145).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        let n = self.updates.fetch_add(1, Ordering::AcqRel);
        if n >= 1 {
            panic!("update boom after a good frame");
        }
        canvas.fill([0.0, 1.0, 0.0]);
    }
}

/// Waits until `check` is true, or the deadline passes. Returns whether it became true.
async fn wait_for(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    check()
}

/// Whether the controller holds the given colour on every LED.
fn controller_is(server: &FakeOpenRgbServer, color: Rgb8) -> bool {
    let colors = server.colors(0);
    !colors.is_empty() && colors.iter().all(|c| *c == color)
}

#[tokio::test]
async fn a_resume_from_sleep_sends_the_last_good_frame_again_after_an_effect_panic() {
    let home = std::env::temp_dir().join(format!(
        "canvas-rgb-power-resume-panic-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();

    let server = FakeOpenRgbServer::start(vec![stick()])
        .await
        .expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");

    let device = stick();
    let engine = spawn_engine(EngineSettings {
        fps: 30,
        power: Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    });
    engine.set_layout(layout_for(&device), vec![device.clone()]);

    let (quit_tx, _quit_rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        canvas_daemon::storage::Store::at(&home),
        engine.clone(),
        Box::new(adapter),
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
        "0.0.0-test".into(),
        quit_tx,
    ));

    // The engine's sink: a bounded channel into the writer task that writes to the
    // adapter, the same path the daemon builds in `main.rs`.
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<Rgb8>>(1);
    let writer = tokio::spawn(canvas_daemon::writer::writer_loop(
        app.adapter.clone(),
        device.id.clone(),
        rx,
        Arc::clone(&app.last_frame),
    ));
    engine.add_sink(
        device.id.clone(),
        Arc::new(DeviceSink::new(device.id.clone(), tx)),
    );

    let updates = Arc::new(AtomicU32::new(0));
    engine.set_effect(
        Box::new(OneFrameThenPanic {
            updates: updates.clone(),
        }),
        &Props::new(),
    );

    // The good frame reaches the device.
    assert!(
        wait_for(|| controller_is(&server, GOOD)).await,
        "the good frame reached the device; colors = {:?}",
        server.colors(0)
    );
    // The effect panics, and the delivery stops: the device keeps the good frame.
    assert!(
        wait_for(|| updates.load(Ordering::Acquire) >= 2).await,
        "the effect panicked"
    );

    let mut watcher = PowerWatcher::new();
    power::handle(&app, &mut watcher, PowerEvent::Suspend).await;
    // The suspend writes black: the lights go dark for the sleep.
    assert!(
        wait_for(|| controller_is(&server, Rgb8::BLACK)).await,
        "the suspend wrote black; colors = {:?}",
        server.colors(0)
    );

    let mode_writes_before = server.custom_mode_writes(0);
    power::handle(&app, &mut watcher, PowerEvent::Resume).await;

    // The resume sends the last good frame again: the colours come back...
    assert!(
        wait_for(|| controller_is(&server, GOOD)).await,
        "the last good frame came back after the resume; colors = {:?}",
        server.colors(0)
    );
    // ...and the controller re-enters custom mode, which the frame carried (the #5
    // shape: the sticks would stay on their built-in rainbow without it).
    assert!(
        wait_for(|| server.custom_mode_writes(0) > mode_writes_before).await,
        "the controller re-entered custom mode; writes = {}",
        server.custom_mode_writes(0)
    );

    writer.abort();
    engine.shutdown();
    let _ = std::fs::remove_dir_all(&home);
}
