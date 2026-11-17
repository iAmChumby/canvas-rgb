//! Luke's case on the fakes (Patterns plan, Task 12 Step 1): nine case fans in three roles, three
//! patterns, one lit LED per fan.
//!
//! The whole daemon runs in-process around a real engine and nine fake devices with a recording
//! sink on each, so the case is proved by what the sinks are handed and not by what a route
//! answers. No device, no port 9430, nothing on the hardware.
//!
//! No sleep decides a verdict here: every wait polls a condition against a deadline, and the
//! timing scan (`canvas-svc/tests/timing_class.rs`) reads the loops as waits, not as bounds.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::state::App;
use canvas_proto::api::routes;
use canvas_proto::color::Rgb8;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::pattern::{BuiltOn, Frame, Motion, Pattern};
use canvas_proto::props::Props;
use canvas_proto::scene::{
    Arrangement, Group, GroupMode, Light, LightsFile, Plays, Scene, ScenesFile, LIGHTS_SCHEMA,
    SCENES_SCHEMA,
};
use canvas_proto::traits::{Engine, EngineSettings, FrameSink};
use common::FakeAdapter;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Every fan is a ring of twelve: Luke's case fans once chain setup has split them.
const LEDS: u32 = 12;
/// The scene the case lives in.
const CASE: &str = "case";

/// One role of the case: the pattern it plays, and the colour its fans mark LED 0 with.
struct Role {
    /// The role's name, and its pattern's name: "Intake".
    name: &'static str,
    /// The pattern's id: `pattern:` + the name.
    effect: &'static str,
    /// The colour of LED 0 on this role's fans. Every other LED is off.
    colour: Rgb8,
}

const ROLES: [Role; 3] = [
    Role { name: "Intake", effect: "pattern:Intake", colour: Rgb8::new(210, 30, 40) },
    Role { name: "Exhaust", effect: "pattern:Exhaust", colour: Rgb8::new(30, 190, 60) },
    Role { name: "Radiator", effect: "pattern:Radiator", colour: Rgb8::new(40, 90, 210) },
];

/// The fake device of fan `number` of `role`: `fake:intake1` to `fake:intake3`.
fn fan_device(role: &Role, number: usize) -> String {
    format!("fake:{}{number}", role.name.to_ascii_lowercase())
}

/// The light id of a fan's whole zone.
fn fan_light(role: &Role, number: usize) -> String {
    format!("{}#z0", fan_device(role, number))
}

/// A device whose one linear zone has [`LEDS`] LEDs.
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

/// One fan as a light: its whole zone, a ring of twelve, named "Intake 1" and so on.
fn fan_light_of(role: &Role, number: usize) -> Light {
    Light {
        id: fan_light(role, number),
        device_id: DeviceId(fan_device(role, number)),
        zone_index: 0,
        leds: None,
        name: Some(format!("{} {number}", role.name)),
        arrangement: Arrangement::Ring,
        reversed: false,
        kind: None,
        copies: None,
    }
}

/// A pattern built on a ring of twelve with LED 0 in `colour` and every other LED off.
fn marked(name: &str, colour: Rgb8) -> Pattern {
    let mut colors = vec![Rgb8::BLACK; LEDS as usize];
    colors[0] = colour;
    Pattern {
        id: format!("pattern:{name}"),
        built_on: BuiltOn { led_count: LEDS, arrangement: Arrangement::Ring, grid: None },
        frames: vec![Frame { colors, hold: 0, transition: None, extra: serde_json::Map::new() }],
        motion: Motion::none(),
        playback: None,
        extra: serde_json::Map::new(),
    }
}

/// The case: one scene, three groups of three fans, each playing its role's pattern.
fn case_scene() -> ScenesFile {
    ScenesFile {
        schema_version: SCENES_SCHEMA,
        active: CASE.into(),
        scenes: vec![Scene {
            id: CASE.into(),
            name: "Case".into(),
            rev: 1,
            groups: ROLES
                .iter()
                .map(|role| Group {
                    id: role.name.to_ascii_lowercase(),
                    name: format!("{} fans", role.name),
                    members: (1..=3).map(|number| fan_light(role, number)).collect(),
                    plays: Plays::Effect { effect: role.effect.into(), props: Props::new() },
                    mode: GroupMode::FlowAcross,
                    brightness: 100,
                })
                .collect(),
        }],
    }
}

struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-patterns-case-{}-{:?}",
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

/// A sink that keeps the newest frames it was handed and counts every one.
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
        if frames.len() == 4096 {
            frames.pop_front();
        }
        frames.push_back(colors);
        self.seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

struct Rig {
    router: axum::Router,
    app: Arc<App>,
    /// Every fan's sink, by device id.
    sinks: BTreeMap<String, Arc<Rec>>,
    /// Kept alive so the temp home goes away when the test ends.
    _home: TempHome,
}

impl Rig {
    fn sink(&self, device: &str) -> &Arc<Rec> {
        self.sinks.get(device).unwrap_or_else(|| panic!("no sink for {device}"))
    }
}

/// The daemon's state around a real engine, the nine fake fans, and the three patterns on disk.
async fn case_rig() -> Rig {
    let home = TempHome::new();
    let effects = home.path.join("effects");
    std::fs::create_dir_all(&effects).expect("the effects folder is created");
    let patterns: Vec<Pattern> = ROLES.iter().map(|role| marked(role.name, role.colour)).collect();
    canvas_daemon::patterns::write_file(&home.path, &patterns).expect("patterns.json is written");

    let devices: Vec<DeviceInfo> =
        ROLES.iter().flat_map(|role| (1..=3).map(|n| device(&fan_device(role, n)))).collect();
    let lights: Vec<Light> = ROLES.iter().flat_map(|role| (1..=3).map(|n| fan_light_of(role, n))).collect();

    let mut store = canvas_daemon::storage::Store::at(&home.path);
    store.lights = LightsFile { chains: Vec::new(), schema_version: LIGHTS_SCHEMA, lights };
    store.scenes = case_scene();

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

    let mut sinks = BTreeMap::new();
    for device in &devices {
        let sink = Arc::new(Rec::default());
        engine.add_sink(device.id.clone(), Arc::clone(&sink) as Arc<dyn FrameSink>);
        sinks.insert(device.id.0.clone(), sink);
    }

    // The way start-up applies the saved scenes.
    canvas_daemon::scenes::restore_active(&app).await;
    Rig { router: canvas_daemon::api::router(app.clone()), app, sinks, _home: home }
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

fn pattern_path(id: &str) -> String {
    route(routes::PATTERN, &[("id", id)])
}

/// Waits (real time) until `sink`'s newest frame answers `ok`, for at most twenty seconds.
async fn wait_for_frame(
    sink: &Rec,
    what: &str,
    mut ok: impl FnMut(&[Rgb8]) -> bool,
) -> Vec<Rgb8> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if let Some(frame) = sink.last() {
            if ok(&frame) {
                return frame;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "{what}: the frame never came; the newest starts {:?}",
        sink.last().map(|f| f[..3.min(f.len())].to_vec())
    );
}

/// Waits (real time) until `sink` has handed over a frame after the first `since`, then every
/// frame it has handed over since.
async fn frames_since(sink: &Rec, since: usize, what: &str) -> Vec<Vec<Rgb8>> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if sink.seen() > since {
            return sink.since(since);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what}: no frame was handed over after the {since} already seen");
}

/// The frame of a fan that plays a marked pattern: LED 0 in `colour`, every other LED off.
fn marked_frame(frame: &[Rgb8], colour: Rgb8) -> bool {
    frame.len() == LEDS as usize
        && frame[0] == colour
        && frame[1..].iter().all(|c| *c == Rgb8::BLACK)
}

/// The frame of a fan in a group that is dark: every LED off.
fn dark_frame(frame: &[Rgb8]) -> bool {
    frame.len() == LEDS as usize && frame.iter().all(|c| *c == Rgb8::BLACK)
}

/// Waits until every fan of every role shows its role's colour, then takes the frame count of
/// each sink, so a later change can be told apart from what was there before it.
async fn settle(rig: &Rig) -> BTreeMap<String, usize> {
    let mut marks = BTreeMap::new();
    for role in &ROLES {
        for number in 1..=3 {
            let device = fan_device(role, number);
            let what = format!("{} {number}", role.name);
            wait_for_frame(rig.sink(&device), &what, |frame| marked_frame(frame, role.colour)).await;
            let seen = rig.sink(&device).seen();
            marks.insert(device, seen);
        }
    }
    marks
}

/// (a) Each fan's frame on its fake sink has exactly one lit LED, at index 0, in its group's
/// colour.
#[tokio::test]
async fn each_fan_shows_one_lit_led_at_the_top_in_its_group_colour() {
    let rig = case_rig().await;
    for role in &ROLES {
        for number in 1..=3 {
            let device = fan_device(role, number);
            let what = format!("{} {number}", role.name);
            let frame =
                wait_for_frame(rig.sink(&device), &what, |frame| marked_frame(frame, role.colour))
                    .await;
            let lit = frame.iter().filter(|c| **c != Rgb8::BLACK).count();
            assert_eq!(lit, 1, "{what}: exactly one LED is lit: {frame:?}");
            assert_eq!(frame[0], role.colour, "{what}: the lit LED is index 0, in its role's colour");
        }
    }
}

/// (b) Editing `pattern:Exhaust` through the route changes only the exhaust fans' frames.
#[tokio::test]
async fn editing_the_exhaust_pattern_changes_only_the_exhaust_fans() {
    let rig = case_rig().await;
    let marks = settle(&rig).await;

    // A colour no role has, so the change cannot be mistaken for a frame already on its way.
    let edited = Rgb8::new(235, 175, 25);
    let body = serde_json::to_value(marked("Exhaust", edited)).unwrap();
    let (status, answer) = send(&rig, "PUT", &pattern_path("pattern:Exhaust"), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let saved: Pattern = serde_json::from_value(answer).expect("the save answers with the pattern");
    assert_eq!(saved.first_frame().colors[0], edited, "the saved pattern carries the new colour");

    let exhaust = &ROLES[1];
    for number in 1..=3 {
        let device = fan_device(exhaust, number);
        let what = format!("Exhaust {number} after the edit");
        let frame =
            wait_for_frame(rig.sink(&device), &what, |frame| marked_frame(frame, edited)).await;
        assert_eq!(frame[0], edited, "{what}: LED 0 is the new colour");
    }

    // The intake and radiator fans kept their own pattern in every frame after the edit: the
    // change reached the exhaust groups only.
    for role in [&ROLES[0], &ROLES[2]] {
        for number in 1..=3 {
            let device = fan_device(role, number);
            let what = format!("{} {number} after the exhaust edit", role.name);
            let frames = frames_since(rig.sink(&device), marks[&device], &what).await;
            for frame in &frames {
                assert!(
                    marked_frame(frame, role.colour),
                    "{what}: a fan that does not play the edited pattern changed: {frame:?}"
                );
            }
        }
    }
}

/// (c) Deleting a played pattern is refused (409, naming the scene) until forced, and forced it
/// darkens only that group.
#[tokio::test]
async fn deleting_a_played_pattern_is_refused_until_forced_and_then_darkens_only_that_group() {
    let rig = case_rig().await;
    let marks = settle(&rig).await;
    let exhaust = &ROLES[1];

    let (status, answer) = send(&rig, "DELETE", &pattern_path("pattern:Exhaust"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("Exhaust"), "the refusal names the pattern: {message}");
    assert!(message.contains("Case"), "the refusal names the scene: {message}");
    assert!(message.contains("Delete it anyway"), "the refusal says what a forced delete does: {message}");
    assert!(rig.app.patterns.get("pattern:Exhaust").is_some(), "the pattern is still there");
    // The refusal changed nothing: every exhaust fan still shows its colour.
    for number in 1..=3 {
        let what = format!("Exhaust {number} after the refusal");
        wait_for_frame(rig.sink(&fan_device(exhaust, number)), &what, |frame| {
            marked_frame(frame, exhaust.colour)
        })
        .await;
    }

    let forced = format!("{}?force=true", pattern_path("pattern:Exhaust"));
    let (status, answer) = send(&rig, "DELETE", &forced, None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let note = answer["note"].as_str().unwrap_or_default();
    assert!(note.contains("pattern:Exhaust"), "the answer says a group went dark: {answer}");
    assert!(rig.app.patterns.get("pattern:Exhaust").is_none(), "the pattern is gone");

    // The exhaust fans go dark; the intake and radiator fans keep their colour in every frame
    // after the delete.
    for number in 1..=3 {
        let device = fan_device(exhaust, number);
        let what = format!("Exhaust {number} after the forced delete");
        let frame = wait_for_frame(rig.sink(&device), &what, dark_frame).await;
        assert!(frame.iter().all(|c| *c == Rgb8::BLACK), "{what}: the fan is dark");
    }
    for role in [&ROLES[0], &ROLES[2]] {
        for number in 1..=3 {
            let device = fan_device(role, number);
            let what = format!("{} {number} after the forced delete", role.name);
            let frames = frames_since(rig.sink(&device), marks[&device], &what).await;
            for frame in &frames {
                assert!(
                    marked_frame(frame, role.colour),
                    "{what}: a group that does not play the deleted pattern went dark: {frame:?}"
                );
            }
        }
    }

    // The scene says the exhaust group is dark and names the pattern; the other two run.
    let (status, body) = send(&rig, "GET", routes::SCENES, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let scene: canvas_proto::api::ScenesResponse = serde_json::from_value(body).unwrap();
    let failed: BTreeMap<String, Option<String>> =
        scene.group_states.into_iter().map(|state| (state.id, state.failed)).collect();
    let reason = failed
        .get("exhaust")
        .cloned()
        .flatten()
        .expect("the exhaust group is dark and says why");
    assert!(reason.contains("pattern:Exhaust"), "the failure names the pattern: {reason}");
    assert_eq!(failed.get("intake"), Some(&None), "the intake group is not failed");
    assert_eq!(failed.get("radiator"), Some(&None), "the radiator group is not failed");
}
