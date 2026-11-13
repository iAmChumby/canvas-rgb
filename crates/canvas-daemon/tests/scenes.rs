//! The scenes API and the daemon running a scene (Scenes plan, Task 6).
//!
//! A real engine with two fake devices and a recording sink on each: a route that changes what
//! plays is proved by what the sinks are handed, not by what the handler says. No device, no
//! port 9430: the router is driven in-process.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::state::App;
use canvas_proto::api::{routes, ScenePut, ScenesResponse, Status};
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::props::{PropValue, Props, PropertySchema};
use canvas_proto::scene::{
    Arrangement, Group, GroupMode, Light, LightsFile, Plays, Scene, ScenesFile, LIGHTS_SCHEMA,
    SCENES_SCHEMA,
};
use canvas_proto::traits::{Canvas, Effect, Engine, EngineSettings, FrameSink, Inputs};
use canvas_proto::Rgb8;
use common::FakeAdapter;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const A: &str = "fake:a";
const B: &str = "fake:b";
const LEDS: u32 = 12;

struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-scenes-{}-{:?}",
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

    /// How many frames the sink has been handed so far.
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
        arrangement: Arrangement::Strip,
        reversed: false,
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
const BLUE: (u8, u8, u8) = (0, 0, 200);
const GREEN: (u8, u8, u8) = (0, 200, 0);

fn rgb(c: (u8, u8, u8)) -> Rgb8 {
    Rgb8::new(c.0, c.1, c.2)
}

/// Two scenes: `main` (left = A red, right = B blue) is active; `other` plays green on both.
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
                    group("right", &["fake:b#z0"], "solid", color(BLUE)),
                ],
            },
            Scene {
                id: "other".into(),
                name: "Other".into(),
                rev: 1,
                groups: vec![group("all", &["fake:a#z0", "fake:b#z0"], "solid", color(GREEN))],
            },
        ],
    }
}

/// The source of the saved user effect the rig's user effects hold.
const MINE: &str = "<html><body>the saved effect</body></html>";
/// A source no saved effect has: a draft of it plays on every light.
const UNUSED: &str = "<html><body>an effect nobody saved</body></html>";

/// An effect that paints one colour, whatever it is.
struct Solid {
    colour: Rgb8,
}

impl Effect for Solid {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "fake:solid".into(),
            name: "Fake Solid".into(),
            description: String::new(),
            needs: Default::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        let c = self.colour;
        canvas.fill([c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0]);
    }
}

/// The builder the rig's effects and drafts use: a draft paints green, the saved effect it
/// takes over paints red.
fn fake_builder() -> canvas_daemon::user_effects::Builder {
    Arc::new(|id: &str, _source: &str| {
        let colour = if id.starts_with("draft:") { rgb(GREEN) } else { rgb(RED) };
        Ok(Box::new(Solid { colour }) as Box<dyn Effect>)
    })
}

/// The scenes the draft tests use: the left group plays the saved user effect, the right one a
/// solid colour.
fn draft_scenes_file() -> ScenesFile {
    ScenesFile {
        schema_version: SCENES_SCHEMA,
        active: "main".into(),
        scenes: vec![
            Scene {
                id: "main".into(),
                name: "Main".into(),
                rev: 1,
                groups: vec![
                    group("left", &["fake:a#z0"], "html:mine", Props::new()),
                    group("right", &["fake:b#z0"], "solid", color(BLUE)),
                ],
            },
            Scene {
                id: "other".into(),
                name: "Other".into(),
                rev: 1,
                groups: vec![group("all", &["fake:a#z0", "fake:b#z0"], "solid", color(GREEN))],
            },
        ],
    }
}

/// The scenes the stack guard test uses: the scene that is not active plays the stack.
fn stack_scenes_file() -> ScenesFile {
    let mut scenes = draft_scenes_file();
    scenes.scenes[1].groups = vec![group("all", &["fake:a#z0"], "stack:Glow", Props::new())];
    scenes
}

struct Rig {
    router: axum::Router,
    app: Arc<App>,
    a: Arc<Rec>,
    b: Arc<Rec>,
    home: TempHome,
}

/// The daemon's state around a real engine and the two devices, with `scenes` active.
///
/// The user effects and the drafts are fakes: a draft paints green and the saved effect it
/// takes over paints red, so a test knows the picture it sees.
async fn rig_with(scenes: ScenesFile) -> Rig {
    rig_in(scenes, &[]).await
}

/// [`rig_with`] with HTML effect files in the daemon's effects folder.
async fn rig_in(scenes: ScenesFile, html: &[(&str, &str)]) -> Rig {
    let rig = rig_unstarted(scenes, html).await;
    // The way start-up applies the saved scenes.
    canvas_daemon::scenes::restore_active(&rig.app).await;
    rig
}

/// The rig before start-up has run: the engine has its device sinks and nothing else.
async fn rig_unstarted(scenes: ScenesFile, html: &[(&str, &str)]) -> Rig {
    let home = TempHome::new();
    let effects = home.path.join("effects");
    std::fs::create_dir_all(&effects).expect("the effects folder is created");
    for (name, source) in html {
        std::fs::write(effects.join(name), source).expect("the effect file is written");
    }
    let mut store = canvas_daemon::storage::Store::at(&home.path);
    store.lights = LightsFile { schema_version: LIGHTS_SCHEMA, lights: vec![light(A), light(B)] };
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
    app.effects_dir = effects.clone();
    // The saved user effect the draft tests take over: its source is `MINE`, so a draft of
    // that source is that effect.
    app.user_effects = Arc::new(canvas_daemon::user_effects::UserEffects::with_stock(
        fake_builder(),
        &[("mine", MINE)],
    ));
    // The drafts are built by the same fake, so a draft's picture is known.
    app.draft = canvas_daemon::draft::Draft::with_builder(Arc::new(|| Duration::ZERO), fake_builder());
    app.user_effects.rescan(&effects);
    let app = Arc::new(app);
    *app.devices.write().await = vec![device(A), device(B)];
    let (a, b) = (Arc::new(Rec::default()), Arc::new(Rec::default()));
    engine.add_sink(DeviceId(A.into()), a.clone());
    engine.add_sink(DeviceId(B.into()), b.clone());
    Rig { router: canvas_daemon::api::router(app.clone()), app, a, b, home }
}

async fn rig() -> Rig {
    rig_with(scenes_file()).await
}

async fn send(
    rig: &Rig,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    send_on(&rig.router, method, path, body).await
}

async fn send_on(
    router: &axum::Router,
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
    let response = router.clone().oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn route(template: &str, pairs: &[(&str, &str)]) -> String {
    pairs.iter().fold(template.to_string(), |path, (key, value)| path.replace(&format!("{{{key}}}"), value))
}

/// Like [`shows`], for a frame that arrived after the first `since` frames of the sink.
async fn shows_after(sink: &Rec, since: usize, want: Rgb8, what: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if sink.since(since).iter().any(|f| f.len() == LEDS as usize && f.iter().all(|c| *c == want)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what}: no frame after #{since} showed {want:?}");
}

/// Waits (real time) until `sink`'s newest frame has `leds` LEDs that are all `want`.
async fn shows(sink: &Rec, want: Rgb8, what: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if sink.last().is_some_and(|f| f.len() == LEDS as usize && f.iter().all(|c| *c == want)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what}: never showed {want:?}; last frame starts {:?}", sink.last().map(|f| f[..3.min(f.len())].to_vec()));
}

async fn stored(rig: &Rig, id: &str) -> Scene {
    rig.app.store.read().await.scenes.scenes.iter().find(|s| s.id == id).cloned().expect("the scene exists")
}

fn scene_of(json: serde_json::Value) -> Scene {
    serde_json::from_value(json).expect("a Scene")
}

// ---------------------------------------------------------------- reading

#[tokio::test]
async fn list_returns_the_active_scene_and_its_groups() {
    let rig = rig().await;
    let (status, body) = send(&rig, "GET", routes::SCENES, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: ScenesResponse = serde_json::from_value(body).unwrap();
    assert_eq!(list.active, "main");
    assert_eq!(list.scenes.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["main", "other"]);
    let ids: Vec<&str> = list.group_states.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(ids, ["left", "right"], "the active scene's groups, in its order");
    assert!(list.group_states.iter().all(|g| g.failed.is_none()), "{:?}", list.group_states);
}

#[tokio::test]
async fn status_names_the_active_scene() {
    let rig = rig().await;
    let (status, body) = send(&rig, "GET", routes::STATUS, None).await;
    assert_eq!(status, StatusCode::OK);
    let status: Status = serde_json::from_value(body).unwrap();
    let scene = status.active_scene.expect("the active scene is named");
    assert_eq!((scene.id.as_str(), scene.name.as_str()), ("main", "Main"));
    assert_eq!(status.active_effects, vec!["solid".to_string()], "one entry per distinct effect");
}

// ---------------------------------------------------------------- the whole-scene write

#[tokio::test]
async fn put_scene_applies_and_bumps_rev() {
    let rig = rig().await;
    shows(&rig.a, rgb(RED), "before").await;
    let mut scene = stored(&rig, "main").await;
    scene.groups[0].plays = Plays::Effect { effect: "solid".into(), props: color(GREEN) };
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(scene_of(answer).rev, 2);
    assert_eq!(stored(&rig, "main").await.rev, 2);
    let on_disk: ScenesFile =
        serde_json::from_slice(&std::fs::read(rig.home.path.join("scenes.json")).unwrap()).unwrap();
    assert_eq!(on_disk.scenes[0].rev, 2, "saved");
    shows(&rig.a, rgb(GREEN), "after").await;
    shows(&rig.b, rgb(BLUE), "the other group is untouched").await;
}

/// Review Focus 4: two editor windows write the same scene.
#[tokio::test]
async fn stale_rev_is_a_conflict() {
    let rig = rig().await;
    let before = rig.app.store.read().await.scenes.clone();
    let mut scene = stored(&rig, "main").await;
    scene.name = "Renamed".into();
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 99 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert_eq!(answer["error"], "conflict");
    let current = scene_of(answer["scene"].clone());
    assert_eq!((current.name.as_str(), current.rev), ("Main", 1), "the body carries the current scene");
    assert_eq!(rig.app.store.read().await.scenes, before, "the store is unchanged");
    assert!(!rig.home.path.join("scenes.json").exists(), "nothing was written");
}

#[tokio::test]
async fn a_light_in_two_groups_is_a_422() {
    let rig = rig().await;
    let before = stored(&rig, "main").await;
    let mut scene = before.clone();
    scene.groups[1].members.push("fake:a#z0".into());
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert_eq!(answer["error"], "validation");
    assert_eq!(stored(&rig, "main").await, before, "refused: the stored groups are as they were");
    let on_disk = rig.home.path.join("scenes.json");
    assert!(!on_disk.exists(), "refused: nothing was written");
}

#[tokio::test]
async fn two_html_groups_are_a_422() {
    let rig = rig().await;
    let mut scene = stored(&rig, "main").await;
    scene.groups[0].plays = Plays::Effect { effect: "html:one".into(), props: Props::new() };
    scene.groups[1].plays = Plays::Effect { effect: "html:two".into(), props: Props::new() };
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
}

// ---------------------------------------------------------------- create, delete, set

#[tokio::test]
async fn create_with_copy_of_copies_groups_under_new_ids() {
    let rig = rig().await;
    let body = serde_json::json!({ "name": "Evening", "copy_of": "main" });
    let (status, answer) = send(&rig, "POST", routes::SCENES, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let copy = scene_of(answer);
    let main = stored(&rig, "main").await;
    assert_eq!((copy.id.as_str(), copy.name.as_str(), copy.rev), ("evening", "Evening", 1));
    assert_eq!(copy.groups.len(), main.groups.len());
    for (new, old) in copy.groups.iter().zip(&main.groups) {
        assert_ne!(new.id, old.id, "a fresh group id");
        assert_eq!((&new.members, &new.plays, new.mode, new.brightness), (&old.members, &old.plays, old.mode, old.brightness));
    }
    assert_eq!(stored(&rig, "evening").await, copy, "stored");
    assert_eq!(rig.app.store.read().await.scenes.active, "main", "creating does not set it");

    // Without copy_of: one group of every light, set to Off.
    let (status, answer) = send(&rig, "POST", routes::SCENES, Some(serde_json::json!({ "name": "Blank" }))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let blank = scene_of(answer);
    assert_eq!(blank.groups.len(), 1);
    assert_eq!(blank.groups[0].members, vec!["fake:a#z0".to_string(), "fake:b#z0".to_string()]);
    assert_eq!(blank.groups[0].plays, Plays::Off);

    // A copy of a scene that is not there is a 404; an empty name is a 422.
    let (status, _) = send(&rig, "POST", routes::SCENES, Some(serde_json::json!({ "name": "X", "copy_of": "nope" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&rig, "POST", routes::SCENES, Some(serde_json::json!({ "name": "  " }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_last_scene_cannot_be_deleted() {
    let rig = rig().await;
    let (status, _) = send(&rig, "DELETE", &route(routes::SCENE, &[("id", "other")]), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, answer) = send(&rig, "DELETE", &route(routes::SCENE, &[("id", "main")]), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert_eq!(rig.app.store.read().await.scenes.scenes.len(), 1);
    let (status, _) = send(&rig, "DELETE", &route(routes::SCENE, &[("id", "nope")]), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_the_active_scene_activates_another() {
    let rig = rig().await;
    let (status, answer) = send(&rig, "DELETE", &route(routes::SCENE, &[("id", "main")]), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let list: ScenesResponse = serde_json::from_value(answer).unwrap();
    assert_eq!(list.active, "other");
    assert_eq!(rig.app.store.read().await.scenes.active, "other");
    shows(&rig.a, rgb(GREEN), "the new active scene plays").await;
    shows(&rig.b, rgb(GREEN), "the new active scene plays").await;
}

#[tokio::test]
async fn setting_a_scene_changes_what_the_fake_devices_show() {
    let rig = rig().await;
    shows(&rig.a, rgb(RED), "main").await;
    shows(&rig.b, rgb(BLUE), "main").await;
    let (status, answer) = send(&rig, "PUT", routes::SCENE_ACTIVE, Some(serde_json::json!({ "id": "other" }))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(serde_json::from_value::<ScenesResponse>(answer).unwrap().active, "other");
    shows(&rig.a, rgb(GREEN), "other").await;
    shows(&rig.b, rgb(GREEN), "other").await;
    assert_eq!(rig.app.store.read().await.scenes.active, "other");
    let (status, _) = send(&rig, "PUT", routes::SCENE_ACTIVE, Some(serde_json::json!({ "id": "nope" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------- takeovers

/// A draft plays on the groups of the active scene that play its effect.
#[tokio::test]
async fn a_draft_plays_on_the_groups_that_play_its_effect() {
    let rig = rig_with(draft_scenes_file()).await;
    shows(&rig.a, rgb(RED), "the saved effect plays").await;
    shows(&rig.b, rgb(BLUE), "the other group plays").await;
    // The draft is the saved effect's own source, so it plays where that effect plays.
    let body = serde_json::json!({ "html": MINE, "props": {} });
    let (status, answer) = send(&rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb(GREEN), "the draft plays on the group that plays its effect").await;
    shows(&rig.b, rgb(BLUE), "the other group keeps its own effect").await;
}

/// A draft of an effect no group plays goes on one temporary group of every light.
#[tokio::test]
async fn a_draft_for_an_unused_effect_plays_on_every_light() {
    let rig = rig_with(draft_scenes_file()).await;
    shows(&rig.a, rgb(RED), "before").await;
    shows(&rig.b, rgb(BLUE), "before").await;
    let body = serde_json::json!({ "html": UNUSED, "props": {} });
    let (status, answer) = send(&rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb(GREEN), "the draft plays on every light").await;
    shows(&rig.b, rgb(GREEN), "the draft plays on every light").await;
}

/// Stopping a draft puts the active scene back.
#[tokio::test]
async fn stopping_a_draft_restores_the_active_scene() {
    let rig = rig_with(draft_scenes_file()).await;
    shows(&rig.a, rgb(RED), "before").await;
    shows(&rig.b, rgb(BLUE), "before").await;
    let body = serde_json::json!({ "html": UNUSED, "props": {} });
    let (status, answer) = send(&rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb(GREEN), "the draft is on").await;
    let (status, answer) = send(&rig, "DELETE", routes::EFFECT_DRAFT, None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb(RED), "the active scene is back").await;
    shows(&rig.b, rgb(BLUE), "the active scene is back").await;
}

/// A stack a group of any scene plays is not deleted without `?force=true`, and the refusal
/// names the scenes it is in.
#[tokio::test]
async fn a_stack_played_by_a_group_in_an_inactive_scene_cannot_be_deleted() {
    let rig = rig_with(stack_scenes_file()).await;
    // The stack is saved first: the delete route answers 404 for a stack that is not there.
    let stack = serde_json::json!({
        "id": "stack:Glow",
        "layers": [{ "id": "l1", "name": "l1", "effect": "solid", "area": { "kind": "canvas" } }],
    });
    let (status, answer) = send(&rig, "PUT", "/api/v1/stacks/stack:Glow", Some(stack)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    // The scene that plays it is not the active one, and the guard still refuses.
    assert_eq!(rig.app.store.read().await.scenes.active, "main");
    let (status, answer) = send(&rig, "DELETE", "/api/v1/stacks/stack:Glow", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("Other"), "the refusal names the scene: {message}");
    assert!(rig.app.stacks.get("stack:Glow").is_some(), "the stack is still there");
    // With the force it goes, and the group that played it is dark.
    let (status, answer) = send(&rig, "DELETE", "/api/v1/stacks/stack:Glow?force=true", None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(rig.app.stacks.get("stack:Glow").is_none(), "the stack is gone");
}

/// A draft that moves from the takeover group to a group of the scene lands on that group: the
/// engine holds the takeover group, and an effect for a group it does not have is dropped.
#[tokio::test]
async fn a_draft_that_moves_to_a_scene_group_lands_on_that_group() {
    let rig = rig_with(draft_scenes_file()).await;
    // The first draft is of an effect no group plays: it lands on the takeover group, and
    // every light follows it.
    let body = serde_json::json!({ "html": UNUSED, "props": {} });
    let (status, answer) = send(&rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb(GREEN), "the draft is on every light").await;
    shows(&rig.b, rgb(GREEN), "the draft is on every light").await;

    // The next draft is of the effect the left group plays: it lands on that group, and the
    // other group keeps its own effect.
    let body = serde_json::json!({ "html": MINE, "props": {} });
    let (status, answer) = send(&rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb(GREEN), "the draft plays on the group that plays its effect").await;
    shows(&rig.b, rgb(BLUE), "the other group keeps its own effect").await;
}

/// Identify is a sink-level override: it does not take the lights away from a draft, and
/// stopping it does not put the scene back over the draft.
#[tokio::test]
async fn identify_during_a_draft_leaves_the_draft_on_the_lights() {
    let rig = rig_with(draft_scenes_file()).await;
    let body = serde_json::json!({ "html": UNUSED, "props": {} });
    let (status, answer) = send(&rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb(GREEN), "the draft is on the lights").await;

    // The identify flashes one light: it overrides what the engine sends.
    let (status, answer) = send(
        &rig,
        "POST",
        routes::LIGHTS_FLASH,
        Some(serde_json::json!({ "lights": ["fake:a#z0"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    // The identify stops: the draft is still the thing on the lights.
    let (status, answer) = send(
        &rig,
        "POST",
        routes::LIGHTS_FLASH,
        Some(serde_json::json!({ "lights": ["fake:a#z0"], "stop": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(
        rig.app.draft.snapshot().is_some(),
        "the draft is still the draft: the identify did not end it"
    );
    assert!(
        !rig.app.scene.held().takeover.is_empty(),
        "the draft still holds the lights: the identify did not put the scene back over it"
    );
}

/// A stack draft's first instance resolves its areas in the group it lands on.
#[tokio::test]
async fn a_stack_draft_resolves_its_areas_in_the_group_it_lands_on() {
    let rig = rig_with(draft_scenes_file()).await;
    // A stack with a layer over the device of the left group.
    let stack = serde_json::json!({
        "id": "stack:Area",
        "layers": [{
            "id": "l1",
            "name": "l1",
            "effect": "solid",
            "area": { "kind": "device", "device": "fake:a" },
        }],
    });
    let (status, answer) = send(&rig, "PUT", "/api/v1/stacks/stack:Area", Some(stack)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    // The draft of that stack: no group plays it, so it lands on the takeover group of every
    // light, and its areas resolve in that group's plan.
    let body = serde_json::json!({
        "stack": {
            "id": "stack:Area",
            "layers": [{
                "id": "l1",
                "name": "l1",
                "effect": "solid",
                "area": { "kind": "device", "device": "fake:a" },
            }],
        },
        "epoch": "0123456789abcdef",
        "seq": 1,
        "props": {},
    });
    let (status, answer) = send(&rig, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        rig.app.stacks.running_group().as_deref(),
        Some(canvas_daemon::scenes::TAKEOVER_GROUP),
        "the draft's stack resolves in the takeover group's plan"
    );
}

/// A stack with an HTML layer cannot be saved while another group of the scene plays HTML: the
/// scene would run more HTML groups than it may.
#[tokio::test]
async fn a_stack_with_an_html_layer_is_refused_when_the_scene_is_at_the_limit() {
    let mut scenes = draft_scenes_file();
    // The left group plays a stack, the right one an HTML effect: the scene is at the limit.
    scenes.scenes[0].groups[0].plays =
        Plays::Effect { effect: "stack:Html".into(), props: Props::new() };
    scenes.scenes[0].groups[1].plays =
        Plays::Effect { effect: "html:mine".into(), props: Props::new() };
    let rig = rig_with(scenes).await;
    // The stack is saved first: it has no HTML layer yet.
    let stack = serde_json::json!({
        "id": "stack:Html",
        "layers": [{ "id": "l1", "name": "l1", "effect": "solid" }],
    });
    let (status, answer) = send(&rig, "PUT", "/api/v1/stacks/stack:Html", Some(stack)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    // Now it has an HTML layer: the scene would run two HTML groups.
    let stack = serde_json::json!({
        "id": "stack:Html",
        "layers": [{ "id": "l1", "name": "l1", "effect": "html:mine" }],
    });
    let (status, answer) = send(&rig, "PUT", "/api/v1/stacks/stack:Html", Some(stack)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    let message = answer["message"].as_str().unwrap_or_default();
    assert!(message.contains("HTML"), "the refusal says why: {message}");
}

/// The first run sizes the JARGB headers: the zones it sized are lights now, and they join the
/// first-run scene, so they are lit and not dark.
#[tokio::test]
async fn the_sized_jargb_headers_join_the_first_run_scene() {
    let home = TempHome::new();
    // The MSI header, with a zone that has no LEDs yet.
    let mut header = device("fake:MSI B850 Mystic Light");
    header.name = "MSI B850 Mystic Light".into();
    header.zones[0].name = "JARGB1".into();
    header.zones[0].led_count = 0;
    header.zones[0].min_leds = 0;
    header.zones[0].max_leds = 0;

    let mut store = canvas_daemon::storage::Store::at(&home.path);
    // The first-run scene: the default one, with no light yet: the header had no LEDs when the
    // scene was made.
    store.lights = canvas_proto::scene::LightsFile {
        schema_version: LIGHTS_SCHEMA,
        lights: Vec::new(),
    };
    store.scenes = canvas_daemon::scenes::default_scenes(&store.lights);
    let engine: Arc<dyn Engine> = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    });
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    // The adapter the daemon holds has the header in it.
    let adapter = FakeAdapter::default();
    *adapter.devices.lock().unwrap() = vec![header.clone()];
    let mut app = App::new(
        store,
        engine,
        Box::new(adapter),
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
        tx,
    );
    app.user_effects = Arc::new(canvas_daemon::user_effects::UserEffects::with_stock(
        fake_builder(),
        &[("mine", MINE)],
    ));
    let app = Arc::new(app);
    *app.devices.write().await = vec![header.clone()];

    // The first run sizes the header: the zone gets LEDs, and the light joins the scene.
    let counts = canvas_daemon::first_run::JargbCounts {
        source: canvas_daemon::first_run::CountSource::Unknown,
        counts: vec![12],
    };
    let sized = canvas_daemon::first_run::size_headers(&app, &[header.clone()], &counts).await;
    assert_eq!(sized[0].zones[0].led_count, 12, "the header was sized");

    let store = app.store.read().await;
    assert_eq!(
        store.lights.lights.len(),
        1,
        "the sized header is a light now: {:?}",
        store.lights.lights
    );
    let scene = canvas_daemon::scenes::active_scene(&store.scenes);
    assert_eq!(
        scene.groups[0].members,
        vec![store.lights.lights[0].id.clone()],
        "and it joins the first run's group"
    );
}

// ---------------------------------------------------------------- one group

#[tokio::test]
async fn patch_group_props_changes_one_group_only() {
    let rig = rig().await;
    shows(&rig.a, rgb(RED), "before").await;
    let path = route(routes::GROUP_PROPS, &[("id", "main"), ("group", "left")]);
    let body = serde_json::to_value(color(GREEN)).unwrap();
    let (status, answer) = send(&rig, "PATCH", &path, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let scene = scene_of(answer);
    assert_eq!(scene.rev, 2, "saved: rev + 1");
    assert_eq!(scene.groups[0].plays, Plays::Effect { effect: "solid".into(), props: color(GREEN) });
    assert_eq!(scene.groups[1], stored(&rig, "main").await.groups[1], "the other group is as it was");
    shows(&rig.a, rgb(GREEN), "the patched group").await;
    shows(&rig.b, rgb(BLUE), "the other group").await;
    let on_disk: ScenesFile =
        serde_json::from_slice(&std::fs::read(rig.home.path.join("scenes.json")).unwrap()).unwrap();
    assert_eq!(on_disk.scenes[0].rev, 2);

    let (status, _) = send(&rig, "PATCH", &route(routes::GROUP_PROPS, &[("id", "main"), ("group", "zz")]), Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn put_group_brightness_saves_bumps_rev_and_answers_the_scene() {
    let rig = rig().await;
    let path = route(routes::GROUP_BRIGHTNESS, &[("id", "main"), ("group", "left")]);
    let (status, answer) = send(&rig, "PUT", &path, Some(serde_json::json!({ "brightness": 50 }))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let scene = scene_of(answer);
    assert_eq!((scene.rev, scene.groups[0].brightness, scene.groups[1].brightness), (2, 50, 100));
    // 200 at half brightness is 100, give or take a rounding step.
    let start = Instant::now();
    loop {
        let r = rig.a.last().map(|f| f[0].r);
        if r.is_some_and(|r| (99..=101).contains(&r)) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "group A never dimmed: {r:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    shows(&rig.b, rgb(BLUE), "the other group").await;
    let (status, _) = send(&rig, "PUT", &path, Some(serde_json::json!({ "brightness": 101 }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(stored(&rig, "main").await.rev, 2, "a refused value changes nothing");
}

// ---------------------------------------------------------------- playing an effect

#[tokio::test]
async fn activate_with_no_groups_plays_on_every_group() {
    let rig = rig().await;
    let body = serde_json::json!({ "props": serde_json::to_value(color((0, 150, 150))).unwrap() });
    let (status, answer) = send(&rig, "POST", &route(routes::EFFECT_ACTIVATE, &[("id", "solid")]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb((0, 150, 150)), "A").await;
    shows(&rig.b, rgb((0, 150, 150)), "B").await;
    let scene = stored(&rig, "main").await;
    assert_eq!(scene.rev, 2);
    assert!(scene.groups.iter().all(|g| matches!(&g.plays, Plays::Effect { effect, props } if effect == "solid" && *props == color((0, 150, 150)))));
}

#[tokio::test]
async fn activate_with_a_group_plays_only_there() {
    let rig = rig().await;
    let body = serde_json::json!({
        "props": serde_json::to_value(color(GREEN)).unwrap(),
        "groups": ["RIGHT"],
    });
    let (status, answer) = send(&rig, "POST", &route(routes::EFFECT_ACTIVATE, &[("id", "solid")]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.b, rgb(GREEN), "the named group").await;
    shows(&rig.a, rgb(RED), "the other group").await;
    let scene = stored(&rig, "main").await;
    assert_eq!(scene.groups[0].plays, Plays::Effect { effect: "solid".into(), props: color(RED) });

    let body = serde_json::json!({ "groups": ["nobody"] });
    let (status, _) = send(&rig, "POST", &route(routes::EFFECT_ACTIVATE, &[("id", "solid")]), Some(body)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a group name the scene does not have");
}

#[tokio::test]
async fn effect_defaults_do_not_touch_a_playing_group() {
    let rig = rig().await;
    let path = route(routes::EFFECT_DEFAULTS, &[("id", "solid")]);
    let (status, answer) = send(&rig, "PATCH", &path, Some(serde_json::to_value(color(GREEN)).unwrap())).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let saved = rig.app.store.read().await.effects.saved.get("solid").cloned().expect("starting values are kept");
    assert_eq!(saved.get("color"), Some(&PropValue::Color(rgb(GREEN))));
    let on_disk = std::fs::read_to_string(rig.home.path.join("effects.json")).unwrap();
    assert!(on_disk.contains("solid"), "{on_disk}");
    let main = stored(&rig, "main").await;
    assert_eq!(main.rev, 1, "no scene changed");
    assert_eq!(main.groups[0].plays, Plays::Effect { effect: "solid".into(), props: color(RED) });
    shows(&rig.a, rgb(RED), "the playing group keeps its own values").await;
    let (status, _) = send(&rig, "PATCH", &route(routes::EFFECT_DEFAULTS, &[("id", "no-such-effect")]), Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------- one group fails, the rest run

/// Review Focus 3: a group names an effect that no longer exists.
#[tokio::test]
async fn missing_effect_darkens_only_its_group() {
    let mut scenes = scenes_file();
    scenes.scenes[0].groups[0].plays = Plays::Effect { effect: "html:deleted".into(), props: Props::new() };
    let rig = rig_with(scenes).await;
    shows(&rig.a, Rgb8::BLACK, "the group with no effect is dark").await;
    shows(&rig.b, rgb(BLUE), "the other group runs").await;
    let (status, body) = send(&rig, "GET", routes::SCENES, None).await;
    assert_eq!(status, StatusCode::OK);
    let list: ScenesResponse = serde_json::from_value(body).unwrap();
    let left = list.group_states.iter().find(|g| g.id == "left").unwrap();
    let right = list.group_states.iter().find(|g| g.id == "right").unwrap();
    assert!(left.failed.as_deref().is_some_and(|why| why.contains("deleted")), "names the cause: {left:?}");
    assert_eq!(right.failed, None);
}

/// Task 3's planner skips a member that names no light; the daemon hands it through.
#[tokio::test]
async fn a_member_naming_no_light_is_skipped_and_the_rest_light() {
    let mut scenes = scenes_file();
    scenes.scenes[0].groups[0].members = vec!["fake:gone#z9".into(), "fake:a#z0".into()];
    let rig = rig_with(scenes).await;
    shows(&rig.a, rgb(RED), "the real member").await;
    shows(&rig.b, rgb(BLUE), "the other group").await;
}

// ---------------------------------------------------------------- flash

#[tokio::test]
async fn flash_lights_the_named_lights_and_blacks_the_rest_then_restores() {
    let rig = rig().await;
    let (tx_a, mut rx_a) = tokio::sync::mpsc::channel(8);
    let (tx_b, mut rx_b) = tokio::sync::mpsc::channel(8);
    rig.app.identify.register(DeviceId(A.into()), tx_a);
    rig.app.identify.register(DeviceId(B.into()), tx_b);

    let body = serde_json::json!({ "lights": ["fake:a#z0"], "stop": false });
    let (status, answer) = send(&rig, "POST", routes::LIGHTS_FLASH, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(rig.app.identify.is_active(), "the override holds the lights");
    let a = rx_a.try_recv().expect("A got a frame at once");
    let b = rx_b.try_recv().expect("B got a frame at once");
    assert_eq!(a, vec![Rgb8::WHITE; LEDS as usize], "the named light is white");
    assert_eq!(b, vec![Rgb8::BLACK; LEDS as usize], "everything else is black");

    // Several lights at once.
    let body = serde_json::json!({ "lights": ["fake:a#z0", "fake:b#z0"], "stop": false });
    let (status, _) = send(&rig, "POST", routes::LIGHTS_FLASH, Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rx_a.try_recv().unwrap(), vec![Rgb8::WHITE; LEDS as usize]);
    assert_eq!(rx_b.try_recv().unwrap(), vec![Rgb8::WHITE; LEDS as usize]);

    // An id that names no light is a 404.
    let body = serde_json::json!({ "lights": ["fake:nope#z0"] });
    let (status, _) = send(&rig, "POST", routes::LIGHTS_FLASH, Some(body)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Stop restores the normal frames: the device sinks show the scene again, in frames that came
    // after the stop (the flag alone proves nothing about the lights).
    let (a_seen, b_seen) = (rig.a.seen(), rig.b.seen());
    let (status, _) = send(&rig, "POST", routes::LIGHTS_FLASH, Some(serde_json::json!({ "stop": true }))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!rig.app.identify.is_active(), "the override is released");
    shows_after(&rig.a, a_seen, rgb(RED), "A after the flash").await;
    shows_after(&rig.b, b_seen, rgb(BLUE), "B after the flash").await;
    // Silence past the watchdog restores too.

    let body = serde_json::json!({ "lights": ["fake:b#z0"] });
    send(&rig, "POST", routes::LIGHTS_FLASH, Some(body)).await;
    assert!(rig.app.identify.is_active());
    let devices = rig.app.devices.read().await.clone();
    let later = Instant::now() + canvas_daemon::identify::WATCHDOG;
    assert_eq!(rig.app.identify.tick(later, &devices), canvas_daemon::identify::Tick::Expired);
    assert!(!rig.app.identify.is_active(), "the watchdog ends it");
}

// ---------------------------------------------------------------- the daemon follows the devices

#[tokio::test]
async fn a_device_list_puts_the_daemon_in_scene_mode() {
    let rig = rig().await;
    assert!(rig.app.scene.is_on(), "apply_active puts the engine in scene mode");
    let app = Arc::clone(&rig.app);
    // The list arrives again (a rescan): the scene is applied to it, and the legacy layout path
    // does not take the engine back out of scene mode.
    canvas_daemon::device_sync::apply_devices(&app, vec![device(A), device(B)]).await;
    assert_eq!(
        rig.app.engine.group_states().iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
        ["left", "right"],
        "the engine still runs the scene"
    );
    // `apply_devices` gave each device its own writer sink; put the recorders back on top.
    let (a, b) = (Arc::new(Rec::default()), Arc::new(Rec::default()));
    rig.app.engine.add_sink(DeviceId(A.into()), a.clone());
    rig.app.engine.add_sink(DeviceId(B.into()), b.clone());
    shows(&a, rgb(RED), "A").await;
    shows(&b, rgb(BLUE), "B").await;
}

// ---------------------------------------------------------------- the migration wait (Task 5 left it UNKNOWN)

fn placed(id: &str, x: f32) -> canvas_proto::layout::PlacedZone {
    use canvas_proto::layout::{PlacedZone, ZoneShape};
    PlacedZone {
        device_id: DeviceId(id.to_string()),
        zone_index: 0,
        x,
        y: 0.0,
        width: 160.0,
        height: 200.0,
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

/// `apply_devices` with a device the layouts place missing from the list: the migration waits, the
/// timer fires 60 s after the first list on tokio's paused clock, and `lights.json` and
/// `scenes.json` are written from the layouts and the devices present.
#[tokio::test(start_paused = true)]
async fn the_wait_timer_writes_the_scenes_when_a_placed_device_never_arrives() {
    let (app, _router, home, layouts) = held_app().await;

    canvas_daemon::device_sync::apply_devices(&app, vec![device(A)]).await;
    assert!(app.store.read().await.scenes_pending.is_some(), "fake:gone is placed and absent: it waits");
    assert!(!home.path.join("scenes.json").exists() && !home.path.join("lights.json").exists());

    // The wait is 60 s (SCENES_DEVICE_WAIT) and the timer adds 50 ms: at 59 s the scenes are still
    // held, at 61 s they are written, with the devices present.
    assert_eq!(canvas_daemon::storage::SCENES_DEVICE_WAIT, Duration::from_secs(60));
    tokio::time::sleep(Duration::from_secs(59)).await;
    assert!(app.store.read().await.scenes_pending.is_some(), "59 s in, still waiting");
    assert!(!home.path.join("scenes.json").exists());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let store = app.store.read().await;
    assert!(store.scenes_pending.is_none(), "the timer wrote the scenes");
    assert!(home.path.join("scenes.json").exists() && home.path.join("lights.json").exists());
    let ids: Vec<&str> = store.lights.lights.iter().map(|l| l.id.as_str()).collect();
    assert!(ids.contains(&"fake:a#z0") && ids.contains(&"fake:gone#z0"), "{ids:?}");
    let scene = canvas_daemon::scenes::active_scene(&store.scenes);
    assert_eq!(scene.name, "Desk");
    assert!(layouts_file_untouched(&home, &layouts));
}

fn layouts_file_untouched(home: &TempHome, layouts: &canvas_proto::config::LayoutsFile) -> bool {
    std::fs::read(home.path.join("layouts.json")).unwrap() == serde_json::to_vec(layouts).unwrap()
}

// ---------------------------------------------------------------- the lights API follows the engine

/// `PATCH /lights/{id}` (the lights builder's route) re-applies the scene: reversing the light
/// that plays a static left-to-right gradient puts the blue end first.
#[tokio::test]
async fn light_patch_renames_and_sets_ring_and_the_engine_follows() {
    let mut props = Props::new();
    props.insert("color1".into(), PropValue::Color(Rgb8::new(255, 0, 0)));
    props.insert("color2".into(), PropValue::Color(Rgb8::new(0, 0, 255)));
    props.insert("angle".into(), PropValue::Angle(0.0));
    props.insert("scroll_speed".into(), PropValue::Number(0.0));
    let mut scenes = scenes_file();
    scenes.scenes[0].groups = vec![group("solo", &["fake:a#z0"], "gradient", props)];
    let rig = rig_with(scenes).await;

    let start = Instant::now();
    let first = |rig: &Rig| rig.a.last().filter(|f| f.len() == LEDS as usize).map(|f| (f[0], f[f.len() - 1]));
    loop {
        if first(&rig).is_some_and(|(head, tail)| head.r > head.b && tail.b > tail.r) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "the gradient never ran red to blue: {:?}", first(&rig));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let path = route(routes::LIGHT, &[("id", "fake:a%23z0")]);
    let body = serde_json::json!({ "name": "Desk strip", "reversed": true });
    let (status, answer) = send(&rig, "PATCH", &path, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["label"], "Desk strip");
    let stored = rig.app.store.read().await.lights.lights[0].clone();
    assert_eq!((stored.name.as_deref(), stored.reversed), (Some("Desk strip"), true));
    let start = Instant::now();
    loop {
        if first(&rig).is_some_and(|(head, tail)| head.b > head.r && tail.r > tail.b) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "the engine never followed the patch: {:?}", first(&rig));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // A ring reads the gradient round a circle. LED j sits 30 degrees * j clockwise from the top,
    // and the gradient depends on x alone, so the 12 LEDs fall into 7 colours by their x: the two
    // that share a side, top and bottom, the left and right extremes. The classes below are those
    // from the left (LED 9) to the right (LED 3); each class is one colour, and the colour runs
    // from red to blue across them.
    let (status, _) = send(&rig, "PATCH", &path, Some(serde_json::json!({ "arrangement": "ring", "reversed": false }))).await;
    assert_eq!(status, StatusCode::OK);
    let start = Instant::now();
    let ring = loop {
        // On a strip LED 0 and LED 6 differ (opposite ends of the gradient); on a ring they share an x.
        if let Some(f) = rig.a.last().filter(|f| f.len() == LEDS as usize && f[0] == f[6] && f[9] != f[3]) {
            break f;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "the engine never read the light as a ring: {:?}", rig.a.last());
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let classes: [&[usize]; 7] = [&[9], &[8, 10], &[7, 11], &[6, 0], &[5, 1], &[4, 2], &[3]];
    for class in classes {
        assert!(class.iter().all(|&j| ring[j] == ring[class[0]]), "LEDs {class:?} share an x, so one colour: {ring:?}");
    }
    for pair in classes.windows(2) {
        let (left, right) = (ring[pair[0][0]], ring[pair[1][0]]);
        assert!(left.r > right.r && left.b < right.b, "from LED {:?} to LED {:?} the gradient runs red to blue: {left:?} then {right:?}", pair[0], pair[1]);
    }
}

// ---------------------------------------------------------------- the light feed (PREVIEW_LIGHTS)

/// A WebSocket client over an in-memory connection: the handshake by hand and the server's frames
/// (unmasked) read back. The daemon has no WebSocket client crate to lean on, and these few bytes
/// of framing are all the feed uses.
#[derive(Debug)]
struct Ws {
    stream: tokio::io::DuplexStream,
    buf: Vec<u8>,
}

/// What a refused handshake answered.
#[derive(Debug)]
struct Refused {
    status: String,
    body: String,
}

impl Ws {
    /// The handshake. `headers` are extra lines (`Origin: ...`); `host` is the `Host` header.
    async fn open_with(
        stream: tokio::io::DuplexStream,
        target: &str,
        host: &str,
        headers: &[&str],
    ) -> Result<Ws, Refused> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = stream;
        let mut request = format!(
            "GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
             Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
        );
        for line in headers {
            request.push_str(line);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await.expect("the handshake is written");
        let mut buf = Vec::new();
        let head_end = loop {
            if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
            let mut chunk = [0u8; 512];
            let n = stream.read(&mut chunk).await.expect("the handshake answer is read");
            assert!(n > 0, "the server closed before it answered");
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let status = head.lines().next().unwrap_or("").to_string();
        if status.contains(" 101 ") {
            buf.drain(..head_end);
            return Ok(Ws { stream, buf });
        }
        let length = head
            .lines()
            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
            .unwrap_or(0);
        while buf.len() < head_end + length {
            let mut chunk = [0u8; 512];
            let n = stream.read(&mut chunk).await.expect("the refusal body is read");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Err(Refused { status, body: String::from_utf8_lossy(&buf[head_end..]).to_string() })
    }

    async fn open(server: &common::MemoryServer, query: &str) -> Result<Ws, Refused> {
        Self::open_with(server.connect(), &format!("{}{query}", routes::PREVIEW_LIGHTS), "127.0.0.1:9430", &[]).await
    }

    /// The next frame: `(opcode, payload)`. 1 is text, 2 is binary.
    async fn frame(&mut self) -> (u8, Vec<u8>) {
        use tokio::io::AsyncReadExt;
        loop {
            if self.buf.len() >= 2 {
                let opcode = self.buf[0] & 0x0f;
                let (len, head) = match self.buf[1] & 0x7f {
                    126 if self.buf.len() >= 4 => (u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize, 4),
                    127 if self.buf.len() >= 10 => {
                        (u64::from_be_bytes(self.buf[2..10].try_into().unwrap()) as usize, 10)
                    }
                    n @ 0..=125 => (n as usize, 2),
                    _ => (usize::MAX, 0),
                };
                if head > 0 && self.buf.len() >= head + len {
                    let payload = self.buf[head..head + len].to_vec();
                    self.buf.drain(..head + len);
                    return (opcode, payload);
                }
            }
            let mut chunk = [0u8; 4096];
            let n = tokio::time::timeout(Duration::from_secs(20), self.stream.read(&mut chunk))
                .await
                .expect("a frame came within 20 s")
                .expect("the frame is read");
            assert!(n > 0, "the server closed the socket");
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// The next text frame (binary frames are skipped), as JSON.
    async fn next_index(&mut self) -> serde_json::Value {
        loop {
            let (opcode, payload) = self.frame().await;
            if opcode == 1 {
                return serde_json::from_slice(&payload).expect("the text frame is the index JSON");
            }
        }
    }
}

/// A binary light frame: `u16 index, u16 count, count RGB triples`, repeated.
fn decode_lights(payload: &[u8]) -> Vec<(u16, Vec<Rgb8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < payload.len() {
        let index = u16::from_le_bytes([payload[at], payload[at + 1]]);
        let count = u16::from_le_bytes([payload[at + 2], payload[at + 3]]) as usize;
        at += 4;
        let colors = payload[at..at + count * 3].chunks(3).map(|c| Rgb8::new(c[0], c[1], c[2])).collect();
        at += count * 3;
        out.push((index, colors));
    }
    out
}

#[tokio::test]
async fn the_light_feed_opens_with_only_a_query_token_and_streams_the_lights() {
    let rig = rig().await;
    let server = common::serve_in_memory(rig.router.clone());
    // A browser sends the token in the query and nothing else.
    let mut ws = Ws::open(&server, &format!("?token={TOKEN}")).await.expect("the socket opens");
    let (opcode, first) = ws.frame().await;
    assert_eq!(opcode, 1, "the first message is text");
    let index: serde_json::Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(index, serde_json::json!({ "0": "fake:a#z0", "1": "fake:b#z0" }));
    // Then binary frames: light A (red) is index 0 and light B (blue) is index 1, 12 LEDs each.
    let start = Instant::now();
    loop {
        let (opcode, payload) = ws.frame().await;
        if opcode != 2 {
            continue;
        }
        let lights = decode_lights(&payload);
        let red = vec![rgb(RED); LEDS as usize];
        let blue = vec![rgb(BLUE); LEDS as usize];
        if lights.iter().any(|(i, c)| *i == 0 && *c == red) && lights.iter().any(|(i, c)| *i == 1 && *c == blue) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "no frame showed A red and B blue; last {lights:?}");
    }
}

#[tokio::test]
async fn the_light_feed_without_a_token_or_with_a_bad_one_is_a_401_from_the_handler() {
    let rig = rig().await;
    let server = common::serve_in_memory(rig.router.clone());
    let none = Ws::open(&server, "").await.expect_err("no token is refused");
    assert!(none.status.contains("401"), "{none:?}");
    assert!(none.body.contains("the light preview needs a token"), "the handler answered, not the guard: {none:?}");
    let bad = Ws::open(&server, "?token=not-the-token").await.expect_err("a bad token is refused");
    assert!(bad.status.contains("401"), "{bad:?}");
    assert!(bad.body.contains("the API token is not valid"), "the handler answered, not the guard: {bad:?}");
    assert_eq!(rig.app.scene.feed.watchers(), 0, "a refused handshake joins nothing");
}

#[tokio::test]
async fn the_light_feed_still_checks_the_origin_and_the_host() {
    let rig = rig().await;
    let server = common::serve_in_memory(rig.router.clone());
    let target = format!("{}?token={TOKEN}", routes::PREVIEW_LIGHTS);
    let origin = Ws::open_with(server.connect(), &target, "127.0.0.1:9430", &["Origin: http://evil.example"])
        .await
        .expect_err("a foreign origin is refused");
    assert!(origin.status.contains("403") && origin.body.contains("forbidden_origin"), "{origin:?}");
    let host = Ws::open_with(server.connect(), &target, "evil.example", &[]).await.expect_err("a foreign host is refused");
    assert!(host.status.contains("403") && host.body.contains("bad_host"), "{host:?}");
}

#[tokio::test]
async fn the_light_feed_sends_the_index_again_when_the_scene_or_the_lights_change() {
    let rig = rig().await;
    let server = common::serve_in_memory(rig.router.clone());
    let mut ws = Ws::open(&server, &format!("?token={TOKEN}")).await.expect("the socket opens");
    assert_eq!(ws.next_index().await.as_object().unwrap().len(), 2);

    // A scene change sends it again.
    let (status, _) = send(&rig, "PUT", routes::SCENE_ACTIVE, Some(serde_json::json!({ "id": "other" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ws.next_index().await.as_object().unwrap().len(), 2, "after a scene change");

    // A light that appears is in the next index.
    {
        let mut store = rig.app.store.write().await;
        let mut third = light(A);
        third.id = "fake:c#z0".into();
        store.lights.lights.push(third);
    }
    rig.app.scene.notify();
    let index = ws.next_index().await;
    assert_eq!(index["2"], "fake:c#z0", "after a light change: {index}");
}

#[tokio::test]
async fn the_light_feed_takes_a_few_windows_and_answers_503_past_that() {
    let rig = rig().await;
    let server = common::serve_in_memory(rig.router.clone());
    let cap = canvas_daemon::scenes::LIGHT_FEED_MAX_WATCHERS;
    let mut open = Vec::new();
    for _ in 0..cap {
        open.push(Ws::open(&server, &format!("?token={TOKEN}")).await.expect("a window within the cap opens"));
    }
    assert_eq!(rig.app.scene.feed.watchers(), cap);
    let over = Ws::open(&server, &format!("?token={TOKEN}")).await.expect_err("one past the cap is refused");
    assert!(over.status.contains("503"), "{over:?}");
    assert_eq!(rig.app.scene.feed.watchers(), cap, "the refused window took no slot");
    // A window that leaves frees its slot.
    drop(open.pop());
    let start = Instant::now();
    loop {
        if Ws::open(&server, &format!("?token={TOKEN}")).await.is_ok() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "the freed slot never came back");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_window_that_stops_reading_loses_its_slot() {
    let rig = rig().await;
    rig.app.scene.feed.set_send_timeout(Duration::from_millis(200));
    let server = common::serve_in_memory(rig.router.clone());
    // A pipe of 256 bytes each way: the handshake answer and the index fit, the frames do not.
    let stuck = Ws::open_with(
        server.connect_with_buffer(256),
        &format!("{}?token={TOKEN}", routes::PREVIEW_LIGHTS),
        "127.0.0.1:9430",
        &[],
    )
    .await
    .expect("the socket opens");
    assert_eq!(rig.app.scene.feed.watchers(), 1);
    // `stuck` stays open and unread. The server gives up on it after the send timeout.
    let start = Instant::now();
    while rig.app.scene.feed.watchers() != 0 {
        assert!(start.elapsed() < Duration::from_secs(20), "the stuck window still holds its slot");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(stuck);
}

#[tokio::test]
async fn a_watcher_whose_task_dies_gives_its_slot_back() {
    let rig = rig().await;
    let app = Arc::clone(&rig.app);
    let task = tokio::spawn(async move {
        let _watch = canvas_daemon::scenes::LightWatch::join(&app).expect("a slot is free");
        assert_eq!(app.scene.feed.watchers(), 1);
        panic!("the feed task dies");
    });
    assert!(task.await.is_err(), "the task panicked");
    assert_eq!(rig.app.scene.feed.watchers(), 0, "the guard gave the slot back");
}

// ---------------------------------------------------------------- what a save hands the engine (fix pass)

/// The scenes held in memory: a `layouts.json` that places a device the list never brings, so the
/// migration waits and the scenes live only in memory. The engine is a fake; the daemon is in scene
/// mode, as start-up leaves it.
async fn held_app() -> (Arc<App>, axum::Router, TempHome, canvas_proto::config::LayoutsFile) {
    use canvas_proto::layout::{CaseForm, Layout, RenderScale, SamplingMode};
    let home = TempHome::new();
    let layouts = canvas_proto::config::LayoutsFile {
        schema_version: canvas_proto::config::LAYOUTS_SCHEMA,
        active: "desk".into(),
        layouts: vec![Layout {
            id: "desk".into(),
            name: "Desk".into(),
            render_scale: RenderScale::P100,
            sampling: SamplingMode::AreaAverage,
            zones: vec![placed(A, 0.0), placed("fake:gone", 160.0)],
            effect: None,
            case_form: CaseForm::None,
            landmarks: Vec::new(),
        }],
    };
    std::fs::write(home.path.join("layouts.json"), serde_json::to_vec(&layouts).unwrap()).unwrap();
    let mut store = canvas_daemon::storage::Store::load_from(&home.path);
    assert!(store.scenes_pending.is_some(), "no scenes.json: the migration is held");
    store.config.first_run_done = true;
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        Arc::new(common::FakeEngine::default()),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
        tx,
    ));
    let router = canvas_daemon::api::router(app.clone());
    (app, router, home, layouts)
}

/// S1: a props or brightness change is already what the engine runs, so the next device list (the
/// supervisor repeats it every few seconds) rebuilds nothing and no effect restarts.
#[tokio::test]
async fn a_group_change_is_not_rebuilt_by_the_next_device_list() {
    let rig = rig().await;
    let built = rig.app.scene.builds();
    assert_eq!(built, 2, "start-up built the two groups once");

    let path = route(routes::GROUP_PROPS, &[("id", "main"), ("group", "left")]);
    let (status, _) = send(&rig, "PATCH", &path, Some(serde_json::to_value(color(GREEN)).unwrap())).await;
    assert_eq!(status, StatusCode::OK);
    canvas_daemon::scenes::apply_for_devices(&rig.app).await;
    assert_eq!(rig.app.scene.builds(), built, "a props patch, then the same device list, built nothing");

    let path = route(routes::GROUP_BRIGHTNESS, &[("id", "main"), ("group", "right")]);
    let (status, _) = send(&rig, "PUT", &path, Some(serde_json::json!({ "brightness": 40 }))).await;
    assert_eq!(status, StatusCode::OK);
    canvas_daemon::scenes::apply_for_devices(&rig.app).await;
    assert_eq!(rig.app.scene.builds(), built, "a brightness change, then the same device list, built nothing");
    shows(&rig.a, rgb(GREEN), "the patched group still plays").await;
}

/// Review 327 verify: a device list that arrives while a group change holds the scene lock is never
/// recorded as applied by that change, so the `apply_for_devices` that follows still takes it.
#[tokio::test]
async fn a_device_list_that_lands_during_a_group_change_is_still_applied() {
    let rig = rig().await;
    let built = rig.app.scene.builds();

    // The supervisor has written the new list (device A grew) but not yet applied it.
    let mut grown = device(A);
    grown.zones[0].led_count = LEDS * 2;
    *rig.app.devices.write().await = vec![grown, device(B)];

    let path = route(routes::GROUP_PROPS, &[("id", "main"), ("group", "left")]);
    let (status, _) = send(&rig, "PATCH", &path, Some(serde_json::to_value(color(GREEN)).unwrap())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rig.app.scene.builds(), built, "the props patch alone rebuilds nothing");

    canvas_daemon::scenes::apply_for_devices(&rig.app).await;
    assert!(rig.app.scene.builds() > built, "the new device list is applied, not skipped as a repeat");
}

/// A save rebuilds only what it changed: values and brightness keep the running effect, one
/// group's new effect builds one effect, a changed membership takes the whole scene again.
#[tokio::test]
async fn put_scene_rebuilds_only_what_changed() {
    let rig = rig().await;
    let built = rig.app.scene.builds();
    let path = route(routes::SCENE, &[("id", "main")]);

    let mut scene = stored(&rig, "main").await;
    scene.groups[0].plays = Plays::Effect { effect: "solid".into(), props: color(GREEN) };
    scene.groups[1].brightness = 60;
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, _) = send(&rig, "PUT", &path, Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rig.app.scene.builds(), built, "new values and a new brightness rebuild nothing");

    let mut scene = stored(&rig, "main").await;
    scene.groups[0].plays = Plays::Effect { effect: "gradient".into(), props: Props::new() };
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 2 }).unwrap();
    let (status, _) = send(&rig, "PUT", &path, Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rig.app.scene.builds(), built + 1, "one group's new effect builds one effect");

    let mut scene = stored(&rig, "main").await;
    scene.groups[0].members.push("fake:b#z0".into());
    scene.groups.remove(1);
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 3 }).unwrap();
    let (status, _) = send(&rig, "PUT", &path, Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rig.app.scene.builds(), built + 2, "a changed membership takes the one group that is left, built again");
}

/// A light patch that renames rebuilds nothing; one that changes what the engine plays does.
#[tokio::test]
async fn a_name_only_light_patch_builds_nothing() {
    let rig = rig().await;
    let built = rig.app.scene.builds();
    let path = route(routes::LIGHT, &[("id", "fake:a%23z0")]);
    let (status, _) = send(&rig, "PATCH", &path, Some(serde_json::json!({ "name": "Desk" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rig.app.scene.builds(), built, "a rename builds nothing");
    let (status, _) = send(&rig, "PATCH", &path, Some(serde_json::json!({ "reversed": true }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rig.app.scene.builds(), built + 2, "a direction change takes the whole scene again");
}

/// S2: while the scenes are held in memory a change would be lost, so activate says so.
#[tokio::test]
async fn activate_during_the_migration_wait_is_a_503() {
    let (app, router, home, layouts) = held_app().await;
    canvas_daemon::scenes::apply_active(&app, false).await;
    assert!(app.scene.is_on(), "start-up put the engine in scene mode");
    let held = send_on(&router, "POST", &route(routes::EFFECT_ACTIVATE, &[("id", "solid")]), Some(serde_json::json!({}))).await;
    assert_eq!(held.0, StatusCode::SERVICE_UNAVAILABLE, "{}", held.1);
    assert_eq!(held.1["error"], "service_unavailable");
    assert!(held.1["message"].as_str().is_some_and(|m| m.contains("not written yet")), "{}", held.1);
    assert!(app.store.read().await.effects.saved.is_empty(), "nothing was saved");
    assert!(!home.path.join("effects.json").exists());
    assert_eq!(std::fs::read(home.path.join("layouts.json")).unwrap(), serde_json::to_vec(&layouts).unwrap(), "layouts.json is as it was");
}

/// S3: a whole-scene write passes the same caps and schema fit as a props patch.
#[tokio::test]
async fn put_scene_caps_and_fits_the_props_like_a_patch() {
    let rig = rig().await;
    let path = route(routes::SCENE, &[("id", "main")]);

    let mut huge = Props::new();
    for i in 0..10_000 {
        huge.insert(format!("key{i}"), PropValue::Bool(true));
    }
    let mut scene = stored(&rig, "main").await;
    scene.groups[0].plays = Plays::Effect { effect: "gradient".into(), props: huge };
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &path, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "10 000 keys: {answer}");
    assert_eq!(stored(&rig, "main").await.rev, 1, "refused: nothing changed");

    // A value outside the range is clamped, a key the effect does not have is dropped.
    let mut props = Props::new();
    props.insert("scroll_speed".into(), PropValue::Number(99_999.0));
    props.insert("nonsense".into(), PropValue::Bool(true));
    let mut scene = stored(&rig, "main").await;
    scene.groups[0].plays = Plays::Effect { effect: "gradient".into(), props };
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &path, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let answered = scene_of(answer);
    let Plays::Effect { props, .. } = &answered.groups[0].plays else { panic!("plays an effect") };
    assert_eq!(props.get("scroll_speed"), Some(&PropValue::Number(100.0)), "clamped to the schema's maximum");
    assert!(!props.contains_key("nonsense"), "a key the effect lacks is dropped: {props:?}");
    assert_eq!(stored(&rig, "main").await, answered, "stored as answered");
}

/// S4: in scene mode activate plays on groups and leaves the effect's starting values alone; only
/// `PATCH /effects/{id}/defaults` writes them.
#[tokio::test]
async fn activate_in_scene_mode_never_writes_the_starting_values() {
    let rig = rig().await;
    let body = serde_json::json!({ "props": serde_json::to_value(color((0, 150, 150))).unwrap(), "groups": ["LEFT"] });
    let (status, answer) = send(&rig, "POST", &route(routes::EFFECT_ACTIVATE, &[("id", "solid")]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb((0, 150, 150)), "the group plays the values it was given").await;
    assert!(!rig.app.store.read().await.effects.saved.contains_key("solid"), "no starting values were written");
    assert!(!rig.home.path.join("effects.json").exists(), "effects.json was never written");
    // The route that does write them still does.
    let (status, _) = send(&rig, "PATCH", &route(routes::EFFECT_DEFAULTS, &[("id", "solid")]), Some(serde_json::to_value(color(GREEN)).unwrap())).await;
    assert_eq!(status, StatusCode::OK);
    assert!(rig.app.store.read().await.effects.saved.contains_key("solid"));
}

/// An activate with no values starts from the effect's saved starting values.
#[tokio::test]
async fn activate_with_no_props_starts_from_the_saved_values() {
    let rig = rig().await;
    let (status, _) = send(&rig, "PATCH", &route(routes::EFFECT_DEFAULTS, &[("id", "solid")]), Some(serde_json::to_value(color((0, 90, 200))).unwrap())).await;
    assert_eq!(status, StatusCode::OK);
    let (status, answer) = send(&rig, "POST", &route(routes::EFFECT_ACTIVATE, &[("id", "solid")]), Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    shows(&rig.a, rgb((0, 90, 200)), "A starts from the saved values").await;
    shows(&rig.b, rgb((0, 90, 200)), "B starts from the saved values").await;
    assert!(stored(&rig, "main").await.groups.iter().all(|g| matches!(&g.plays, Plays::Effect { props, .. } if *props == color((0, 90, 200)))));
}

// ---------------------------------------------------------------- HTML effects, names, caps (fix pass)

/// An HTML effect that draws a moving square on a dark ground: never all black once it runs.
const SPARK: &str = r##"<head>
  <title>Spark</title>
  <meta description="A test effect." />
  <meta property="speed" label="Speed" type="number" min="0" max="100" default="10" />
</head>
<body><canvas id="stage" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("stage");
  var ctx = c.getContext("2d");
  var x = 0;
  function draw() {
    ctx.fillStyle = "#101020";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = "#ffcc00";
    ctx.fillRect(x, 0, 320, 200);
    x = (x + 3) % 300;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

async fn save_stack(rig: &Rig, name: &str, effect: &str) {
    let body = serde_json::json!({
        "id": format!("stack:{name}"),
        "layers": [{ "id": "l1", "name": "One", "effect": effect }],
    });
    let (status, answer) = send(rig, "PUT", &format!("/api/v1/stacks/stack:{name}"), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

fn plays(effect: &str) -> Plays {
    Plays::Effect { effect: effect.into(), props: Props::new() }
}

/// S6: a stack with an HTML layer is an HTML group, on a whole-scene write and on activate.
#[tokio::test]
async fn a_stack_with_an_html_layer_counts_toward_the_html_limit() {
    let rig = rig_in(scenes_file(), &[("spark.html", SPARK)]).await;
    save_stack(&rig, "Glow", "html:spark").await;
    save_stack(&rig, "Plain", "solid").await;
    let path = route(routes::SCENE, &[("id", "main")]);
    let before = stored(&rig, "main").await;

    // PUT: an HTML stack and an HTML effect are two HTML groups.
    let mut scene = before.clone();
    scene.groups[0].plays = plays("stack:Glow");
    scene.groups[1].plays = plays("html:spark");
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &path, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert!(answer["message"].as_str().is_some_and(|m| m.contains("HTML")), "{answer}");
    assert_eq!(stored(&rig, "main").await, before, "refused: nothing changed");

    // A stack without an HTML layer is not one: two of them pass.
    let mut scene = before.clone();
    scene.groups[0].plays = plays("stack:Plain");
    scene.groups[1].plays = plays("stack:Plain");
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &path, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    // Activate on every group: two groups playing one HTML effect, or one HTML stack, is over the
    // limit; so is naming a second group for an HTML effect while the first already plays one.
    let before = stored(&rig, "main").await;
    for effect in ["html:spark", "stack:Glow"] {
        let (status, answer) = send(&rig, "POST", &route(routes::EFFECT_ACTIVATE, &[("id", effect)]), Some(serde_json::json!({}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{effect}: {answer}");
    }
    assert_eq!(stored(&rig, "main").await, before, "refused: the scene is as it was");
}

/// A whole-scene write refuses an empty scene or group name, as create does, and an over-long one.
#[tokio::test]
async fn put_scene_refuses_empty_names() {
    use canvas_proto::scene::{GROUP_NAME_MAX, SCENE_NAME_MAX};
    let rig = rig().await;
    let path = route(routes::SCENE, &[("id", "main")]);
    let before = stored(&rig, "main").await;
    type Change = Box<dyn Fn(&mut Scene)>;
    let cases: [(&str, Change); 4] = [
        ("an empty scene name", Box::new(|s| s.name = "   ".into())),
        ("an empty group name", Box::new(|s| s.groups[0].name = String::new())),
        ("a long scene name", Box::new(|s| s.name = "n".repeat(SCENE_NAME_MAX + 1))),
        ("a long group name", Box::new(|s| s.groups[1].name = "n".repeat(GROUP_NAME_MAX + 1))),
    ];
    for (what, change) in cases {
        let mut scene = before.clone();
        change(&mut scene);
        let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
        let (status, answer) = send(&rig, "PUT", &path, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{what}: {answer}");
    }
    assert_eq!(stored(&rig, "main").await, before, "every refusal left the scene as it was");
}

/// The flash takes a bounded list: a longer one is refused before any light is looked up.
#[tokio::test]
async fn flash_lights_takes_a_bounded_list() {
    let rig = rig().await;
    let cap = canvas_daemon::api::scenes::FLASH_LIGHTS_MAX;
    let lights: Vec<&str> = vec!["fake:a#z0"; cap + 1];
    let (status, answer) = send(&rig, "POST", routes::LIGHTS_FLASH, Some(serde_json::json!({ "lights": lights }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert!(!rig.app.identify.is_active(), "nothing flashed");
    let lights: Vec<&str> = vec!["fake:a#z0"; cap];
    let (status, _) = send(&rig, "POST", routes::LIGHTS_FLASH, Some(serde_json::json!({ "lights": lights }))).await;
    assert_eq!(status, StatusCode::OK, "a list at the cap is taken");
}

/// Whether `sink`'s newest frame has a lit LED.
fn is_lit(sink: &Rec) -> bool {
    sink.last().is_some_and(|f| f.iter().any(|c| *c != Rgb8::BLACK))
}

async fn wait_lit(sink: &Rec, what: &str) {
    let start = Instant::now();
    while !is_lit(sink) {
        assert!(start.elapsed() < Duration::from_secs(20), "{what}: never lit");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A group dark because its effect file was gone comes back when a save names the same effect once
/// the file is there again.
#[tokio::test]
async fn a_dark_group_is_rebuilt_when_its_file_returns_and_a_save_names_it() {
    let mut scenes = scenes_file();
    scenes.scenes[0].groups[0].plays = plays("html:spark");
    let rig = rig_in(scenes, &[]).await;
    shows(&rig.a, Rgb8::BLACK, "no file: dark").await;
    let state = |rig: &Rig| rig.app.scene.failures().get("left").cloned();
    assert!(state(&rig).is_some_and(|why| why.contains("spark")), "named: {:?}", state(&rig));

    // The file returns. A save that names the same effect (and changes only a value) lights it.
    std::fs::write(rig.home.path.join("effects").join("spark.html"), SPARK).unwrap();
    rig.app.user_effects.rescan(&rig.home.path.join("effects"));
    let mut scene = stored(&rig, "main").await;
    scene.groups[0].plays = Plays::Effect { effect: "html:spark".into(), props: color(GREEN) };
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(state(&rig), None, "no longer named as failed");
    wait_lit(&rig.a, "the rebuilt group").await;
    shows(&rig.b, rgb(BLUE), "the other group is untouched").await;
}

/// A save names an effect that cannot be built: the store keeps what was saved, the group is dark
/// and named, the other group runs. Store and engine agree: the store says what the group is meant
/// to play, the engine says why it is dark.
#[tokio::test]
async fn a_save_that_cannot_build_leaves_the_store_and_the_engine_agreeing() {
    let rig = rig().await;
    shows(&rig.a, rgb(RED), "before").await;
    let mut scene = stored(&rig, "main").await;
    scene.groups[0].plays = plays("html:not-there");
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(stored(&rig, "main").await.groups[0].plays, plays("html:not-there"), "the store keeps what was saved");
    shows(&rig.a, Rgb8::BLACK, "the group is dark, not still red").await;
    shows(&rig.b, rgb(BLUE), "the other group runs").await;
    let list: ScenesResponse = serde_json::from_value(send(&rig, "GET", routes::SCENES, None).await.1).unwrap();
    let left = list.group_states.iter().find(|g| g.id == "left").unwrap();
    assert!(left.failed.as_deref().is_some_and(|why| why.contains("not-there")), "named: {left:?}");
}

// ---------------------------------------------------------------- the rest of the plan's rules (fix pass)

/// The status names the active scene's effects once each, and only the active scene's.
#[tokio::test]
async fn status_lists_each_active_effect_once_and_only_from_the_active_scene() {
    let mut scenes = scenes_file();
    scenes.scenes[1].groups[0].plays = plays("gradient");
    let rig = rig_with(scenes).await;
    let effects = |rig: &Rig| {
        let app = Arc::clone(&rig.app);
        async move { app.status().await.active_effects }
    };
    // Two groups play solid and the other scene plays gradient: one entry, no gradient.
    assert_eq!(effects(&rig).await, vec!["solid".to_string()]);
    // A second effect in the active scene is a second entry, in the scene's order.
    let mut scene = stored(&rig, "main").await;
    scene.groups[1].plays = plays("gradient");
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(effects(&rig).await, vec!["solid".to_string(), "gradient".to_string()]);
}

/// A scene change fades: frames between the old and the new colour reach the device.
#[tokio::test]
async fn setting_a_scene_fades_through_frames_between_the_two() {
    let rig = rig().await;
    shows(&rig.a, rgb(RED), "main").await;
    let seen = rig.a.seen();
    let (status, _) = send(&rig, "PUT", routes::SCENE_ACTIVE, Some(serde_json::json!({ "id": "other" }))).await;
    assert_eq!(status, StatusCode::OK);
    shows(&rig.a, rgb(GREEN), "other").await;
    let blended = rig.a.since(seen).into_iter().any(|f| f.iter().all(|c| c.r > 0 && c.r < 200 && c.g > 0 && c.g < 200));
    assert!(blended, "no frame between red and green was sent: {:?}", rig.a.since(seen).iter().map(|f| f[0]).collect::<Vec<_>>());
}

/// Start-up: the saved active scene plays through the same function `main` calls, with no fade, and
/// a group whose effect is gone is dark and named.
#[tokio::test]
async fn start_up_applies_the_active_scene() {
    let mut scenes = scenes_file();
    scenes.scenes[0].groups[1].plays = plays("html:deleted");
    let rig = rig_unstarted(scenes, &[]).await;
    assert!(!rig.app.scene.is_on(), "before start-up the engine runs no scene");
    canvas_daemon::scenes::restore_active(&rig.app).await;
    assert!(rig.app.scene.is_on());
    shows(&rig.a, rgb(RED), "the saved scene's first group").await;
    shows(&rig.b, Rgb8::BLACK, "the group whose effect is gone is dark").await;
    assert!(rig.app.scene.failures().get("right").is_some_and(|why| why.contains("deleted")));
    // No fade at start-up: no frame between the colours the engine has not shown.
    assert!(rig.a.since(0).iter().all(|f| f.iter().all(|c| *c == rgb(RED) || *c == Rgb8::BLACK)), "no blended frame");
}

/// In scene mode the legacy layout call stays out of the engine: the groups keep playing.
#[tokio::test]
async fn set_layout_stays_out_of_the_engine_in_scene_mode() {
    use canvas_proto::layout::{CaseForm, Layout, RenderScale, SamplingMode};
    let rig = rig().await;
    shows(&rig.a, rgb(RED), "before").await;
    let layout = Layout {
        id: "legacy".into(),
        name: "Legacy".into(),
        render_scale: RenderScale::P100,
        sampling: SamplingMode::AreaAverage,
        zones: vec![placed(A, 0.0), placed(B, 160.0)],
        effect: None,
        case_form: CaseForm::None,
        landmarks: Vec::new(),
    };
    let seen = (rig.a.seen(), rig.b.seen());
    rig.app.set_layout(&layout, &[device(A), device(B)]);
    let groups: Vec<String> = rig.app.engine.group_states().iter().map(|g| g.id.clone()).collect();
    assert_eq!(groups, ["left", "right"], "the engine still runs the scene's groups");
    shows_after(&rig.a, seen.0, rgb(RED), "A keeps its group's colour").await;
    shows_after(&rig.b, seen.1, rgb(BLUE), "B keeps its group's colour").await;
}
