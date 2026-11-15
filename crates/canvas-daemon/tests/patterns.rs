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
    /// Every device's sink, by device id.
    sinks: std::collections::BTreeMap<String, Arc<Rec>>,
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
    Rig { router: canvas_daemon::api::router(app.clone()), app, a, b, sinks, home }
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
    assert!(message.contains("force"), "and says how to force it: {message}");
    assert!(rig.app.patterns.get("pattern:Intake").is_some(), "it is still there");
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
        frames: vec![Frame { colors, hold: 0, extra: serde_json::Map::new() }],
        motion: Motion::none(),
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
async fn a_group_playing_a_canvas_effect_has_no_flags() {
    let rig = rig().await;
    let fits = group_fits(&rig).await;
    assert!(fits["left"].is_empty(), "a canvas group has no fits: {:?}", fits["left"]);
    assert!(!fits["right"].is_empty(), "the pattern group has its fits");
}


