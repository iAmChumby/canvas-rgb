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
use canvas_proto::pattern::{BuiltOn, FitKind, Frame, LightFit, Motion, Pattern};
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
    device_of(id, LEDS)
}

/// A device whose one linear zone has `leds` LEDs.
fn device_of(id: &str, leds: u32) -> DeviceInfo {
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
            led_count: leds,
            min_leds: leds,
            max_leds: leds,
            resizable: false,
        }],
        connected: true,
    }
}

fn light(device: &str) -> Light {
    light_of(device, Arrangement::Ring)
}

/// The whole zone of `device` as one light, in `arrangement`.
fn light_of(device: &str, arrangement: Arrangement) -> Light {
    Light {
        id: format!("{device}#z0"),
        device_id: DeviceId(device.into()),
        zone_index: 0,
        leds: None,
        name: None,
        arrangement,
        reversed: false,
        kind: None,
        copies: None,
        first_led: None,
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
        sequence: None,
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
        frames: vec![Frame { colors, hold: 0, transition: None, extra: serde_json::Map::new() }],
        motion: Motion::none(),
        playback: None,
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

/// A clock the test moves by hand: a draft lease is tested without waiting.
#[derive(Clone)]
struct TestClock {
    now: Arc<Mutex<std::time::Duration>>,
}

impl TestClock {
    fn new() -> Self {
        Self { now: Arc::new(Mutex::new(std::time::Duration::ZERO)) }
    }

    /// The clock the draft reads.
    fn clock(&self) -> canvas_daemon::draft::Clock {
        let now = Arc::clone(&self.now);
        Arc::new(move || *now.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    /// Moves the clock on.
    fn advance(&self, by: std::time::Duration) {
        let mut now = self.now.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *now += by;
    }
}

struct Rig {
    router: axum::Router,
    app: Arc<App>,
    a: Arc<Rec>,
    b: Arc<Rec>,
    /// Every device's sink, by device id.
    sinks: std::collections::BTreeMap<String, Arc<Rec>>,
    clock: TestClock,
    home: TempHome,
}

/// The daemon's state around a real engine, the two fake devices and the saved pattern.
async fn rig() -> Rig {
    rig_with_patterns(vec![pattern("Intake")], scenes_file()).await
}

/// [`rig`] with `patterns` already saved in `patterns.json` and `scenes` active.
async fn rig_with_patterns(patterns: Vec<Pattern>, scenes: ScenesFile) -> Rig {
    rig_full(
        patterns,
        scenes,
        vec![device(A), device(B)],
        vec![light(A), light(B)],
    )
    .await
}

/// A rig of any devices and lights, with a recording sink on each device.
async fn rig_full(
    patterns: Vec<Pattern>,
    scenes: ScenesFile,
    devices: Vec<DeviceInfo>,
    lights: Vec<Light>,
) -> Rig {
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
        lights,
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
    // A lease is driven by the test's clock, never by the wall clock.
    let clock = TestClock::new();
    app.draft = canvas_daemon::draft::Draft::with_clock(clock.clock());
    let app = Arc::new(app);
    *app.devices.write().await = devices.clone();
    let mut sinks = std::collections::BTreeMap::new();
    for device in &devices {
        let sink = Arc::new(Rec::default());
        engine.add_sink(device.id.clone(), Arc::clone(&sink) as Arc<dyn FrameSink>);
        sinks.insert(device.id.0.clone(), sink);
    }
    let a = Arc::clone(sinks.get(A).expect("the first fake device has a sink"));
    let b = Arc::clone(sinks.get(B).expect("the second fake device has a sink"));
    // The way start-up applies the saved scenes.
    canvas_daemon::scenes::restore_active(&app).await;
    Rig { router: canvas_daemon::api::router(app.clone()), app, a, b, sinks, clock, home }
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

/// Waits (real time) until `sink`'s newest frame has `LEDS` LEDs and every one of them is black.
async fn goes_dark(sink: &Rec, what: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if sink.last().is_some_and(|f| f.len() == LEDS as usize && f.iter().all(|c| *c == Rgb8::BLACK)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} never went dark; last frame starts {:?}", sink.last().map(|f| f[..3.min(f.len())].to_vec()));
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
    assert!(!pattern.needs.screen && !pattern.needs.audio);
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

// ---------------------------------------------------------------- Task 5: the routes

fn pattern_path(id: &str) -> String {
    route(routes::PATTERN, &[("id", id)])
}

fn saved(body: &serde_json::Value) -> Pattern {
    serde_json::from_value(body.clone()).expect("the body is a Pattern")
}

fn listed(body: &serde_json::Value) -> PatternsResponse {
    serde_json::from_value(body.clone()).expect("the body is a PatternsResponse")
}

fn deleted(body: &serde_json::Value) -> DeletePatternResponse {
    serde_json::from_value(body.clone()).expect("the body is a DeletePatternResponse")
}

#[tokio::test]
async fn put_creates_get_reads_and_the_list_has_it() {
    let rig = rig().await;
    let warm = pattern("Warm");
    let (status, answer) = send(
        &rig,
        "PUT",
        &pattern_path("pattern:Warm"),
        Some(serde_json::to_value(&warm).unwrap()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(saved(&answer), warm, "the saved pattern comes back");

    // By the id, and by the bare name.
    let (status, answer) = send(&rig, "GET", &pattern_path("pattern:Warm"), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(saved(&answer), warm);
    let (status, answer) = send(&rig, "GET", &pattern_path("Warm"), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(saved(&answer), warm, "the name finds it, ignoring case and the prefix");

    let (status, answer) = send(&rig, "GET", routes::PATTERNS, None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let ids: Vec<String> = listed(&answer).patterns.into_iter().map(|p| p.id).collect();
    assert_eq!(ids, ["pattern:Intake", "pattern:Warm"], "both are listed, in the file's order");

    // It is in the library too, as a pattern.
    let (_, list) = send(&rig, "GET", routes::EFFECTS, None).await;
    let list: EffectsResponse = serde_json::from_value(list).unwrap();
    let entry = list.effects.iter().find(|e| e.id == "pattern:Warm").expect("the new pattern is listed");
    assert_eq!(entry.kind, EffectKind::Pattern);
}

#[tokio::test]
async fn a_body_naming_another_id_is_a_422() {
    let rig = rig().await;
    let mut body = pattern("Warm");
    body.id = "pattern:Other".into();
    let (status, answer) = send(
        &rig,
        "PUT",
        &pattern_path("pattern:Warm"),
        Some(serde_json::to_value(&body).unwrap()),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("pattern:Other"), "the sentence names the body's id: {message}");
    assert!(message.contains("pattern:Warm"), "and the path's: {message}");
    assert!(rig.app.patterns.get("pattern:Warm").is_none(), "nothing was saved");
}

#[tokio::test]
async fn a_name_taken_ignoring_case_is_a_409() {
    let rig = rig().await;
    // The id differs from `pattern:Intake` (case matters to an id), so this is the name conflict
    // and not the path check.
    let shouty = pattern("INTAKE");
    let (status, answer) = send(
        &rig,
        "PUT",
        &pattern_path("pattern:INTAKE"),
        Some(serde_json::to_value(&shouty).unwrap()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert_eq!(answer["error"], "conflict");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("INTAKE") || message.contains("ignoring case"), "{message}");
    assert!(
        !rig.app.patterns.all().iter().any(|p| p.id == "pattern:INTAKE"),
        "nothing was saved"
    );
}

#[tokio::test]
async fn a_frame_of_the_wrong_length_is_a_422() {
    let rig = rig().await;
    let mut short = pattern("Short");
    short.frames[0].colors.pop();
    let (status, answer) = send(
        &rig,
        "PUT",
        &pattern_path("pattern:Short"),
        Some(serde_json::to_value(&short).unwrap()),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("11") && message.contains("12"), "the sentence counts them: {message}");
    assert!(rig.app.patterns.get("pattern:Short").is_none(), "nothing was saved");
}

#[tokio::test]
async fn delete_is_refused_while_any_scene_plays_it_and_names_the_scenes() {
    // `pattern:Intake` is played by a group of `Main` and by a group of `Other`, and `Other` is
    // not the active scene: a pattern in a scene that is not active is still in use.
    let rig = rig().await;
    let (status, answer) = send(&rig, "DELETE", &pattern_path("pattern:Intake"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("Main"), "the refusal names Main: {message}");
    assert!(message.contains("Other"), "and Other: {message}");
    assert!(message.contains("Delete it anyway"), "and says what a forced delete does: {message}");
    assert!(!message.contains("?force"), "in the owner's words, not the route's: {message}");
    assert!(rig.app.patterns.get("pattern:Intake").is_some(), "it is still there");
}

/// The saved pattern's id in other case: the store matches a group's id ignoring ASCII case, so
/// the daemon must too.
const LOWER_INTAKE: &str = "pattern:intake";

/// A rig with one active scene and both groups painting solid, so the only group that names the
/// pattern is the one a test gives it to.
async fn rig_one_scene() -> Rig {
    rig_with_patterns(
        vec![pattern("Intake")],
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "main".into(),
            scenes: vec![Scene {
                id: "main".into(),
                name: "Main".into(),
                rev: 1,
                groups: vec![
                    group("left", &["fake:a#z0"], "solid", color(RED)),
                    group("right", &["fake:b#z0"], "solid", color(RED)),
                ],
            }],
        },
    )
    .await
}

/// Puts `main` whole, with the right group playing `effect` as given: the case a group stores.
async fn put_main_playing(rig: &Rig, effect: &str) {
    let scene = Scene {
        id: "main".into(),
        name: "Main".into(),
        rev: 1,
        groups: vec![
            group("left", &["fake:a#z0"], "solid", color(RED)),
            group("right", &["fake:b#z0"], effect, Props::new()),
        ],
    };
    let body = serde_json::json!({
        "expected_rev": 1,
        "scene": serde_json::to_value(scene).unwrap(),
    });
    let (status, answer) =
        send(rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

#[tokio::test]
async fn an_edit_reaches_a_group_that_stores_the_pattern_id_in_other_case() {
    let rig = rig_one_scene().await;
    put_main_playing(&rig, LOWER_INTAKE).await;
    // The group plays it: the store found the pattern whatever the case, so the lights show it.
    shows_marked(&rig.b, LEDS as usize, 0, rgb(RED), "the group storing the id in other case").await;

    let blue = Rgb8::new(0, 0, 200);
    let (status, answer) = put_pattern_body(&rig, INTAKE, &pattern_with("Intake", LEDS, blue)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    shows_marked(&rig.b, LEDS as usize, 0, blue, "the group after the edit").await;
}

#[tokio::test]
async fn a_delete_is_refused_for_a_group_that_stores_the_pattern_id_in_other_case() {
    let rig = rig_one_scene().await;
    put_main_playing(&rig, LOWER_INTAKE).await;
    shows_marked(&rig.b, LEDS as usize, 0, rgb(RED), "the group storing the id in other case").await;

    let (status, answer) = send(&rig, "DELETE", &pattern_path(INTAKE), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("Main"), "the refusal names the scene: {message}");
    assert!(message.contains("Delete it anyway"), "and what a forced delete does: {message}");
    assert!(rig.app.patterns.get(INTAKE).is_some(), "it is still there");
}

#[tokio::test]
async fn a_forced_delete_removes_the_pattern_and_its_favourite() {
    let rig = rig().await;
    let body = serde_json::json!({ "favorite": true });
    let (status, answer) =
        send(&rig, "PUT", &route(routes::EFFECT_FAVORITE, &[("id", INTAKE)]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let (status, answer) =
        send(&rig, "DELETE", &format!("{}?force=true", pattern_path(INTAKE)), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let answer = deleted(&answer);
    assert!(answer.ok);
    assert_eq!(answer.id, INTAKE);
    assert!(answer.favorite_removed, "its favourite went with it");
    assert!(answer.note.is_some(), "the answer says a group went dark: {:?}", answer.note);

    let (status, _) = send(&rig, "GET", &pattern_path(INTAKE), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "it is gone");
    assert!(rig.app.patterns.get(INTAKE).is_none());
    assert!(
        !rig.app.store.read().await.effects.favorites.iter().any(|f| f == INTAKE),
        "the favourite is gone from effects.json too"
    );
    let on_disk = std::fs::read_to_string(rig.home.path.join("patterns.json")).unwrap();
    assert!(!on_disk.contains("Intake"), "{on_disk}");
    let (_, list) = send(&rig, "GET", routes::EFFECTS, None).await;
    let list: EffectsResponse = serde_json::from_value(list).unwrap();
    assert!(!list.effects.iter().any(|e| e.id == INTAKE), "and from the library");
}

#[tokio::test]
async fn a_forced_delete_of_a_played_pattern_darkens_that_group_only() {
    let rig = rig().await;
    // The left group paints solid red: it is the group the delete must leave alone.
    shows(&rig.a, rgb(RED), "the left group before").await;

    let (status, answer) =
        send(&rig, "DELETE", &format!("{}?force=true", pattern_path(INTAKE)), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    // The group that played it is dark and says why; the group next to it still paints.
    let states = group_states(&rig).await;
    let failed = states.get("right").cloned().flatten().expect("the right group is dark and says why");
    assert!(failed.contains(INTAKE), "the failure names the pattern: {failed}");
    assert_eq!(states.get("left"), Some(&None), "the other group is not failed");
    shows(&rig.a, rgb(RED), "the left group after").await;
    // Its device is handed black frames: a dark group still renders every tick.
    goes_dark(&rig.b, "the right device").await;
}

#[tokio::test]
async fn a_bundle_round_trips_patterns_and_a_0_15_bundle_imports_with_none() {
    let rig = rig().await;
    let (status, bundle) = send(&rig, "GET", routes::CONFIG_EXPORT, None).await;
    assert_eq!(status, StatusCode::OK, "{bundle}");
    assert_eq!(bundle["patterns"].as_array().map(Vec::len), Some(1), "the export carries the patterns");
    assert_eq!(bundle["patterns"][0]["id"], serde_json::json!(INTAKE));

    // A bundle from before patterns has none, and importing it replaces the saved patterns with
    // none, as it does the stacks.
    let mut old = bundle.clone();
    old.as_object_mut().unwrap().remove("patterns");
    let (status, answer) = send(&rig, "POST", routes::CONFIG_IMPORT, Some(old)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(rig.app.patterns.all().is_empty(), "no patterns after the old bundle");
    let on_disk = std::fs::read_to_string(rig.home.path.join("patterns.json")).unwrap();
    assert!(!on_disk.contains("Intake"), "{on_disk}");
    let (_, list) = send(&rig, "GET", routes::PATTERNS, None).await;
    assert!(listed(&list).patterns.is_empty());

    // A bundle with a pattern imports it, and the store reads it back.
    let mut with_one = bundle.clone();
    with_one["patterns"] = serde_json::json!([serde_json::to_value(pattern("Return")).unwrap()]);
    let (status, answer) = send(&rig, "POST", routes::CONFIG_IMPORT, Some(with_one)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        rig.app.patterns.all().iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        ["pattern:Return"]
    );
    let (status, answer) = send(&rig, "GET", &pattern_path("pattern:Return"), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(saved(&answer).name(), "Return");

    // A bundle whose pattern breaks a rule is refused before anything is written.
    let mut broken = bundle;
    let mut bad = pattern("Bad");
    bad.frames.clear();
    broken["patterns"] = serde_json::json!([
        serde_json::to_value(pattern("Fine")).unwrap(),
        serde_json::to_value(&bad).unwrap()
    ]);
    let (status, answer) = send(&rig, "POST", routes::CONFIG_IMPORT, Some(broken)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert!(rig.app.patterns.get("pattern:Fine").is_none(), "nothing was imported");
    assert!(rig.app.patterns.get("pattern:Return").is_some(), "the patterns as they were");
}

#[tokio::test]
async fn an_unknown_future_field_survives_a_put_and_a_get() {
    let rig = rig().await;
    let body = serde_json::json!({
        "id": "pattern:Spin",
        "built_on": { "led_count": 2, "arrangement": "ring" },
        "frames": [
            { "colors": ["#ff0000", "#00ff00"], "hold": 250, "label": "first" },
            { "colors": ["#0000ff", "#ffffff"], "hold": 250 }
        ],
        "motion": { "type": "rotate", "speed": 2 },
        "tags": ["warm"]
    });
    let (status, answer) = send(&rig, "PUT", &pattern_path("pattern:Spin"), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let (status, answer) = send(&rig, "GET", &pattern_path("pattern:Spin"), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let text = answer.to_string();
    for kept in ["label", "rotate", "speed", "tags"] {
        assert!(text.contains(kept), "{kept} was dropped by the read: {text}");
    }
    assert_eq!(answer["frames"].as_array().map(Vec::len), Some(2), "both frames are kept");

    let on_disk = std::fs::read_to_string(rig.home.path.join("patterns.json")).unwrap();
    for kept in ["label", "rotate", "speed", "tags"] {
        assert!(on_disk.contains(kept), "{kept} was dropped on disk: {on_disk}");
    }
}

// ---------------------------------------------------------------- Task 6: groups and flags

/// The `fits` of every group of the active scene, by group id, from `GET /api/v1/scenes`.
async fn group_fits(rig: &Rig) -> std::collections::BTreeMap<String, Vec<LightFit>> {
    let (status, body) = send(rig, "GET", routes::SCENES, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let scene: canvas_proto::api::ScenesResponse = serde_json::from_value(body).unwrap();
    scene
        .group_states
        .into_iter()
        .map(|state| (state.id, state.fits))
        .collect()
}

/// Waits (real time) until `sink`'s newest frame has `leds` LEDs with `want` at `index` and every
/// other LED black.
async fn shows_marked(sink: &Rec, leds: usize, index: usize, want: Rgb8, what: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        let right = sink.last().is_some_and(|frame| {
            frame.len() == leds
                && frame[index] == want
                && frame.iter().enumerate().all(|(i, c)| i == index || *c == Rgb8::BLACK)
        });
        if right {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what}: never showed {want:?} at LED {index}; last frame starts {:?}",
        sink.last().map(|f| f[..3.min(f.len())].to_vec()));
}

/// A pattern of `leds` LEDs with `lit` lit and every other LED off.
fn pattern_with(name: &str, leds: u32, lit: Rgb8) -> Pattern {
    let mut colors = vec![Rgb8::BLACK; leds as usize];
    colors[0] = lit;
    Pattern {
        id: format!("pattern:{name}"),
        built_on: BuiltOn { led_count: leds, arrangement: Arrangement::Ring, grid: None },
        frames: vec![Frame { colors, hold: 0, transition: None, extra: serde_json::Map::new() }],
        motion: Motion::none(),
        playback: None,
        extra: serde_json::Map::new(),
    }
}

async fn put_pattern_body(rig: &Rig, id: &str, body: &Pattern) -> (StatusCode, serde_json::Value) {
    send(rig, "PUT", &pattern_path(id), Some(serde_json::to_value(body).unwrap())).await
}

/// A pattern group's colours are on the fake sinks, LED by LED: the daemon path, not just the
/// engine's unit test (the plan's `a_pattern_group_writes_its_colours_by_led`, proved on the
/// fakes).
#[tokio::test]
async fn a_pattern_group_writes_its_colours_by_led() {
    let rig = rig().await;
    shows_marked(&rig.b, LEDS as usize, 0, rgb(RED), "the pattern group")
        .await;
}

#[tokio::test]
async fn a_pattern_edit_reaches_every_group_playing_it() {
    let rig = rig_full(
        vec![pattern("Intake")],
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "main".into(),
            scenes: vec![Scene {
                id: "main".into(),
                name: "Main".into(),
                rev: 1,
                groups: vec![
                    group("one", &["fake:a#z0"], INTAKE, Props::new()),
                    group("two", &["fake:b#z0"], INTAKE, Props::new()),
                    group("three", &["fake:c#z0"], INTAKE, Props::new()),
                    group("four", &["fake:d#z0"], "solid", color(RED)),
                ],
            }],
        },
        vec![device(A), device(B), device("fake:c"), device("fake:d")],
        vec![light(A), light(B), light("fake:c"), light("fake:d")],
    )
    .await;
    let (a, b, c, d) = (
        Arc::clone(rig.sinks.get(A).unwrap()),
        Arc::clone(rig.sinks.get(B).unwrap()),
        Arc::clone(rig.sinks.get("fake:c").unwrap()),
        Arc::clone(rig.sinks.get("fake:d").unwrap()),
    );
    shows_marked(&a, LEDS as usize, 0, rgb(RED), "group one before").await;
    shows_marked(&c, LEDS as usize, 0, rgb(RED), "group three before").await;

    let blue = Rgb8::new(0, 0, 200);
    let (status, answer) = put_pattern_body(&rig, INTAKE, &pattern_with("Intake", LEDS, blue)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    for (sink, what) in [(&a, "group one"), (&b, "group two"), (&c, "group three")] {
        shows_marked(sink, LEDS as usize, 0, blue, what).await;
    }
    // The group on another effect is untouched.
    shows(&d, rgb(RED), "group four").await;
}

#[tokio::test]
async fn a_pattern_edit_waits_while_a_takeover_holds_the_group() {
    let rig = rig().await;
    // A takeover of the left group, as a draft leaves the engine: the group holds another effect,
    // and `Held` says so.
    let green = Rgb8::new(0, 200, 0);
    let mut green_props = Props::new();
    green_props.insert("color".into(), PropValue::Color(green));
    let effect = canvas_daemon::create_effect(
        &rig.app.user_effects,
        &rig.app.stacks,
        &rig.app.patterns,
        "solid",
        Some("left"),
    )
    .expect("the solid effect builds");
    rig.app.engine.set_group_effect("left", Some(effect), &green_props);
    rig.app.scene.land_takeover(canvas_daemon::scenes::Held {
        groups: vec!["left".into(), "right".into()],
        takeover: vec!["left".into()],
    });
    shows(&rig.a, green, "the held group").await;

    let blue = Rgb8::new(0, 0, 200);
    let (status, answer) = put_pattern_body(&rig, INTAKE, &pattern_with("Intake", LEDS, blue)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    // The group the takeover holds keeps the takeover's effect; the free group takes the edit.
    shows_marked(&rig.b, LEDS as usize, 0, blue, "the free group").await;
    shows(&rig.a, green, "the held group after the edit").await;
}

#[tokio::test]
async fn a_missing_pattern_darkens_only_its_group() {
    let rig = rig().await;
    shows(&rig.a, rgb(RED), "the canvas group").await;
    shows_marked(&rig.b, LEDS as usize, 0, rgb(RED), "the pattern group").await;

    // The pattern is gone from the store without a scene write, as a hand edit of the file
    // followed by a reload would leave it: the change is reported for it.
    assert!(rig.app.patterns.delete(INTAKE).unwrap().is_some(), "the pattern was there");
    canvas_daemon::scenes::pattern_changed(&rig.app, INTAKE).await;

    let states = group_states(&rig).await;
    let failed = states.get("right").cloned().flatten().expect("the right group is dark and says why");
    assert!(failed.contains(INTAKE), "the failure names the pattern: {failed}");
    assert_eq!(states.get("left"), Some(&None), "the other group is not failed");
    shows(&rig.a, rgb(RED), "the other group runs").await;
    goes_dark(&rig.b, "the group whose pattern is gone").await;
}

#[tokio::test]
async fn group_states_flag_the_stretched_lights() {
    let rig = rig_full(
        vec![pattern("Intake")],
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "main".into(),
            scenes: vec![Scene {
                id: "main".into(),
                name: "Main".into(),
                rev: 1,
                groups: vec![group(
                    "all",
                    &["fake:a#z0", "fake:b#z0", "fake:c#z0"],
                    INTAKE,
                    Props::new(),
                )],
            }],
        },
        vec![device(A), device_of(B, 24), device_of("fake:c", 1)],
        vec![light(A), light(B), light_of("fake:c", Arrangement::Single)],
    )
    .await;
    let fits = group_fits(&rig).await;
    let all = fits.get("all").expect("the group is in the answer");
    assert_eq!(
        all,
        &vec![
            LightFit { light: "fake:a#z0".into(), fit: FitKind::Exact },
            LightFit { light: "fake:b#z0".into(), fit: FitKind::Stretched },
            LightFit { light: "fake:c#z0".into(), fit: FitKind::OneLed },
        ],
        "the flags follow the rule: exact, stretched, one LED"
    );
}

#[tokio::test]
async fn the_flags_follow_a_resize_without_a_save() {
    let rig = rig().await;
    let fits = group_fits(&rig).await;
    assert_eq!(fits["right"][0].fit, FitKind::Exact, "the group starts exact");

    // The zone grows (a chain-setup commit) and nothing is saved: the next read works the fit out
    // again from the current devices.
    *rig.app.devices.write().await = vec![device_of(A, LEDS), device_of(B, 24)];
    let fits = group_fits(&rig).await;
    assert_eq!(fits["right"][0].fit, FitKind::Stretched, "the 24-LED light is stretched now");
    assert_eq!(fits["right"][0].light, "fake:b#z0");
}

#[tokio::test]
async fn a_light_with_no_leds_has_no_fit() {
    // B's zone has no LEDs and C's device is gone (an unplugged fan lists no zone): the engine
    // plays nothing on either, so neither is exact and neither is called stretched. A keeps its
    // fit, and the group is still the lights that play.
    let rig = rig_full(
        vec![pattern("Intake")],
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "main".into(),
            scenes: vec![Scene {
                id: "main".into(),
                name: "Main".into(),
                rev: 1,
                groups: vec![group(
                    "all",
                    &["fake:a#z0", "fake:b#z0", "fake:c#z0"],
                    INTAKE,
                    Props::new(),
                )],
            }],
        },
        vec![device(A), device_of(B, 0)],
        vec![light(A), light(B), light("fake:c")],
    )
    .await;
    let fits = group_fits(&rig).await;
    assert_eq!(
        fits.get("all"),
        Some(&vec![LightFit { light: "fake:a#z0".into(), fit: FitKind::Exact }]),
        "only the light that has LEDs carries a fit"
    );
}

#[tokio::test]
async fn a_group_playing_a_canvas_effect_has_no_flags() {
    let rig = rig().await;
    let fits = group_fits(&rig).await;
    assert!(fits["left"].is_empty(), "a canvas group has no fits: {:?}", fits["left"]);
    assert!(!fits["right"].is_empty(), "the pattern group has its fits");
}

// ---------------------------------------------------------------- Task 7: the draft takeover

const EPOCH: &str = "0123456789abcdef";
const EPOCH2: &str = "fedcba9876543210";
const C: &str = "fake:c";

/// Three lights in one group painting solid red: the prior a pattern draft takes over from. Two
/// are rings of twelve (the kind a pattern is built on) and one is a strip of twelve.
fn draft_scene() -> ScenesFile {
    ScenesFile {
        schema_version: SCENES_SCHEMA,
        active: "main".into(),
        scenes: vec![Scene {
            id: "main".into(),
            name: "Main".into(),
            rev: 1,
            groups: vec![group(
                "all",
                &["fake:a#z0", "fake:b#z0", "fake:c#z0"],
                "solid",
                color(RED),
            )],
        }],
    }
}

async fn draft_rig(patterns: Vec<Pattern>) -> Rig {
    rig_full(
        patterns,
        draft_scene(),
        vec![device(A), device(B), device_of(C, LEDS)],
        vec![light(A), light(B), light_of(C, Arrangement::Strip)],
    )
    .await
}

fn sink(rig: &Rig, id: &str) -> Arc<Rec> {
    Arc::clone(rig.sinks.get(id).unwrap_or_else(|| panic!("no sink for {id}")))
}

/// The scene's frame at the draft's dim level.
fn dimmed() -> Rgb8 {
    let percent = canvas_proto::pattern::PATTERN_EDIT_DIM_PERCENT as u32;
    Rgb8::new(
        (RED.0 as u32 * percent / 100) as u8,
        (RED.1 as u32 * percent / 100) as u8,
        (RED.2 as u32 * percent / 100) as u8,
    )
}

async fn put_pattern_draft(
    rig: &Rig,
    pattern: &Pattern,
    epoch: &str,
    seq: u64,
) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({
        "pattern": serde_json::to_value(pattern).unwrap(),
        "props": {},
        "epoch": epoch,
        "seq": seq,
    });
    send(rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await
}

async fn status_of(rig: &Rig) -> canvas_proto::api::DraftStatus {
    let (status, body) = send(rig, "GET", routes::EFFECT_DRAFT, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).expect("the body is a DraftStatus")
}

#[tokio::test]
async fn a_pattern_draft_lights_the_devices_of_its_kind_and_dims_the_rest() {
    let rig = draft_rig(Vec::new()).await;
    shows(&rig.a, rgb(RED), "the scene before").await;

    let (status, answer) =
        put_pattern_draft(&rig, &pattern_with("Draft", LEDS, Rgb8::new(0, 0, 200)), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["applied"], serde_json::json!(true), "{answer}");

    // The two rings of twelve show the draft, LED by LED.
    for sink in [sink(&rig, A), sink(&rig, B)] {
        shows_marked(&sink, LEDS as usize, 0, Rgb8::new(0, 0, 200), "a ring of the draft's kind")
            .await;
    }
    // The strip is not of the pattern's kind: it keeps the scene's frame at the dim level.
    shows(&sink(&rig, C), dimmed(), "the dimmed strip").await;
    assert_ne!(dimmed(), Rgb8::BLACK, "the dimmed lights are not black");
}

#[tokio::test]
async fn a_stroke_changes_the_lit_devices_without_rebuilding_the_scene() {
    let rig = draft_rig(Vec::new()).await;
    let blue = Rgb8::new(0, 0, 200);
    let green = Rgb8::new(0, 200, 0);
    let (status, answer) = put_pattern_draft(&rig, &pattern_with("Draft", LEDS, blue), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, blue, "the first stroke").await;

    let builds = rig.app.scene.builds();
    let (status, answer) =
        put_pattern_draft(&rig, &pattern_with("Draft", LEDS, green), EPOCH, 2).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, green, "the second stroke").await;
    assert_eq!(
        rig.app.scene.builds(),
        builds,
        "a stroke swaps the pattern on its group and builds nothing"
    );
}

/// The editor's heartbeat sends the document that is already on the lights. The effect it would
/// install restarts its clock and fits every frame again (`GroupSlot::install`), so a playing
/// pattern would start over every five seconds: the same document is not installed again, and a
/// different one is (#481).
#[tokio::test]
async fn the_same_document_again_is_not_installed_over_a_running_pattern() {
    let rig = draft_rig(Vec::new()).await;
    let blue = Rgb8::new(0, 0, 200);
    let green = Rgb8::new(0, 200, 0);
    let draft = pattern_with("Draft", LEDS, blue);
    let (status, answer) = put_pattern_draft(&rig, &draft, EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, blue, "the first document").await;
    let landed = rig.app.scene.takeover_generation();

    let (status, answer) = put_pattern_draft(&rig, &draft, EPOCH, 2).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["applied"], serde_json::json!(true), "the beat still keeps the lease: {answer}");
    assert_eq!(
        rig.app.scene.takeover_generation(),
        landed,
        "the same document is not installed again, so its clock keeps running"
    );
    shows_marked(&rig.a, LEDS as usize, 0, blue, "the same document still plays").await;

    let (status, answer) = put_pattern_draft(&rig, &pattern_with("Draft", LEDS, green), EPOCH, 3).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(
        rig.app.scene.takeover_generation() > landed,
        "a different document is installed, and its clock starts over"
    );
    shows_marked(&rig.a, LEDS as usize, 0, green, "the changed document").await;
}

#[tokio::test]
async fn an_older_stroke_never_replaces_a_newer_one() {
    let rig = draft_rig(Vec::new()).await;
    let blue = Rgb8::new(0, 0, 200);
    let green = Rgb8::new(0, 200, 0);
    let (status, answer) = put_pattern_draft(&rig, &pattern_with("Draft", LEDS, blue), EPOCH, 5).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, blue, "stroke 5").await;

    let (status, answer) =
        put_pattern_draft(&rig, &pattern_with("Draft", LEDS, green), EPOCH, 3).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["applied"], serde_json::json!(false), "the older stroke is superseded");
    // The newer stroke is still what the lights show.
    shows_marked(&rig.a, LEDS as usize, 0, blue, "stroke 5 after stroke 3").await;
}

#[tokio::test]
async fn a_new_epoch_supersedes_the_old_one() {
    let rig = draft_rig(Vec::new()).await;
    let blue = Rgb8::new(0, 0, 200);
    let green = Rgb8::new(0, 200, 0);
    let red = Rgb8::new(200, 0, 0);
    let (status, answer) = put_pattern_draft(&rig, &pattern_with("Draft", LEDS, blue), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, blue, "the first page").await;

    // A page the daemon has not seen is the newest page and takes the draft.
    let (status, answer) =
        put_pattern_draft(&rig, &pattern_with("Draft", LEDS, green), EPOCH2, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["applied"], serde_json::json!(true), "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, green, "the new page").await;

    // The page that lost keeps losing, whatever its sequence.
    let (status, answer) = put_pattern_draft(&rig, &pattern_with("Draft", LEDS, red), EPOCH, 2).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["applied"], serde_json::json!(false), "the older page is superseded");
    shows_marked(&rig.a, LEDS as usize, 0, green, "the new page after the old one").await;
}

#[tokio::test]
async fn the_lease_ending_puts_the_scene_back() {
    let rig = draft_rig(Vec::new()).await;
    let (status, answer) =
        put_pattern_draft(&rig, &pattern_with("Draft", LEDS, Rgb8::new(0, 0, 200)), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, Rgb8::new(0, 0, 200), "the draft").await;

    rig.clock.advance(canvas_proto::consts::DRAFT_LEASE + Duration::from_secs(1));
    let status = status_of(&rig).await;
    assert!(!status.active, "the lease ran out: {status:?}");
    shows(&rig.a, rgb(RED), "the scene after the lease").await;
    shows(&sink(&rig, C), rgb(RED), "the strip after the lease").await;
}

#[tokio::test]
async fn delete_stops_the_draft_and_restores_the_scene() {
    let rig = draft_rig(Vec::new()).await;
    let (status, answer) =
        put_pattern_draft(&rig, &pattern_with("Draft", LEDS, Rgb8::new(0, 0, 200)), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, Rgb8::new(0, 0, 200), "the draft").await;

    let (status, _) = send(&rig, "DELETE", routes::EFFECT_DRAFT, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!status_of(&rig).await.active, "the draft is over");
    shows(&rig.a, rgb(RED), "the scene after the delete").await;
}

#[tokio::test]
async fn a_draft_with_no_device_of_its_kind_starts_and_says_so() {
    let rig = draft_rig(Vec::new()).await;
    // A ring of 24, a kind no fake here is.
    let wide = pattern_with("Wide", 24, Rgb8::new(0, 0, 200));
    let (status, answer) = put_pattern_draft(&rig, &wide, EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["active"], serde_json::json!(true), "the draft still starts: {answer}");
    assert_eq!(answer["id"], serde_json::json!("draft:pattern:Wide"));
    assert_eq!(
        answer["note"],
        serde_json::json!("no device of this kind is connected"),
        "{answer}"
    );
    // Nothing is of that kind, so every light is at the scene's dim level.
    shows(&rig.a, dimmed(), "the dimmed ring").await;
}

#[tokio::test]
async fn an_invalid_pattern_draft_is_a_422_and_a_running_draft_stays() {
    let rig = draft_rig(Vec::new()).await;
    let blue = Rgb8::new(0, 0, 200);
    let (status, answer) = put_pattern_draft(&rig, &pattern_with("Draft", LEDS, blue), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, blue, "the good draft").await;

    let mut broken = pattern_with("Draft", LEDS, Rgb8::new(0, 200, 0));
    broken.frames[0].colors.pop();
    let (status, answer) = put_pattern_draft(&rig, &broken, EPOCH, 2).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("11") && message.contains("12"), "the sentence counts them: {message}");

    // The draft that was running is left alone, showing its last good stroke.
    let status = status_of(&rig).await;
    assert!(status.active, "the running draft stayed");
    shows_marked(&rig.a, LEDS as usize, 0, blue, "the running draft after the refusal").await;
}

#[tokio::test]
async fn deleting_the_pattern_being_drafted_ends_the_draft_first() {
    let rig = draft_rig(vec![pattern("Intake")]).await;
    let blue = Rgb8::new(0, 0, 200);
    let (status, answer) =
        put_pattern_draft(&rig, &pattern_with("Intake", LEDS, blue), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, blue, "the pattern being drafted").await;

    // No group plays it, so no force is needed: the draft ends before the pattern goes.
    let (status, answer) = send(&rig, "DELETE", &pattern_path(INTAKE), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(rig.app.patterns.get(INTAKE).is_none(), "the pattern is gone");
    assert!(!status_of(&rig).await.active, "the draft ended first");
    shows(&rig.a, rgb(RED), "the scene is back").await;
}

#[tokio::test]
async fn a_draft_with_html_and_a_pattern_is_refused() {
    let rig = draft_rig(Vec::new()).await;
    let body = serde_json::json!({
        "html": "<html><body>hi</body></html>",
        "pattern": serde_json::to_value(pattern_with("Draft", LEDS, Rgb8::new(0, 0, 200))).unwrap(),
        "props": {},
        "epoch": EPOCH,
        "seq": 1,
    });
    let (status, answer) = send(&rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("html") && message.contains("pattern"), "{message}");
    assert!(!status_of(&rig).await.active, "nothing started");
}

#[tokio::test]
async fn a_draft_with_no_field_says_what_it_needs() {
    let rig = draft_rig(Vec::new()).await;
    let body = serde_json::json!({ "props": {} });
    let (status, answer) = send(&rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert_eq!(message, "a draft needs html, stack or pattern", "{answer}");
}




// ---------------------------------------------------------------- Spin (#468)

/// The LED with the most red in a frame, and the frame's lit LEDs.
fn peak(frame: &[Rgb8]) -> (usize, usize) {
    let at = frame.iter().enumerate().max_by_key(|(_, c)| c.r).map(|(i, _)| i).unwrap_or(0);
    (at, frame.iter().filter(|c| **c != Rgb8::BLACK).count())
}

/// A ring of twelve with one red LED, spinning at `turns` laps a second.
fn spinning_pattern(turns: f32) -> Pattern {
    let mut pattern = pattern("Intake");
    pattern.motion = Motion::spin(turns);
    pattern
}

/// The daemon path, on the fakes: a pattern with Spin is saved, a scene plays it, and the fake
/// device receives frames whose lit LED travels a whole lap round the ring (about four seconds at
/// 0.25 laps a second), one LED at a time, with the parts between LEDs blended.
///
/// Run with `--nocapture` to read the recorded run (every 15th frame).
#[tokio::test]
async fn a_spinning_pattern_sends_the_fake_device_rotating_frames_for_a_lap() {
    let rig = rig_with_patterns(vec![spinning_pattern(0.25)], scenes_file()).await;
    shows_something(&rig.b).await;
    let from = rig.b.seen();
    // Wait for the condition, not a clock: until the lit LED has gone a whole lap (12 steps).
    wait_until(|| lap_steps(&rig.b.since(from)) >= LEDS as usize).await;
    let frames = rig.b.since(from);
    println!("recorded run: {} frames for one lap, fake:b (a ring of {LEDS}), spin 0.25 laps/s", frames.len());
    let mut last: Option<usize> = None;
    let mut seen = std::collections::BTreeSet::new();
    let mut blended = 0;
    let mut steps = 0;
    for (n, frame) in frames.iter().enumerate() {
        assert_eq!(frame.len(), LEDS as usize);
        if n % 15 == 0 {
            let lit: Vec<String> =
                frame.iter().enumerate().filter(|(_, c)| **c != Rgb8::BLACK).map(|(i, c)| format!("{i}:{c}")).collect();
            println!("frame {n:3}  peak LED {:2}  lit {}", peak(frame).0, lit.join(" "));
        }
        let (at, lit) = peak(frame);
        if lit > 1 {
            blended += 1;
        }
        // The peak only ever moves forward round the ring, by at most one LED between frames.
        if let Some(was) = last {
            let step = (at + LEDS as usize - was) % LEDS as usize;
            assert!(step <= 1, "the lit LED jumped from {was} to {at}");
            steps += step;
        }
        seen.insert(at);
        last = Some(at);
    }
    assert_eq!(seen.len(), LEDS as usize, "a lap visits every LED: saw {seen:?}");
    assert!(blended > 10, "between two LEDs the light is shared: {blended} frames");
    assert!(steps >= LEDS as usize, "{steps} LED steps for one lap");
}

/// How many LED steps the peak has moved forward across `frames` (0 while it holds still).
fn lap_steps(frames: &[Vec<Rgb8>]) -> usize {
    frames
        .windows(2)
        .map(|w| (peak(&w[1]).0 + LEDS as usize - peak(&w[0]).0) % LEDS as usize)
        .filter(|step| *step <= 1)
        .sum()
}

/// The first frame with something lit on `sink`: the scene has started and faded in.
async fn shows_something(sink: &Rec) {
    wait_until(|| sink.last().is_some_and(|f| f.len() == LEDS as usize && f.iter().any(|c| c.r > 100))).await;
}

/// A 0.16.0 pattern file (two frames, which 0.16.0 wrote with a hold of 0, and a motion this version
/// does not know) loads and plays its first frame, still, as it did before Spin and frames existed.
#[tokio::test]
async fn a_0_16_0_pattern_file_plays_as_it_did() {
    let colours = |lit: &str| -> Vec<String> {
        (0..LEDS).map(|i| if i == 0 { lit.to_string() } else { "#000000".to_string() }).collect()
    };
    let text = serde_json::json!({
        "schema_version": 1,
        "patterns": [{
            "id": INTAKE,
            "built_on": {"led_count": LEDS, "arrangement": "ring"},
            "frames": [
                {"colors": colours("#c80000"), "hold": 0},
                {"colors": colours("#0000c8"), "hold": 0}
            ],
            "motion": {"type": "rotate", "speed": 2}
        }]
    })
    .to_string();
    let file: canvas_proto::pattern::PatternsFile = serde_json::from_str(&text).expect("a 0.16.0 file loads");
    let pattern = file.patterns[0].clone();
    assert_eq!(pattern.motion.as_spin(), None);
    let rig = rig_with_patterns(vec![pattern], scenes_file()).await;
    shows_marked(&rig.b, LEDS as usize, 0, rgb(RED), "the 0.16.0 pattern").await;
    let from = rig.b.seen();
    // Wait for the condition, not a clock: enough frames to see any movement (a 30 fps engine).
    wait_until(|| rig.b.since(from).len() > 45).await;
    let frames = rig.b.since(from);
    let want = rig.b.last().expect("a frame");
    for frame in &frames {
        assert_eq!(frame, &want, "a pattern with an unknown motion stays as its first frame");
    }
}

/// A spinning pattern saves and reads back with its motion intact: the route keeps `Motion::spin`.
#[tokio::test]
async fn a_spin_pattern_saves_and_reads_back() {
    let rig = rig().await;
    let mut spinning = pattern("Fan");
    spinning.motion = Motion::spin(-0.5);
    let (status, answer) =
        send(&rig, "PUT", &pattern_path("pattern:Fan"), Some(serde_json::to_value(&spinning).unwrap())).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let (status, answer) = send(&rig, "GET", &pattern_path("pattern:Fan"), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(saved(&answer).motion.as_spin(), Some(-0.5), "{answer}");
    assert_eq!(answer["motion"]["type"], "spin", "{answer}");
}

/// The editor's own path: a draft with a spinning pattern turns on the fake lights it plays on.
#[tokio::test]
async fn a_spinning_draft_turns_on_the_lights() {
    let rig = draft_rig(Vec::new()).await;
    let spinning = spinning_pattern(0.5);
    let (status, answer) = put_pattern_draft(&rig, &spinning, EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let a = sink(&rig, A);
    wait_until(|| a.last().is_some_and(|f| f.len() == LEDS as usize && f.iter().any(|c| c.r > 100))).await;
    let from = a.seen();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let frames = a.since(from);
    let peaks: std::collections::BTreeSet<usize> =
        frames.iter().filter(|f| f.iter().any(|c| c.r > 60)).map(|f| peak(f).0).collect();
    // 0.5 laps a second on 12 LEDs is 6 LEDs a second: well over four positions in 1.5 s.
    assert!(peaks.len() >= 5, "the lit LED stood at {peaks:?}");
}


// ---------------------------------------------------------------- Frames (#468)

fn solid_frame(colour: Rgb8, hold: u32, transition: Option<canvas_proto::pattern::Transition>) -> Frame {
    Frame { colors: vec![colour; LEDS as usize], hold, transition, extra: serde_json::Map::new() }
}

fn fade(ms: u32) -> Option<canvas_proto::pattern::Transition> {
    Some(canvas_proto::pattern::Transition {
        kind: canvas_proto::pattern::TransitionKind::Fade,
        ms,
        easing: canvas_proto::pattern::Easing::Linear,
    })
}

/// Red holds 250 ms, fades into blue over 250 ms, blue holds 250 ms and cuts back to red.
fn red_fade_blue() -> Pattern {
    let mut p = pattern("Intake");
    p.frames = vec![solid_frame(rgb(RED), 250, fade(250)), solid_frame(rgb(BLUE), 250, None)];
    p
}

/// What a frame of the fake device shows, in one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Red,
    Blue,
    Between,
    Other,
}

fn phase_of(frame: &[Rgb8]) -> Phase {
    let Some(first) = frame.first() else { return Phase::Other };
    if frame.iter().any(|c| c != first) {
        return Phase::Other;
    }
    if *first == rgb(RED) {
        Phase::Red
    } else if *first == rgb(BLUE) {
        Phase::Blue
    } else if *first != Rgb8::BLACK {
        // Any one colour on every LED that is neither end is a step of the fade. (Near blue the
        // OKLab blend rounds red to 0 before blue reaches 255, so `r > 0 && b > 0` misread the last
        // fade steps as something else: CI saw [Red, Between, Other, Blue].)
        Phase::Between
    } else {
        Phase::Other
    }
}

/// The phases of `frames`, each run once: 11 red frames, 7 between, 9 blue come out `[Red, Between, Blue]`.
fn runs(frames: &[Vec<Rgb8>]) -> Vec<Phase> {
    let mut out: Vec<Phase> = Vec::new();
    for f in frames {
        let p = phase_of(f);
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

const BLUE: (u8, u8, u8) = (0, 0, 255);

/// The daemon path, on the fakes: a scene plays a pattern of two frames and the fake device receives
/// red, then every step of the fade (the OKLab blend, red channel falling and blue rising), then
/// blue, then red again as the loop comes round: the frames in order, with the fade between them.
///
/// Run with `--nocapture` to read the recorded run.
#[tokio::test]
async fn a_frames_pattern_sends_the_fake_device_its_frames_in_order_with_the_fade_between() {
    let rig = rig_with_patterns(vec![red_fade_blue()], scenes_file()).await;
    shows(&rig.b, rgb(RED), "the first frame").await;
    let from = rig.b.seen();
    // Wait for the condition, not a clock: until a whole cycle has come round (red, the fade, blue, red).
    wait_until(|| runs(&rig.b.since(from)).len() >= 4).await;
    let frames = rig.b.since(from);
    let sequence = runs(&frames);
    println!("recorded run: {} frames, phases {sequence:?}", frames.len());
    let mut fading: Vec<&Vec<Rgb8>> = frames.iter().filter(|f| phase_of(f) == Phase::Between).collect();
    for f in &fading {
        println!("  between: {} (LED 0)", f[0]);
    }
    assert_eq!(
        &sequence[..4],
        &[Phase::Red, Phase::Between, Phase::Blue, Phase::Red],
        "red, the fade, blue, then the cut back to red: {sequence:?}"
    );
    assert!(fading.len() >= 3, "the fade is more than one step: {} frames", fading.len());
    // The fade runs one way: red falls and blue rises.
    let first = fading.remove(0)[0];
    let last = fading.pop().map(|f| f[0]).unwrap_or(first);
    assert!(first.r > last.r && first.b < last.b, "the fade went from {first} to {last}");
}

/// A pattern file with two frames that hold and no transition (what 0.16.0 could write, and a
/// phase 1 file) cuts: the device alternates the two colours with nothing between them.
#[tokio::test]
async fn frames_with_no_transition_cut() {
    let mut p = pattern("Intake");
    p.frames = vec![solid_frame(rgb(RED), 150, None), solid_frame(rgb(BLUE), 150, None)];
    let rig = rig_with_patterns(vec![p], scenes_file()).await;
    shows(&rig.b, rgb(RED), "the first frame").await;
    let from = rig.b.seen();
    wait_until(|| runs(&rig.b.since(from)).len() >= 4).await;
    let sequence = runs(&rig.b.since(from));
    assert_eq!(&sequence[..4], &[Phase::Red, Phase::Blue, Phase::Red, Phase::Blue], "{sequence:?}");
}

/// Without Loop the pattern stops on its last frame and stays there.
#[tokio::test]
async fn a_pattern_that_does_not_loop_stays_on_its_last_frame() {
    let mut p = red_fade_blue();
    p.playback = Some(canvas_proto::pattern::Playback { looping: false });
    let rig = rig_with_patterns(vec![p], scenes_file()).await;
    shows(&rig.b, rgb(BLUE), "the last frame").await;
    let from = rig.b.seen();
    // Enough frames after it to have seen a loop come round, if there were one (a hold is 250 ms).
    wait_until(|| rig.b.since(from).len() > 60).await;
    for frame in rig.b.since(from) {
        assert_eq!(phase_of(&frame), Phase::Blue, "stays on blue: {frame:?}");
    }
}

/// The editor's own path: a draft carries the whole document, frames and transitions, and the fake
/// lights it plays on go through them.
#[tokio::test]
async fn a_frames_draft_plays_the_whole_document_on_the_lights() {
    let rig = draft_rig(Vec::new()).await;
    let (status, answer) = put_pattern_draft(&rig, &red_fade_blue(), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let a = sink(&rig, A);
    wait_until(|| a.last().is_some_and(|f| phase_of(&f) == Phase::Red)).await;
    let from = a.seen();
    wait_until(|| runs(&a.since(from)).len() >= 4).await;
    let sequence = runs(&a.since(from));
    assert_eq!(&sequence[..4], &[Phase::Red, Phase::Between, Phase::Blue, Phase::Red], "{sequence:?}");
}

/// Save keeps the frames, their transitions and Loop, and reads them back exactly.
#[tokio::test]
async fn a_frames_pattern_saves_and_reads_back_whole() {
    let rig = rig().await;
    let mut p = red_fade_blue();
    p.frames[1].transition = Some(canvas_proto::pattern::Transition {
        kind: canvas_proto::pattern::TransitionKind::Fade,
        ms: 900,
        easing: canvas_proto::pattern::Easing::EaseInOut,
    });
    p.playback = Some(canvas_proto::pattern::Playback { looping: false });
    p.id = "pattern:Fade".into();
    let (status, answer) = send(&rig, "PUT", &pattern_path("pattern:Fade"), Some(serde_json::to_value(&p).unwrap())).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let (status, answer) = send(&rig, "GET", &pattern_path("pattern:Fade"), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(saved(&answer), p, "{answer}");
    assert_eq!(answer["frames"][1]["transition"]["easing"], "ease_in_out", "{answer}");
    assert_eq!(answer["playback"]["loop"], false, "{answer}");
}

/// Save refuses a hold or a transition past the spine's caps, naming the frame.
#[tokio::test]
async fn a_hold_or_a_transition_past_the_caps_is_a_422() {
    let rig = rig().await;
    let mut p = red_fade_blue();
    p.id = "pattern:Long".into();
    p.frames[0].hold = canvas_proto::pattern::PATTERN_HOLD_MS_MAX + 1;
    let (status, answer) = send(&rig, "PUT", &pattern_path("pattern:Long"), Some(serde_json::to_value(&p).unwrap())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert!(answer.to_string().contains("frame 1 holds"), "{answer}");
    p.frames[0].hold = 100;
    p.frames[1].transition = fade(canvas_proto::pattern::TRANSITION_MS_MAX + 1);
    let (status, answer) = send(&rig, "PUT", &pattern_path("pattern:Long"), Some(serde_json::to_value(&p).unwrap())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert!(answer.to_string().contains("frame 2's transition"), "{answer}");
}

// ---------------------------------------------------------------- #468 phase 4: sequences

/// Two fans of 8 (rings) and a strip of 10, as the three fake devices of a sequence.
const FAN1: &str = "fake:a";
const FAN2: &str = "fake:b";
const STRIP: &str = "fake:c";
const RUN: &str = "pattern:Run";

/// A pattern built on a ring of 26 (8 + 8 + 10), one distinct colour per LED, none off.
fn run_pattern(motion: Motion) -> Pattern {
    let colors: Vec<Rgb8> = (0..26).map(|i| Rgb8::new(10 * i as u8 + 5, 0, 7)).collect();
    Pattern {
        id: RUN.into(),
        built_on: BuiltOn { led_count: 26, arrangement: Arrangement::Ring, grid: None },
        frames: vec![Frame { colors, hold: 0, transition: None, extra: serde_json::Map::new() }],
        motion,
        playback: None,
        extra: serde_json::Map::new(),
    }
}

/// The fake rig of the sequence: both fans, the strip, and one group `run` playing [`RUN`] on
/// them in `members` order with `sequence`, saved in scene `main`.
async fn sequence_rig(
    motion: Motion,
    sequence: Option<canvas_proto::scene::SequenceShape>,
    members: [&str; 3],
) -> Rig {
    let mut run = group("run", &[], RUN, Props::new());
    run.members = members.iter().map(|id| format!("{id}#z0")).collect();
    run.sequence = sequence;
    rig_full(
        vec![run_pattern(motion)],
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "main".into(),
            scenes: vec![Scene { id: "main".into(), name: "Main".into(), rev: 1, groups: vec![run] }],
        },
        vec![device_of(FAN1, 8), device_of(FAN2, 8), device_of(STRIP, 10)],
        vec![
            light_of(FAN1, Arrangement::Ring),
            light_of(FAN2, Arrangement::Ring),
            light_of(STRIP, Arrangement::Strip),
        ],
    )
    .await
}

/// Saves scene `main` again after `change`, through the scene API, at the revision it holds.
async fn put_main(rig: &Rig, change: impl FnOnce(&mut Scene)) {
    let mut scene = rig.app.store.read().await.scenes.scenes[0].clone();
    let rev = scene.rev;
    change(&mut scene);
    let body = serde_json::to_value(canvas_proto::api::ScenePut { scene, expected_rev: rev, dry_run: false }).unwrap();
    let (status, answer) = send(rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

/// Waits (real time) until the three fake devices' newest frames are exactly `fan1`, `fan2` and
/// `strip`.
async fn shows_three(rig: &Rig, fan1: &[Rgb8], fan2: &[Rgb8], strip: &[Rgb8], what: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        let same = |id: &str, want: &[Rgb8]| rig.sinks[id].last().is_some_and(|f| f == want);
        if same(FAN1, fan1) && same(FAN2, fan2) && same(STRIP, strip) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let last = |id: &str| rig.sinks[id].last().map(|f| f[..3.min(f.len())].to_vec());
    panic!("{what}: never showed it; last frames start {:?} / {:?} / {:?}", last(FAN1), last(FAN2), last(STRIP));
}

/// The sequence through the scene API, on the fakes: the run follows the members' order, a reorder
/// through the API reorders the run, and Each light (no sequence) plays the whole pattern on each.
#[tokio::test]
async fn a_sequence_group_runs_through_its_lights_and_follows_the_scene_api() {
    use canvas_proto::scene::SequenceShape;
    let rig = sequence_rig(Motion::none(), Some(SequenceShape::Ring), [FAN1, FAN2, STRIP]).await;
    let run = run_pattern(Motion::none()).frames[0].colors.clone();
    shows_three(&rig, &run[0..8], &run[8..16], &run[16..26], "fan 1, fan 2, strip").await;

    // Reorder the members through the API: the strip first, then fan 2, then fan 1.
    put_main(&rig, |scene| {
        scene.groups[0].members = vec![format!("{STRIP}#z0"), format!("{FAN2}#z0"), format!("{FAN1}#z0")];
    })
    .await;
    shows_three(&rig, &run[18..26], &run[10..18], &run[0..10], "strip, fan 2, fan 1").await;

    // Each light again: every light fits the whole ring of 26 to itself.
    put_main(&rig, |scene| scene.groups[0].sequence = None).await;
    let built = run_pattern(Motion::none()).built_on;
    let own = |leds: u32, arrangement| canvas_core::pattern_fit::fit_colors(&run, &built, leds, arrangement);
    shows_three(
        &rig,
        &own(8, Arrangement::Ring),
        &own(8, Arrangement::Ring),
        &own(10, Arrangement::Strip),
        "each light its own",
    )
    .await;
}

/// A scene is made with a ring group in the one create request (#504 item 3), it reads back with
/// the shape, and the engine runs it as one run through the members.
#[tokio::test]
async fn a_scene_made_with_a_ring_group_runs_it_as_a_sequence() {
    use canvas_proto::api::{NewGroup, SceneCreate};
    use canvas_proto::scene::SequenceShape;
    let rig = sequence_rig(Motion::none(), None, [FAN1, FAN2, STRIP]).await;
    let asked = NewGroup {
        name: "Made".into(),
        members: [FAN1, FAN2, STRIP].iter().map(|id| format!("{id}#z0")).collect(),
        plays: Plays::Effect { effect: RUN.into(), props: Props::new() },
        mode: GroupMode::FlowAcross,
        brightness: 100,
        sequence: Some(SequenceShape::Ring),
    };
    let body = serde_json::to_value(SceneCreate {
        name: "Made".into(),
        copy_of: None,
        groups: Some(vec![asked]),
        dry_run: false,
    })
    .unwrap();
    let (status, made) = send(&rig, "POST", routes::SCENES, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{made}");
    assert_eq!(made["groups"][0]["sequence"], "ring", "the create answered the shape: {made}");

    // Read it back: the shape is saved, not only answered.
    let (status, list) = send(&rig, "GET", routes::SCENES, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let saved = list["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Made")
        .expect("the made scene is in the list");
    assert_eq!(saved["groups"][0]["sequence"], "ring", "read back: {list}");

    // Make it the active scene: the engine plays it as one run through the three devices.
    let id = made["id"].as_str().expect("the scene's id").to_string();
    let (status, answer) = send(&rig, "PUT", routes::SCENE_ACTIVE, Some(serde_json::json!({ "id": id }))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let run = run_pattern(Motion::none()).frames[0].colors.clone();
    shows_three(&rig, &run[0..8], &run[8..16], &run[16..26], "the made scene's run").await;
}

/// One shared log of every frame the three devices are handed, in the order they arrive.
#[derive(Default)]
struct Tape {
    frames: Mutex<Vec<(usize, Vec<Rgb8>)>>,
}

struct TapeSink {
    device: usize,
    tape: Arc<Tape>,
}

impl FrameSink for TapeSink {
    fn submit(&self, colors: Vec<Rgb8>) {
        self.tape.frames.lock().unwrap_or_else(|e| e.into_inner()).push((self.device, colors));
    }
}

/// The recorded run: a lit LED at the last LED of fan 1 spins into fan 2's first LED. The fake
/// devices' frames are logged together, grouped into ticks (one frame of each device), and the
/// brightest LED of the three joined in `members` order is followed for a whole lap.
///
/// Run with `--nocapture` to read the recorded run (every 12th tick).
#[tokio::test]
async fn a_spinning_sequence_carries_the_colour_from_fan_one_to_fan_two() {
    use canvas_proto::scene::SequenceShape;
    // One lit LED at run position 7 (the last LED of fan 1); 0.25 laps a second over 26 LEDs.
    let mut lit = run_pattern(Motion::spin(0.25));
    lit.frames[0].colors = (0..26).map(|i| if i == 7 { rgb(RED) } else { Rgb8::BLACK }).collect();
    let rig = {
        let mut run = group("run", &[], RUN, Props::new());
        run.members = [FAN1, FAN2, STRIP].iter().map(|id| format!("{id}#z0")).collect();
        run.sequence = Some(SequenceShape::Ring);
        rig_full(
            vec![lit],
            ScenesFile {
                schema_version: SCENES_SCHEMA,
                active: "main".into(),
                scenes: vec![Scene { id: "main".into(), name: "Main".into(), rev: 1, groups: vec![run] }],
            },
            vec![device_of(FAN1, 8), device_of(FAN2, 8), device_of(STRIP, 10)],
            vec![
                light_of(FAN1, Arrangement::Ring),
                light_of(FAN2, Arrangement::Ring),
                light_of(STRIP, Arrangement::Strip),
            ],
        )
        .await
    };
    // Log the frames from here on, one tape for the three devices.
    let tape = Arc::new(Tape::default());
    for (device, id) in [FAN1, FAN2, STRIP].iter().enumerate() {
        rig.app.engine.add_sink(DeviceId((*id).into()), Arc::new(TapeSink { device, tape: Arc::clone(&tape) }));
    }
    let ticks = || -> Vec<Vec<Rgb8>> {
        let log = tape.frames.lock().unwrap_or_else(|e| e.into_inner());
        let mut slots: [Option<Vec<Rgb8>>; 3] = [None, None, None];
        let mut out = Vec::new();
        for (device, frame) in log.iter() {
            slots[*device] = Some(frame.clone());
            if slots.iter().all(Option::is_some) {
                out.push(slots.iter_mut().flat_map(|s| s.take().unwrap()).collect());
            }
        }
        out
    };
    // The peak of the joined run: the LED with the most red.
    let at = |tick: &Vec<Rgb8>| tick.iter().enumerate().max_by_key(|(_, c)| c.r).map(|(i, _)| i).unwrap_or(0);
    // A lap is 26 steps; wait for the condition (the peak back on LED 7 after going round).
    wait_until(|| {
        let t = ticks();
        t.iter().any(|x| at(x) == 20) && t.len() > 40 && t.iter().skip(40).any(|x| at(x) == 7)
    })
    .await;
    let ticks = ticks();
    assert!(ticks.iter().all(|t| t.len() == 26), "a tick is 8 + 8 + 10 LEDs");
    println!("recorded run: {} ticks, fan1+fan2+strip joined (26 LEDs), spin 0.25 laps/s, red LED starts at 7", ticks.len());
    // Follow the peak from the first time it is on LED 7: it only moves forward, by at most one LED a tick.
    let start = ticks.iter().position(|t| at(t) == 7).expect("the lit LED starts at the end of fan 1");
    let mut crossed_at = None;
    let mut visited = std::collections::BTreeSet::new();
    for (n, pair) in ticks[start..].windows(2).enumerate() {
        let (was, now) = (at(&pair[0]), at(&pair[1]));
        let step = (now + 26 - was) % 26;
        assert!(step <= 1, "the lit LED jumped from run position {was} to {now}");
        if was == 7 && now == 8 {
            crossed_at = Some(n);
        }
        visited.insert(now);
    }
    for (n, tick) in ticks[start..].iter().enumerate().filter(|(n, _)| n % 12 == 0) {
        let lit = |r: std::ops::Range<usize>| tick[r].iter().map(|c| if c.r > 150 { '#' } else if c.r > 0 { '+' } else { '.' }).collect::<String>();
        println!("tick {n:3}  fan1 {}  fan2 {}  strip {}", lit(0..8), lit(8..16), lit(16..26));
    }
    let crossed = crossed_at.expect("the colour crossed from fan 1's last LED to fan 2's first");
    println!("crossing: tick {crossed}, run position 7 (fan 1 LED 7) -> 8 (fan 2 LED 0)");
    assert!(visited.contains(&8) && visited.contains(&16), "it goes on into fan 2 and the strip: {visited:?}");
}

/// The sequence rig of [`a_spinning_sequence_carries_the_colour_from_fan_one_to_fan_two`], with the
/// pattern given, so a test can swap its colours without changing its timing (#518).
async fn lit_sequence_rig(pattern: Pattern) -> Rig {
    use canvas_proto::scene::SequenceShape;
    let mut run = group("run", &[], RUN, Props::new());
    run.members = [FAN1, FAN2, STRIP].iter().map(|id| format!("{id}#z0")).collect();
    run.sequence = Some(SequenceShape::Ring);
    rig_full(
        vec![pattern],
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "main".into(),
            scenes: vec![Scene { id: "main".into(), name: "Main".into(), rev: 1, groups: vec![run] }],
        },
        vec![device_of(FAN1, 8), device_of(FAN2, 8), device_of(STRIP, 10)],
        vec![
            light_of(FAN1, Arrangement::Ring),
            light_of(FAN2, Arrangement::Ring),
            light_of(STRIP, Arrangement::Strip),
        ],
    )
    .await
}

/// A colour-only save of a spinning pattern keeps a sequence group's clock: the marker carries on
/// from where it is instead of starting again at its first LED (#518). The ring's clock is one per
/// group slot, shared by the whole run, so a save must not restart it.
#[tokio::test]
async fn a_spinning_sequence_group_keeps_its_clock_across_a_colour_only_swap() {
    let mut lit = run_pattern(Motion::spin(0.25));
    lit.frames[0].colors = (0..26).map(|i| if i == 7 { rgb(RED) } else { Rgb8::BLACK }).collect();
    let rig = lit_sequence_rig(lit.clone()).await;
    let tape = Arc::new(Tape::default());
    for (device, id) in [FAN1, FAN2, STRIP].iter().enumerate() {
        rig.app.engine.add_sink(DeviceId((*id).into()), Arc::new(TapeSink { device, tape: Arc::clone(&tape) }));
    }
    let ticks = || -> Vec<Vec<Rgb8>> {
        let log = tape.frames.lock().unwrap_or_else(|e| e.into_inner());
        let mut slots: [Option<Vec<Rgb8>>; 3] = [None, None, None];
        let mut out = Vec::new();
        for (device, frame) in log.iter() {
            slots[*device] = Some(frame.clone());
            if slots.iter().all(Option::is_some) {
                out.push(slots.iter_mut().flat_map(|s| s.take().unwrap()).collect());
            }
        }
        out
    };
    let at = |tick: &Vec<Rgb8>, channel: usize| {
        tick.iter().enumerate().max_by_key(|(_, c)| channel_of(c, channel)).map(|(i, _)| i).unwrap_or(0)
    };
    // 0.25 laps a second over 26 LEDs: one LED every 0.5 s. Let the red marker reach the strip.
    wait_until(|| ticks().iter().any(|t| at(t, 0) >= 15)).await;

    // Same id, motion and frames: only the marker's colour changes.
    let mut swapped = lit.clone();
    swapped.frames[0].colors[7] = Rgb8::new(0, 200, 0);
    let (status, answer) = put_pattern_body(&rig, RUN, &swapped).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    wait_until(|| ticks().iter().any(|t| t.iter().any(|c| c.g > 60))).await;
    let all = ticks();
    let green = all
        .iter()
        .position(|t| t.iter().any(|c| c.g > 60))
        .expect("a frame with the new green marker");
    // The tick just before the new colours: the old picture at the moment the save landed. The two
    // are one engine frame apart, so a kept clock moves the marker by an LED at most, while a clock
    // that started over jumps back to the pattern's own LED, well behind where the run had got to.
    let before = green.checked_sub(1).map(|i| at(&all[i], 0)).unwrap_or(0);
    let carried = at(&all[green], 1);
    assert!(
        carried >= before && (carried - before) <= 3,
        "the green marker carries on from run position {before} (it was {carried}; a restart would show 7)"
    );
}

/// The fit the editor's sentence reports for a sequence group is the fit of the whole run.
#[tokio::test]
async fn a_sequence_group_reports_the_fit_of_the_whole_run() {
    use canvas_proto::scene::SequenceShape;
    let rig = sequence_rig(Motion::none(), None, [FAN1, FAN2, STRIP]).await;
    let ids = [format!("{FAN1}#z0"), format!("{FAN2}#z0"), format!("{STRIP}#z0")];
    let want = |fits: Vec<FitKind>| -> Vec<LightFit> {
        ids.iter().zip(fits).map(|(light, fit)| LightFit { light: light.clone(), fit }).collect()
    };
    // Each light its own: the ring of 26 is stretched onto each fan and is another shape for the strip.
    assert_eq!(
        group_fits(&rig).await["run"],
        want(vec![FitKind::Stretched, FitKind::Stretched, FitKind::OtherShape]),
        "no sequence"
    );
    // One ring of 8 + 8 + 10 = 26 on a pattern built on a ring of 26: it fits exactly, on every light.
    put_main(&rig, |s| s.groups[0].sequence = Some(SequenceShape::Ring)).await;
    assert_eq!(group_fits(&rig).await["run"], want(vec![FitKind::Exact; 3]), "one ring");
    // One line: the ring becomes a line, another shape, on every light.
    put_main(&rig, |s| s.groups[0].sequence = Some(SequenceShape::Line)).await;
    assert_eq!(group_fits(&rig).await["run"], want(vec![FitKind::OtherShape; 3]), "one line");
    // Two fans only: the run is 8 + 8 = 16 LEDs, a ring of 16 from a pattern of 26.
    put_main(&rig, |s| {
        s.groups[0].sequence = Some(SequenceShape::Ring);
        s.groups[0].members.pop();
    })
    .await;
    assert_eq!(
        group_fits(&rig).await["run"],
        want(vec![FitKind::Stretched, FitKind::Stretched, FitKind::Stretched])[..2].to_vec(),
        "a ring of 16"
    );
}

// ---------------------------------------------------------------- #481: the frames' memory across lights

/// Waits (real time) until `sink`'s newest frame is one `ok` accepts.
async fn shows_where(sink: &Rec, what: &str, ok: impl Fn(&[Rgb8]) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if sink.last().is_some_and(|f| ok(&f)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what}: the frame never matched; last frame starts {:?}", sink.last().map(|f| f[..3.min(f.len())].to_vec()));
}

/// A pattern of `frames` frames of `LEDS` LEDs that each hold 10 ms, so it plays through them (a
/// pattern with one frame, or with no time in its cycle, stands still and keeps none fitted).
fn reel(frames: usize, lit: Rgb8) -> Pattern {
    let mut reel = pattern_with("Reel", LEDS, lit);
    let frame = reel.frames[0].clone();
    reel.frames = (0..frames).map(|_| Frame { hold: 10, ..frame.clone() }).collect();
    reel
}

/// A scene of two groups: `big` plays `effect` on the lights of `big` (a strip each), `small` plays
/// the reel on one ring of twelve.
fn reel_scene(big: &[String], effect: &str, props: Props) -> ScenesFile {
    let members: Vec<&str> = big.iter().map(String::as_str).collect();
    ScenesFile {
        schema_version: SCENES_SCHEMA,
        active: "main".into(),
        scenes: vec![Scene {
            id: "main".into(),
            name: "Main".into(),
            rev: 1,
            groups: vec![
                group("big", &members, effect, props),
                group("small", &["fake:a#z0"], "pattern:Reel", Props::new()),
            ],
        }],
    }
}

/// Four strips of `sizes` LEDs and the ring of twelve the `small` group plays on; the `big` group
/// plays the reel of 256 frames.
async fn reel_rig(sizes: [u32; 4]) -> Rig {
    reel_rig_with(sizes, reel(256, Rgb8::new(0, 0, 200)), "pattern:Reel", Props::new()).await
}

/// [`reel_rig`] with the reel the store holds, and what the `big` group plays, as given.
async fn reel_rig_with(sizes: [u32; 4], stored: Pattern, effect: &str, props: Props) -> Rig {
    let ids: Vec<String> = (0..4).map(|i| format!("fake:big{i}")).collect();
    let mut devices = vec![device(A), device(B)];
    let mut lights = vec![light(A), light(B)];
    for (id, leds) in ids.iter().zip(sizes) {
        devices.push(device_of(id, leds));
        lights.push(light_of(id, Arrangement::Strip));
    }
    let big: Vec<String> = ids.iter().map(|id| format!("{id}#z0")).collect();
    rig_full(vec![stored], reel_scene(&big, effect, props), devices, lights).await
}

#[tokio::test]
async fn a_pattern_past_the_scenes_frame_budget_is_dark_with_a_sentence_and_the_rest_play() {
    // 256 frames on four different strips of about 4096 LEDs: 256 x 16 378 fitted LED-frames, past
    // the 4 000 000 a scene keeps (one such light alone is 1 048 576, 28 MB and 124 ms to fit).
    let rig = reel_rig([4096, 4095, 4094, 4093]).await;
    let states = group_states(&rig).await;
    let said = states.get("big").cloned().flatten().expect("the big group is dark and says why");
    assert!(
        said.contains("Reel has 256 frames") && said.contains("4192768") && said.contains("4000000"),
        "the sentence names the pattern, its frames, its cost and the limit: {said}"
    );
    assert!(said.contains("fewer frames") && said.contains("fewer lights"), "and the way out: {said}");
    assert_eq!(states.get("small"), Some(&None), "the group inside the budget is not failed");
    // The group that fits plays the reel (LED 0 blue); every light of the group that does not is black.
    shows_marked(&rig.a, LEDS as usize, 0, Rgb8::new(0, 0, 200), "the group inside the budget").await;
    for id in ["fake:big0", "fake:big3"] {
        shows_where(&sink(&rig, id), "a light of the group past the budget", |f| {
            f.len() > 4000 && f.iter().all(|c| *c == Rgb8::BLACK)
        })
        .await;
    }
}

#[tokio::test]
async fn lights_of_one_shape_cost_one_fit_so_the_same_four_lights_play() {
    // The same four strips at 4096 LEDs each share one fitted set: 256 x 4096 = 1 048 576, inside it.
    let rig = reel_rig([4096; 4]).await;
    let states = group_states(&rig).await;
    assert_eq!(states.get("big"), Some(&None), "four lights of one shape are one fit: {states:?}");
    let want = Rgb8::new(0, 0, 200);
    // The ring of twelve is stretched onto a strip of 4096 by LED order: its first colour covers the first LEDs.
    shows_where(&sink(&rig, "fake:big0"), "a light of the shared fit", |f| {
        f.len() == 4096 && f[0] == want && f[4095] == Rgb8::BLACK
    })
    .await;
}

/// The 256-frame reel on four strips of four different sizes is past the scene's budget on every path
/// that builds a pattern group, not only the full build (#481): each of the three below leaves the
/// group dark with the sentence, and the group inside the budget playing.
const OVER: [u32; 4] = [4096, 4095, 4094, 4093];

fn said_over(states: &std::collections::BTreeMap<String, Option<String>>) -> String {
    let said = states.get("big").cloned().flatten().expect("the big group is dark and says why");
    assert!(said.contains("Reel has 256 frames") && said.contains("4000000"), "the sentence names the pattern and the limit: {said}");
    assert_eq!(states.get("small"), Some(&None), "the group inside the budget is not failed: {states:?}");
    said
}

async fn big_strips_go_dark(rig: &Rig, what: &str) {
    for id in ["fake:big0", "fake:big3"] {
        shows_where(&sink(rig, id), what, |f| f.len() > 4000 && f.iter().all(|c| *c == Rgb8::BLACK)).await;
    }
}

#[tokio::test]
async fn a_scene_save_that_gives_a_group_the_reel_past_the_budget_leaves_it_dark_with_a_sentence() {
    // The group plays a solid red on the four strips; the store holds the 256-frame reel.
    let rig = reel_rig_with(OVER, reel(256, Rgb8::new(0, 0, 200)), "solid", color(RED)).await;
    shows_where(&sink(&rig, "fake:big0"), "the strips before", |f| f.len() > 4000 && f[0] == rgb(RED)).await;
    assert_eq!(group_states(&rig).await.get("big"), Some(&None));

    put_main(&rig, |scene| {
        scene.groups[0].plays = Plays::Effect { effect: "pattern:Reel".into(), props: Props::new() };
    })
    .await;

    said_over(&group_states(&rig).await);
    big_strips_go_dark(&rig, "a group the save gave the reel past the budget").await;
    shows_marked(&rig.a, LEDS as usize, 0, Rgb8::new(0, 0, 200), "the group inside the budget").await;
}

#[tokio::test]
async fn a_pattern_save_that_grows_past_the_budget_leaves_the_groups_playing_it_dark_with_a_sentence() {
    // The store holds a one-frame reel, which the four strips play inside the budget.
    let rig = reel_rig_with(OVER, reel(1, Rgb8::new(0, 0, 200)), "pattern:Reel", Props::new()).await;
    shows_where(&sink(&rig, "fake:big0"), "the strips before", |f| f.len() > 4000 && f[0] == Rgb8::new(0, 0, 200)).await;
    assert_eq!(group_states(&rig).await.get("big"), Some(&None));

    // The reel is saved again with 256 frames.
    let (status, answer) = put_pattern_body(&rig, "pattern:Reel", &reel(256, Rgb8::new(0, 0, 200))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    said_over(&group_states(&rig).await);
    big_strips_go_dark(&rig, "a group playing a pattern saved past the budget").await;
}

#[tokio::test]
async fn a_pattern_draft_keeps_the_groups_past_the_budget_dark_with_their_sentence() {
    // The scene's full build already refused the big group.
    let rig = reel_rig(OVER).await;
    said_over(&group_states(&rig).await);
    big_strips_go_dark(&rig, "the group past the budget before the draft").await;

    // A draft on the rings takes the lights over: the strips are not its kind, so the scene's groups
    // play on, dimmed. The refused group is still refused.
    let (status, answer) = put_pattern_draft(&rig, &pattern_with("Draft", LEDS, Rgb8::new(0, 200, 0)), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows_marked(&rig.a, LEDS as usize, 0, Rgb8::new(0, 200, 0), "the ring of the draft's kind").await;
    // The takeover's frame is on every light by now. Wait for a few more frames of the strips (the
    // reel would be in them if it were built), then read what they hold.
    for id in ["fake:big0", "fake:big3"] {
        let strip = sink(&rig, id);
        let until = strip.seen() + 5;
        shows_where(&strip, "five more frames on the strip", |_| strip.seen() >= until).await;
        let last = strip.last().expect("a frame");
        assert!(last.iter().all(|c| *c == Rgb8::BLACK), "{id}: a group past the budget plays under a draft");
    }
    said_over(&group_states(&rig).await);
}

/// A valid HTML effect for the draft PUTs over the pattern rigs.
const DRAFT_HTML: &str = r##"<head><title>Draft</title></head>
<body><canvas id="c" width="320" height="200"></canvas></body>
<script>
var c = document.getElementById("c");
var ctx = c.getContext("2d");
ctx.fillStyle = "#ffcc00";
ctx.fillRect(0, 0, 320, 200);
function draw() { window.requestAnimationFrame(draw); }
window.requestAnimationFrame(draw);
</script>"##;

/// The effect the scene's `draft` group plays, saved by the test as the editor saves one: a draft
/// of this file's own source lands on the groups that play it (#518).
const SAVED_HTML: &str = r##"<head>
  <title>Saved</title>
  <meta description="An effect the draft takeover lands on." />
</head>
<body><canvas id="stage" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("stage");
  var ctx = c.getContext("2d");
  function draw() {
    ctx.fillStyle = "#101020";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = "#ff00ff";
    ctx.fillRect(40, 40, 30, 30);
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

/// An HTML draft PUT, as the editor sends one.
async fn put_html_draft(rig: &Rig, html: &str) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({
        "html": html,
        "props": {},
        "epoch": EPOCH2,
        "seq": 1,
    });
    send(rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await
}

/// Saves `html` as `file_name` through the import route, as the editor does, and returns the
/// effect's id.
async fn import_html_effect(rig: &Rig, file_name: &str, html: &str) -> String {
    let body = serde_json::json!({ "file_name": file_name, "html": html, "replace": false });
    let (status, answer) = send(rig, "POST", routes::EFFECTS_IMPORT, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["effect"]["id"].as_str().expect("the imported effect's id").to_string()
}

/// The reel past the budget on four strips, and a group playing `html:take518`, which the test
/// imports after the rig starts (#518).
async fn html_draft_over_budget_rig() -> Rig {
    let ids: Vec<String> = (0..4).map(|i| format!("fake:big{i}")).collect();
    let sizes = OVER;
    // rig_full needs the two ordinary fake devices for its A/B handles.
    let mut devices = vec![device(A), device(B)];
    let mut lights = vec![light(A), light(B)];
    for (id, leds) in ids.iter().zip(sizes) {
        devices.push(device_of(id, leds));
        lights.push(light_of(id, Arrangement::Strip));
    }
    let big: Vec<String> = ids.iter().map(|id| format!("{id}#z0")).collect();
    let big_refs: Vec<&str> = big.iter().map(String::as_str).collect();
    rig_full(
        vec![reel(256, Rgb8::new(0, 0, 200))],
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "main".into(),
            scenes: vec![Scene {
                id: "main".into(),
                name: "Main".into(),
                rev: 1,
                groups: vec![
                    group("big", &big_refs, "pattern:Reel", Props::new()),
                    group("draft", &["fake:a#z0"], "html:take518", Props::new()),
                ],
            }],
        },
        devices,
        lights,
    )
    .await
}

/// An HTML draft over a scene with a group past the frame budget goes through the takeover that
/// rebuilds every group (`build_and_set_with`), and leaves the pattern group dark with its
/// sentence there too (#518).
#[tokio::test]
async fn an_html_draft_takeover_rebuild_keeps_the_over_budget_group_dark() {
    let rig = html_draft_over_budget_rig().await;
    let states = group_states(&rig).await;
    assert!(
        states.get("big").cloned().flatten().is_some_and(|why| why.contains("4000000")),
        "the reel past the budget is dark on the full build: {states:?}"
    );
    big_strips_go_dark(&rig, "the group past the budget before the draft").await;

    // The effect is imported as the editor imports one; a scene save builds the group that was
    // dark only because the file was not there when the scene started.
    assert_eq!(import_html_effect(&rig, "take518.html", SAVED_HTML).await, "html:take518");
    put_main(&rig, |_| {}).await;
    let states = group_states(&rig).await;
    assert_eq!(states.get("draft"), Some(&None), "the imported effect plays: {states:?}");
    assert!(
        states.get("big").cloned().flatten().is_some_and(|why| why.contains("4000000")),
        "the reel is still past the budget: {states:?}"
    );

    // A draft of a file no group plays: the engine holds the takeover group alone.
    let (status, answer) = put_html_draft(&rig, DRAFT_HTML).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["applied"], serde_json::json!(true), "{answer}");

    // A draft of the saved effect's own source lands on the group that plays it. The engine does
    // not hold the scene's groups, so the whole scene is built again through `build_and_set_with`
    // with the draft on its group: the reel is refused there too.
    let saved = std::fs::read_to_string(rig.home.path.join("effects").join("take518.html"))
        .expect("the imported file is in the effects folder");
    let (status, answer) = put_html_draft(&rig, &saved).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["applied"], serde_json::json!(true), "{answer}");

    let states = group_states(&rig).await;
    let why = states.get("big").cloned().flatten().expect("the big group is dark and says why");
    assert!(
        why.contains("Reel has 256 frames") && why.contains("4000000"),
        "the sentence names the pattern and the limit: {why}"
    );
    assert_eq!(states.get("draft"), Some(&None), "the group the draft landed on plays: {states:?}");
    big_strips_go_dark(&rig, "the group past the budget under the draft takeover").await;
}

/// `reel` under its own id, for scenes with more than one pattern.
fn named_reel(id: &str, frames: usize, lit: Rgb8) -> Pattern {
    let mut reel = reel(frames, lit);
    reel.id = format!("pattern:{id}");
    reel
}

/// The frames each reel of the two-reel rigs has: enough that two together are past the scene's
/// budget, and one alone fits.
const REEL_FRAMES: usize = 128;

/// [`two_reel_rig_with`] with a full reel on each group: the pair is past the budget, so the later
/// group is dark from the start.
async fn two_reel_rig() -> Rig {
    two_reel_rig_with(REEL_FRAMES).await
}

/// Two groups, `a` and `b`, each playing a reel on four strips of four different sizes: either
/// reel of [`REEL_FRAMES`] alone fits the scene's budget, the two together do not (#518). `a` plays
/// `pattern:A` (blue) with `a_frames` frames and `b` plays `pattern:B` (green) with
/// [`REEL_FRAMES`], so saving one changes the budget of the other without naming it.
async fn two_reel_rig_with(a_frames: usize) -> Rig {
    let ids: Vec<String> = (0..8).map(|i| format!("fake:s{i}")).collect();
    let sizes = [4096u32, 4095, 4094, 4093, 4092, 4091, 4090, 4089];
    // rig_full needs the two ordinary fake devices for its A/B handles; no group uses them.
    let mut devices = vec![device(A), device(B)];
    let mut lights = vec![light(A), light(B)];
    for (id, leds) in ids.iter().zip(sizes) {
        devices.push(device_of(id, leds));
        lights.push(light_of(id, Arrangement::Strip));
    }
    let members: Vec<String> = ids.iter().take(4).map(|id| format!("{id}#z0")).collect();
    let other: Vec<String> = ids.iter().skip(4).map(|id| format!("{id}#z0")).collect();
    let member_refs: Vec<&str> = members.iter().map(String::as_str).collect();
    let other_refs: Vec<&str> = other.iter().map(String::as_str).collect();
    rig_full(
        vec![
            named_reel("A", a_frames, Rgb8::new(0, 0, 200)),
            named_reel("B", REEL_FRAMES, Rgb8::new(0, 200, 0)),
        ],
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "main".into(),
            scenes: vec![Scene {
                id: "main".into(),
                name: "Main".into(),
                rev: 1,
                groups: vec![
                    group("a", &member_refs, "pattern:A", Props::new()),
                    group("b", &other_refs, "pattern:B", Props::new()),
                ],
            }],
        },
        devices,
        lights,
    )
    .await
}

#[tokio::test]
async fn a_group_the_budget_left_dark_plays_when_another_pattern_shrinks() {
    let rig = two_reel_rig().await;
    let states = group_states(&rig).await;
    assert_eq!(states.get("a"), Some(&None), "the first group fits alone: {states:?}");
    assert!(states.get("b").cloned().flatten().is_some(), "the second group is dark under the shared budget: {states:?}");
    for id in ["fake:s4", "fake:s7"] {
        shows_where(&sink(&rig, id), "a strip of the refused group", |f| {
            f.len() > 4000 && f.iter().all(|c| *c == Rgb8::BLACK)
        })
        .await;
    }

    // Saving A with one frame frees the budget. The save names only A; B must still be rebuilt.
    let small = named_reel("A", 1, Rgb8::new(0, 0, 200));
    let (status, answer) = put_pattern_body(&rig, "pattern:A", &small).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let states = group_states(&rig).await;
    assert_eq!(states.get("a"), Some(&None), "the shrunken group plays: {states:?}");
    assert_eq!(states.get("b"), Some(&None), "the group the budget left dark now fits and plays: {states:?}");
    for id in ["fake:s4", "fake:s7"] {
        shows_where(&sink(&rig, id), "a strip of the recovered group", |f| {
            f.len() > 4000 && f[0] == Rgb8::new(0, 200, 0)
        })
        .await;
    }
}

/// A pattern save that grows past the shared budget turns a group it does not name dark, with the
/// budget's reason, as a scene save does (#518). The budget is one for the whole scene, so the
/// group that was inside it and no longer is cannot keep playing.
#[tokio::test]
async fn a_pattern_save_that_grows_past_the_budget_turns_an_unnamed_group_dark() {
    // `a` is one frame and `b` is a full reel: together they are 2 096 384 + 16 378, inside the
    // 4 000 000 a scene keeps, so both play.
    let rig = two_reel_rig_with(1).await;
    let states = group_states(&rig).await;
    assert_eq!(states.get("a"), Some(&None), "the one-frame group plays: {states:?}");
    assert_eq!(states.get("b"), Some(&None), "the full reel fits beside it: {states:?}");

    // Saving `a` with a full reel puts the pair past the budget: the later group is refused, as a
    // full build refuses it, though the save names only `a`.
    let grown = named_reel("A", REEL_FRAMES, Rgb8::new(0, 0, 200));
    let (status, answer) = put_pattern_body(&rig, "pattern:A", &grown).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let states = group_states(&rig).await;
    assert_eq!(states.get("a"), Some(&None), "the first group keeps the budget: {states:?}");
    let why = states.get("b").cloned().flatten().expect("the group the budget now refuses is dark");
    assert!(
        why.contains("B has 128 frames") && why.contains("4000000"),
        "the sentence names why: {why}"
    );
    for id in ["fake:s4", "fake:s7"] {
        shows_where(&sink(&rig, id), "a strip of the group the grown pattern refuses", |f| {
            f.len() > 4000 && f.iter().all(|c| *c == Rgb8::BLACK)
        })
        .await;
    }
}

/// A valid HTML effect for the groups the HTML runtime limit leaves dark (#419).
const LIMIT_HTML: &str = r##"<head>
  <title>Limit</title>
  <meta description="An effect for the HTML runtime limit." />
</head>
<body><canvas id="stage" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("stage");
  var ctx = c.getContext("2d");
  function draw() {
    ctx.fillStyle = "#203040";
    ctx.fillRect(0, 0, 320, 200);
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

/// Three groups playing one HTML effect on the rig's three lights, loaded with the effect already
/// in the folder: the scene starts three HTML runtimes where the limit is two (#370).
async fn html_limit_rig() -> Rig {
    let rig = rig_full(
        Vec::new(),
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "main".into(),
            scenes: vec![Scene {
                id: "main".into(),
                name: "Main".into(),
                rev: 1,
                groups: vec![
                    group("one", &["fake:a#z0"], "html:limittest", Props::new()),
                    group("two", &["fake:b#z0"], "html:limittest", Props::new()),
                    group("three", &["fake:c#z0"], "html:limittest", Props::new()),
                ],
            }],
        },
        vec![device(A), device(B), device(C)],
        vec![light(A), light(B), light(C)],
    )
    .await;
    // The file is written after start-up, then found; the scene goes on again with it present.
    std::fs::write(rig.home.path.join("effects").join("limittest.html"), LIMIT_HTML)
        .expect("the effect is written");
    rig.app.user_effects.rescan(&rig.home.path.join("effects"));
    canvas_daemon::scenes::apply_active(&rig.app, false).await;
    rig
}

/// #419: a pattern save must not retry a group the HTML limit left dark. The save frees no runtime,
/// so building that group would run the scene past the cap it was loaded past (#518).
#[tokio::test]
async fn a_pattern_save_does_not_build_a_group_the_html_limit_left_dark() {
    let rig = html_limit_rig().await;
    let over = canvas_daemon::scenes::html_limit_sentence(3);
    let built = rig.app.scene.builds();
    let failures = rig.app.scene.failures();
    assert_eq!(failures.len(), 1, "the two runtimes the limit allows are running: {failures:?}");
    assert_eq!(
        failures.get("three").map(String::as_str),
        Some(over.as_str()),
        "the third group waits under the limit: {failures:?}"
    );

    // A pattern save no group plays: the incremental path revisits the groups it names, and the
    // ones a failure left dark, which must not include a group the HTML limit left dark.
    let mut unused = reel(1, Rgb8::new(0, 0, 200));
    unused.id = "pattern:Unused".into();
    let (status, answer) = put_pattern_body(&rig, "pattern:Unused", &unused).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    assert_eq!(rig.app.scene.builds(), built, "the pattern save built no HTML runtime");
    let failures = rig.app.scene.failures();
    assert_eq!(
        failures.get("three").map(String::as_str),
        Some(over.as_str()),
        "the group the limit left dark is still dark, with the same reason: {failures:?}"
    );
}

/// Which colour a marker is, as the channel it lights: 0 red, 1 green, 2 blue.
type Channel = usize;

fn channel_of(c: &Rgb8, channel: Channel) -> u8 {
    [c.r, c.g, c.b][channel]
}

/// Where the marker of `channel` is in `frame`: its brightest LED, once it is lit enough to be one
/// (a turning pattern is blended across two LEDs, and the lights dim it a little).
fn marker(frame: &[Rgb8], channel: Channel) -> Option<usize> {
    let (index, level) = frame.iter().enumerate().map(|(i, c)| (i, channel_of(c, channel))).max_by_key(|(_, level)| *level)?;
    (level > 60).then_some(index)
}

/// Where the marker of `channel` is in the first frame after the first `since` that has one; waits (real
/// time) for that frame.
async fn first_marker_since(sink: &Rec, since: usize, channel: Channel, what: &str) -> usize {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if let Some(index) = sink.since(since).iter().find_map(|f| marker(f, channel)) {
            return index;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what}: no frame showed a marker in channel {channel}");
}

/// A stroke that changes colours only keeps a spinning ring's angle; a change to the motion starts the
/// spin over (#481). Each stroke installs a new effect, which restarted the clock and put the ring back
/// at angle 0 with every stroke.
#[tokio::test]
async fn a_spinning_ring_keeps_its_angle_across_a_colour_only_stroke_and_a_motion_change_restarts_it() {
    let rig = draft_rig(Vec::new()).await;
    let blue = Rgb8::new(0, 0, 200);
    let green = Rgb8::new(0, 200, 0);
    let red = Rgb8::new(200, 0, 0);
    let spun = |colour: Rgb8, turns: f32| {
        let mut p = pattern_with("Draft", LEDS, colour);
        p.motion = Motion::spin(turns);
        p
    };
    // Half a turn a second: a ring of twelve moves a LED every 167 ms.
    let (status, answer) = put_pattern_draft(&rig, &spun(blue, 0.5), EPOCH, 1).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    // Let it turn well away from LED 0 (a third of a turn or more).
    shows_where(&rig.a, "the ring turned away from LED 0", |f| marker(f, 2).is_some_and(|i| (4..=8).contains(&i))).await;

    // The same spin in other colours: it carries on from where it is, never from LED 0.
    let seen = rig.a.seen();
    let (status, answer) = put_pattern_draft(&rig, &spun(green, 0.5), EPOCH, 2).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let carried = first_marker_since(&rig.a, seen, 1, "the colour-only stroke").await;
    assert!((3..=9).contains(&carried), "a colour-only stroke kept the angle; the new colours start at LED {carried}");

    // A different speed is a different motion: the spin starts over at LED 0.
    let seen = rig.a.seen();
    let (status, answer) = put_pattern_draft(&rig, &spun(red, 0.25), EPOCH, 3).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let restarted = first_marker_since(&rig.a, seen, 0, "the motion change").await;
    assert!(matches!(restarted, 0 | 1 | 11), "a motion change starts the spin over; it starts at LED {restarted}");
}
