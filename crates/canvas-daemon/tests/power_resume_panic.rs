//! #145: a resume from sleep sends a frame again, so the strip comes back and the OpenRGB
//! controller re-enters custom mode.
//!
//! The daemon's power path pauses the engine on a suspend and resumes it on a wake
//! (`crates/canvas-daemon/src/power.rs`). The suspend writes one black frame; the wake must put
//! the running scene back on the strip, and the frame that carries it must make the controller
//! re-enter custom mode (the #5 shape), or the strip stays dark after the sleep.
//!
//! Scenes are the only render path (#310): the engine delivers the scene's frame every tick, so
//! there is no "delivery stopped" state to recover from. The fakes: the fake OpenRGB server
//! (`canvas_adapters::fake`) and the real engine, so the whole path is walked: the engine, the
//! sink, the writer task, the adapter and the server.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_core::spawn as spawn_engine;
use canvas_daemon::power::{self, PowerEvent, PowerWatcher};
use canvas_daemon::state::{App, DeviceSink};
use canvas_proto::api::Power;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::props::{InputNeeds, PropertySchema, Props};
use canvas_proto::traits::{Canvas, DeviceAdapter, Effect, Engine, EngineSettings, Inputs};
use canvas_proto::Rgb8;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The green the scene's effect paints.
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

/// Runs `effect` on the device's one zone, as one light in one group (#310).
fn run_on(
    engine: &Arc<dyn Engine>,
    device: &DeviceInfo,
    effect: Box<dyn canvas_proto::Effect>,
    props: &Props,
) {
    use canvas_proto::scene::{zone_light_id, Arrangement, GroupMode, Light};
    let light = Light {
        id: zone_light_id(&device.id, 0),
        device_id: device.id.clone(),
        zone_index: 0,
        leds: None,
        name: None,
        arrangement: Arrangement::Strip,
        reversed: false,
    };
    engine.set_scene(
        vec![light.clone()],
        vec![device.clone()],
        vec![canvas_proto::traits::EngineGroup {
            id: "all".into(),
            members: vec![light.id],
            mode: GroupMode::FlowAcross,
            brightness: 100,
            effect: Some(effect),
            props: props.clone(),
        }],
        false,
    );
}

/// An effect that paints the whole canvas green.
struct Green;

impl Effect for Green {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_green".into(),
            name: "Test Green".into(),
            description: "Paints the canvas green.".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
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
async fn a_resume_from_sleep_puts_the_scene_back_and_re_enters_custom_mode() {
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

    // The scene the engine runs: one group, one light, green.
    run_on(&engine, &device, Box::new(Green), &Props::new());

    // The scene's frame reaches the device.
    assert!(
        wait_for(|| controller_is(&server, GOOD)).await,
        "the scene's frame reached the device; colors = {:?}",
        server.colors(0)
    );

    // The suspend writes black: the lights go dark for the sleep.
    let mut watcher = PowerWatcher::new();
    power::handle(&app, &mut watcher, PowerEvent::Suspend).await;
    assert!(
        wait_for(|| controller_is(&server, Rgb8::BLACK)).await,
        "the suspend wrote black; colors = {:?}",
        server.colors(0)
    );

    let mode_writes_before = server.custom_mode_writes(0);
    power::handle(&app, &mut watcher, PowerEvent::Resume).await;

    // The resume puts the scene back on the strip...
    assert!(
        wait_for(|| controller_is(&server, GOOD)).await,
        "the scene came back after the resume; colors = {:?}",
        server.colors(0)
    );
    // ...and the controller re-enters custom mode, which the frame carried (the #5 shape: the
    // strip would stay on its built-in rainbow without it).
    assert!(
        wait_for(|| server.custom_mode_writes(0) > mode_writes_before).await,
        "the controller re-entered custom mode; writes = {}",
        server.custom_mode_writes(0)
    );

    writer.abort();
    engine.shutdown();
    let _ = std::fs::remove_dir_all(&home);
}
