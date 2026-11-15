//! Patterns in the daemon (Patterns plan, Tasks 4, 5 and their neighbours): the file, the
//! effects list, the routes and the delete guard.
//!
//! A real engine with two fake devices and a recording sink on each, so a route is proved by what
//! the sinks are handed rather than by what the handler says. No device, no port 9430: the router
//! is driven in-process. Nothing here touches hardware.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::state::App;
use canvas_proto::api::{
    routes, DeletePatternResponse, EffectKind, EffectsResponse, PatternsResponse,
};
use canvas_proto::color::Rgb8;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::LedSlice;
use canvas_proto::pattern::{BuiltOn, Frame, Motion, Pattern};
use canvas_proto::props::{PropValue, Props};
use canvas_proto::scene::{
    Arrangement, Group, GroupMode, Light, LightsFile, Plays, Scene, ScenesFile, LIGHTS_SCHEMA,
    SCENES_SCHEMA,
};
use canvas_proto::traits::{Engine, EngineSettings, FrameSink};
use common::FakeAdapter;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const A: &str = "fake:a";
const B: &str = "fake:b";
const LEDS: u32 = 12;
/// The pattern every rig saves: one red LED on a ring of twelve, and the rest off.
const INTAKE: &str = "pattern:Intake";

struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-patterns-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp directory is created");
        Self { path }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The most frames a [`Rec`] keeps; the oldest go first.
const REC_HISTORY: usize = 4096;

/// A sink that keeps every frame it is given (the newest [`REC_HISTORY`]) and counts them.
#[derive(Default)]
struct Rec {
    frames: Mutex<std::collections::VecDeque<Vec<Rgb8>>>,
    seen: std::sync::atomic::AtomicUsize,
}

impl Rec {
    fn last(&self) -> Option<Vec<Rgb8>> {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).back().cloned()
    }

    fn seen(&self) -> usize {
        self.seen.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The frames handed over after the first `since` of them.
    fn since(&self, since: usize) -> Vec<Vec<Rgb8>> {
        let frames = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        let total = self.seen();
        let kept_from = total - frames.len();
        frames.iter().skip(since.saturating_sub(kept_from)).cloned().collect()
    }
}

impl FrameSink for Rec {
    fn submit(&self, colors: Vec<Rgb8>) {
        let mut frames = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        if frames.len() == REC_HISTORY {
            frames.pop_front();
        }
        frames.push_back(colors);
        self.seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn device(id: &str) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.into()),
        name: id.into(),
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
            led_count: LEDS,
            min_leds: LEDS,
            max_leds: LEDS,
            resizable: false,
        }],
        connected: true,
    }
}

fn light(device: &str) -> Light {
    Light {
        id: format!("{device}#z0"),
        device_id: DeviceId(device.into()),
        zone_index: 0,
        leds: None,
        name: None,
        arrangement: Arrangement::Ring,
        reversed: false,
        kind: None,
    }
}

fn color(rgb: (u8, u8, u8)) -> Props {
    let mut props = Props::new();
    props.insert("color".into(), PropValue::Color(Rgb8::new(rgb.0, rgb.1, rgb.2)));
    props
}

fn group(id: &str, members: &[&str], effect: &str, props: Props) -> Group {
    Group {
        id: id.into(),
        name: id.to_uppercase(),
        members: members.iter().map(|m| m.to_string()).collect(),
        plays: Plays::Effect { effect: effect.into(), props },
        mode: GroupMode::FlowAcross,
        brightness: 100,
    }
}

const RED: (u8, u8, u8) = (200, 0, 0);

fn rgb(c: (u8, u8, u8)) -> Rgb8 {
    Rgb8::new(c.0, c.1, c.2)
}

/// A pattern built on a ring of twelve with LED 0 lit and every other LED off.
fn pattern(name: &str) -> Pattern {
    let mut colors = vec![Rgb8::BLACK; LEDS as usize];
    colors[0] = rgb(RED);
    Pattern {
        id: format!("pattern:{name}"),
        built_on: BuiltOn { led_count: LEDS, arrangement: Arrangement::Ring, grid: None },
        frames: vec![Frame { colors, hold: 0, extra: serde_json::Map::new() }],
        motion: Motion::none(),
        extra: serde_json::Map::new(),
    }
}

/// The scenes: `main` (active) plays the pattern on the right group and a solid red on the left;
/// `other` plays the pattern on both lights, and is **not** active.
fn scenes_file() -> ScenesFile {
    ScenesFile {
        schema_version: SCENES_SCHEMA,
        active: "main".into(),
        scenes: vec![
            Scene {
                id: "main".into(),
                name: "Main".into(),
                rev: 1,
                groups: vec![
                    group("left", &["fake:a#z0"], "solid", color(RED)),
                    group("right", &["fake:b#z0"], INTAKE, Props::new()),
                ],
            },
            Scene {
                id: "other".into(),
                name: "Other".into(),
                rev: 1,
                groups: vec![group("all", &["fake:a#z0", "fake:b#z0"], INTAKE, Props::new())],
            },
        ],
    }
}

struct Rig {
    router: axum::Router,
    app: Arc<App>,
    a: Arc<Rec>,
    b: Arc<Rec>,
    home: TempHome,
}

/// The daemon's state around a real engine, the two fake devices and the saved pattern.
async fn rig() -> Rig {
    rig_with_patterns(vec![pattern("Intake")], scenes_file()).await
}

/// [`rig`] with `patterns` already saved in `patterns.json` and `scenes` active.
async fn rig_with_patterns(patterns: Vec<Pattern>, scenes: ScenesFile) -> Rig {
    let home = TempHome::new();
    let effects = home.path.join("effects");
    std::fs::create_dir_all(&effects).expect("the effects folder is created");
    // The file the daemon reads at start-up, written before the app is built.
    if !patterns.is_empty() {
        canvas_daemon::patterns::write_file(&home.path, &patterns).expect("patterns.json is written");
    }
    let mut store = canvas_daemon::storage::Store::at(&home.path);
    store.lights = LightsFile {
        chains: Vec::new(),
        schema_version: LIGHTS_SCHEMA,
        lights: vec![light(A), light(B)],
    };
    store.scenes = scenes;
    let engine: Arc<dyn Engine> = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    });
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        Arc::clone(&engine),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
        tx,
    );
    app.effects_dir = effects;
    let app = Arc::new(app);
    *app.devices.write().await = vec![device(A), device(B)];
    let (a, b) = (Arc::new(Rec::default()), Arc::new(Rec::default()));
    engine.add_sink(DeviceId(A.into()), a.clone());
    engine.add_sink(DeviceId(B.into()), b.clone());
    // The way start-up applies the saved scenes.
    canvas_daemon::scenes::restore_active(&app).await;
    Rig { router: canvas_daemon::api::router(app.clone()), app, a, b, home }
}

async fn send(
    rig: &Rig,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"));
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = rig.router.clone().oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn route(template: &str, pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .fold(template.to_string(), |path, (key, value)| path.replace(&format!("{{{key}}}"), value))
}

/// Waits (real time) until `check` is true, for at most ten seconds.
async fn wait_until(mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the condition did not come true");
}

/// Waits (real time) until `sink`'s newest frame has `LEDS` LEDs that are all `want`.
async fn shows(sink: &Rec, want: Rgb8, what: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if sink.last().is_some_and(|f| f.len() == LEDS as usize && f.iter().all(|c| *c == want)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "{what}: never showed {want:?}; last frame starts {:?}",
        sink.last().map(|f| f[..3.min(f.len())].to_vec())
    );
}

/// The group state of the active scene, by group id, from `GET /api/v1/scenes`.
async fn group_states(rig: &Rig) -> std::collections::BTreeMap<String, Option<String>> {
    let (status, body) = send(rig, "GET", routes::SCENES, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let scene: canvas_proto::api::ScenesResponse = serde_json::from_value(body).unwrap();
    scene
        .group_states
        .into_iter()
        .map(|state| (state.id, state.failed))
        .collect()
}

// ---------------------------------------------------------------- Task 4: as an effect

#[tokio::test]
async fn a_pattern_is_in_the_effects_list_with_its_kind_and_built_on() {
    let rig = rig().await;
    let (status, body) = send(&rig, "GET", routes::EFFECTS, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: EffectsResponse = serde_json::from_value(body).unwrap();
    let pattern = list
        .effects
        .iter()
        .find(|e| e.id == INTAKE)
        .unwrap_or_else(|| panic!("{INTAKE} is in the list: {:?}", list.effects.iter().map(|e| &e.id).collect::<Vec<_>>()));
    assert_eq!(pattern.kind, EffectKind::Pattern);
    assert_eq!(pattern.name, "Intake");
    assert_eq!(pattern.description, "Pattern, ring of 12");
    assert_eq!(
        pattern.built_on,
        Some(BuiltOn { led_count: LEDS, arrangement: Arrangement::Ring, grid: None })
    );
    assert!(pattern.needs.screen == false && pattern.needs.audio == false);
    // A canvas effect says so, and names no device.
    let solid = list.effects.iter().find(|e| e.id == "solid").expect("solid is in the list");
    assert_eq!(solid.kind, EffectKind::Canvas);
    assert_eq!(solid.built_on, None);
}

#[tokio::test]
async fn a_pattern_can_be_activated_like_an_effect() {
    let rig = rig().await;
    shows(&rig.a, rgb(RED), "the left group before").await;
    let (status, answer) = send(&rig, "POST", &route(routes::EFFECT_ACTIVATE, &[("id", INTAKE)]), Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["id"], serde_json::json!(INTAKE));

    // Every group of the active scene plays it now: the route finds the pattern's schema, builds
    // it and writes the groups as for any effect. No group failed to build.
    let scene = rig.app.store.read().await.scenes.scenes[0].clone();
    let plays: Vec<&str> = scene
        .groups
        .iter()
        .map(|g| match &g.plays {
            Plays::Effect { effect, .. } => effect.as_str(),
            Plays::Off => "off",
        })
        .collect();
    assert_eq!(plays, [INTAKE, INTAKE], "both groups play the pattern");
    assert!(
        group_states(&rig).await.values().all(Option::is_none),
        "no group is dark: the pattern built"
    );
    // The group runs on the engine: its device keeps being handed frames of its own length.
    // (What those LEDs show is the engine's pattern path, Task 3 of the plan.)
    let since = rig.b.seen();
    wait_until(|| rig.b.since(since).iter().any(|f| f.len() == LEDS as usize)).await;
}

#[tokio::test]
async fn a_pattern_can_be_a_favourite() {
    let rig = rig().await;
    let body = serde_json::json!({ "favorite": true });
    let (status, answer) = send(&rig, "PUT", &route(routes::EFFECT_FAVORITE, &[("id", INTAKE)]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let (_, list) = send(&rig, "GET", routes::EFFECTS, None).await;
    let list: EffectsResponse = serde_json::from_value(list).unwrap();
    let pattern = list.effects.iter().find(|e| e.id == INTAKE).unwrap();
    assert!(pattern.favorite, "{:?}", pattern);
    let stored = rig.app.store.read().await.effects.favorites.clone();
    assert!(stored.iter().any(|f| f == INTAKE), "{stored:?}");
}

#[tokio::test]
async fn a_pattern_cannot_be_a_stack_layer() {
    let rig = rig().await;
    let stack = serde_json::json!({
        "id": "stack:Marked",
        "layers": [{ "id": "l1", "name": "Marked", "effect": INTAKE }]
    });
    let (status, answer) = send(&rig, "PUT", &route(routes::STACK, &[("id", "stack:Marked")]), Some(stack)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert_eq!(answer["error"], "validation");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains(INTAKE), "the sentence names the pattern: {message}");
    assert!(message.contains("pattern"), "{message}");
    assert!(!rig.app.stacks.get("stack:Marked").is_some(), "nothing was saved");
}

#[tokio::test]
async fn a_pattern_has_no_presets_and_no_starting_values() {
    let rig = rig().await;
    let preset = serde_json::json!({ "name": "Evening", "props": {} });
    let (status, answer) =
        send(&rig, "POST", &route(routes::EFFECT_PRESETS, &[("id", INTAKE)]), Some(preset)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains(INTAKE) && message.contains("no presets"), "{message}");

    let defaults = serde_json::json!({});
    let (status, answer) =
        send(&rig, "PATCH", &route(routes::EFFECT_DEFAULTS, &[("id", INTAKE)]), Some(defaults)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains(INTAKE) && message.contains("no settings"), "{message}");
    assert!(
        !rig.app.store.read().await.effects.saved.contains_key(INTAKE),
        "no starting value was written"
    );
    assert!(
        !rig.app.store.read().await.effects.presets.contains_key(INTAKE),
        "no preset was written"
    );
}

#[tokio::test]
async fn the_thumbnail_of_a_pattern_is_refused_not_black() {
    let rig = rig().await;
    let (status, answer) =
        send(&rig, "GET", &route(routes::EFFECT_THUMBNAIL, &[("id", INTAKE)]), None).await;
    // A pattern has no picture: the tile route refuses it rather than rendering a black tile,
    // and the sentence says so. (The page draws the pattern's dots instead.)
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains(INTAKE), "{message}");
    assert!(message.contains("pattern"), "{message}");
}

#[test]
fn the_snapshot_keeps_patterns_json() {
    let home = std::env::temp_dir().join(format!(
        "canvas-rgb-patterns-snapshot-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    canvas_daemon::patterns::write_file(&home, &[pattern("Intake")]).expect("patterns.json is written");
    let dir = canvas_daemon::storage::snapshot(&home, "test").expect("the snapshot is written");
    let copy = dir.join("patterns.json");
    assert!(copy.exists(), "the snapshot holds patterns.json: {:?}", std::fs::read_dir(&dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name()).collect::<Vec<_>>()));
    assert!(std::fs::read_to_string(&copy).unwrap().contains(INTAKE));
    let _ = std::fs::remove_dir_all(&home);
}

/// A light that is a part of a split zone: the frame writer skips the LEDs before its start.
#[allow(dead_code)]
fn part(device: &str, start: u32, count: u32) -> Light {
    Light {
        id: format!("{device}#z0p1"),
        device_id: DeviceId(device.into()),
        zone_index: 0,
        leds: Some(LedSlice { start, count }),
        name: None,
        arrangement: Arrangement::Ring,
        reversed: false,
        kind: None,
    }
}
