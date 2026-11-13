//! A panicking `schema` or `init` in a scene's effect is caught by the engine's guard, never a
//! crash (#99). Scenes are the only render path now, so these run through `set_scene`.

mod common;

use std::sync::Arc;

use canvas_daemon::state::App;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::props::{InputNeeds, PropertySchema, Props};
use canvas_proto::scene::{zone_light_id, Arrangement, GroupMode, Light};
use canvas_proto::traits::{Canvas, Effect, Engine, EngineGroup, EngineSettings, Inputs};
use common::{FakeAdapter, FakeEngine};

/// An effect whose `schema` panics every time.
struct PanicSchemaDaemonEffect;

impl Effect for PanicSchemaDaemonEffect {
    fn schema(&self) -> PropertySchema {
        panic!("schema boom");
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {}
}

/// An effect whose schema and update are fine.
struct GoodDaemonEffect;

impl Effect for GoodDaemonEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_good_daemon".into(),
            name: "Test Good Daemon".into(),
            description: "Fine.".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {}
}

fn app_with_fake_engine(tag: &str) -> (Arc<App>, Arc<FakeEngine>) {
    let home = std::env::temp_dir().join(format!(
        "canvas-rgb-effect-apply-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp home is created");
    let store = canvas_daemon::storage::Store::at(&home);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    (app, engine)
}

/// The device, its light and the group a scene needs: one zone of four LEDs.
fn one_light() -> (Vec<Light>, Vec<DeviceInfo>, EngineGroup) {
    let device_id = DeviceId("d0".into());
    let light = Light {
        id: zone_light_id(&device_id, 0),
        device_id: device_id.clone(),
        zone_index: 0,
        leds: None,
        name: None,
        arrangement: Arrangement::Strip,
        reversed: false,
    };
    let device = DeviceInfo {
        id: device_id,
        name: "d0".into(),
        vendor: "test".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "zone0".into(),
            kind: ZoneKind::Linear,
            led_count: 4,
            min_leds: 4,
            max_leds: 4,
            resizable: false,
        }],
        connected: true,
    };
    let group = EngineGroup {
        id: "all".into(),
        members: vec![light.id.clone()],
        mode: GroupMode::FlowAcross,
        brightness: 100,
        effect: None,
        props: Props::new(),
    };
    (vec![light], vec![device], group)
}

/// #99 finding 4: a panicking `schema` becomes a failed group, never a crash, and the engine
/// keeps rendering the next scene.
#[tokio::test]
async fn a_panicking_schema_is_a_failed_group_not_a_crash() {
    let (_app, _engine) = app_with_fake_engine("schema");
    let engine: Arc<dyn Engine> = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    });
    let (lights, devices, mut group) = one_light();
    group.effect = Some(Box::new(PanicSchemaDaemonEffect));
    engine.set_scene(lights.clone(), devices.clone(), vec![group], false);
    let states = engine.group_states();
    assert_eq!(states.len(), 1);
    assert!(
        states[0].failed.as_deref().is_some_and(|text| text.contains("schema")),
        "the group failed, naming the schema: {:?}",
        states[0]
    );

    // The engine keeps running the next scene: a good effect in the same group.
    let (lights, devices, mut group) = one_light();
    group.effect = Some(Box::new(GoodDaemonEffect));
    engine.set_scene(lights, devices, vec![group], false);
    let states = engine.group_states();
    assert!(states[0].failed.is_none(), "the good group runs: {:?}", states[0]);
    engine.shutdown();
}

/// An effect whose `init` panics every time.
struct PanicInitDaemonEffect;

impl Effect for PanicInitDaemonEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_panic_init_daemon".into(),
            name: "Test Panic Init Daemon".into(),
            description: "Panics in init (#99).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {
        panic!("init boom");
    }

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {}
}

/// Waits until `check` is true, or the timeout expires. Returns whether it became true.
fn wait_until(timeout: std::time::Duration, check: impl Fn() -> bool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if check() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    check()
}

/// #99 finding 4: end to end through the real `set_scene` path and the real panic hook. A
/// panicking `init` is caught by the engine's guard, the hook logs "an effect panicked
/// (caught)" at WARN, and logging stays open: a later tracing line still reaches the log. No
/// port 9430 is opened.
#[tokio::test]
async fn a_panicking_init_keeps_logging_open_and_logs_the_caught_warn() {
    let home = std::env::temp_dir().join(format!(
        "canvas-rgb-effect-init-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp home is created");
    let store = canvas_daemon::storage::Store::at(&home);
    let engine: Arc<dyn Engine> = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    });
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let _app = Arc::new(App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));

    let log = common::LogBuf::default();
    let _guard = common::LogBuf::install(&log);
    canvas_daemon::exit_log::install_panic_hook();

    let (lights, devices, mut group) = one_light();
    group.effect = Some(Box::new(PanicInitDaemonEffect));
    engine.set_scene(lights, devices, vec![group], false);
    assert!(
        engine.group_states()[0].failed.is_some(),
        "a caught init panic is a failed group"
    );

    // The WARN was emitted by the real hook while the guarded init unwound.
    assert!(
        wait_until(std::time::Duration::from_secs(3), || log.text().contains("an effect panicked (caught)")),
        "the WARN line was emitted: {:?}",
        log.text()
    );

    // Logging is still open: a later tracing line reaches the log.
    tracing::info!("after the panic logging is still open");
    assert!(
        wait_until(std::time::Duration::from_secs(3), || log.text().contains("logging is still open")),
        "a later log line reached the log: {:?}",
        log.text()
    );

    engine.shutdown();
}
