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
use canvas_proto::props::{PropValue, Props};
use canvas_proto::scene::{
    Arrangement, Group, GroupMode, Light, LightsFile, Plays, Scene, ScenesFile, LIGHTS_SCHEMA,
    SCENES_SCHEMA,
};
use canvas_proto::traits::{Engine, EngineSettings, FrameSink};
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

/// A sink that keeps the last frame it is given.
#[derive(Default)]
struct Rec {
    last: Mutex<Option<Vec<Rgb8>>>,
}

impl Rec {
    fn last(&self) -> Option<Vec<Rgb8>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl FrameSink for Rec {
    fn submit(&self, colors: Vec<Rgb8>) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(colors);
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

struct Rig {
    router: axum::Router,
    app: Arc<App>,
    a: Arc<Rec>,
    b: Arc<Rec>,
    home: TempHome,
}

/// The daemon's state around a real engine and the two devices, with `scenes` active.
async fn rig_with(scenes: ScenesFile) -> Rig {
    let home = TempHome::new();
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
    let app = Arc::new(App::new(
        store,
        Arc::clone(&engine),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.0.0-test".to_string(),
        tx,
    ));
    *app.devices.write().await = vec![device(A), device(B)];
    let (a, b) = (Arc::new(Rec::default()), Arc::new(Rec::default()));
    engine.add_sink(DeviceId(A.into()), a.clone());
    engine.add_sink(DeviceId(B.into()), b.clone());
    canvas_daemon::scenes::apply_active(&app, false).await;
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
    pairs.iter().fold(template.to_string(), |path, (key, value)| path.replace(&format!("{{{key}}}"), value))
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
    let mut scene = stored(&rig, "main").await;
    scene.groups[1].members.push("fake:a#z0".into());
    let body = serde_json::to_value(ScenePut { scene, expected_rev: 1 }).unwrap();
    let (status, answer) = send(&rig, "PUT", &route(routes::SCENE, &[("id", "main")]), Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert_eq!(answer["error"], "validation");
    assert_eq!(stored(&rig, "main").await.rev, 1, "refused: nothing changed");
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

    // Stop restores the normal frames; so does silence past the watchdog.
    let (status, _) = send(&rig, "POST", routes::LIGHTS_FLASH, Some(serde_json::json!({ "stop": true }))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!rig.app.identify.is_active(), "the engine's frames flow again");

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

    canvas_daemon::device_sync::apply_devices(&app, vec![device(A)]).await;
    assert!(app.store.read().await.scenes_pending.is_some(), "fake:gone is placed and absent: it waits");
    assert!(!home.path.join("scenes.json").exists() && !home.path.join("lights.json").exists());

    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(app.store.read().await.scenes_pending.is_some(), "30 s in, still waiting");

    // Past the 60 s wait (plus the timer's 50 ms): the timer settles it with the devices present.
    tokio::time::sleep(Duration::from_secs(31)).await;
    for _ in 0..20 {
        if app.store.read().await.scenes_pending.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
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

    // A ring reads the gradient round a circle: the strip's red and blue ends are gone.
    let (status, _) = send(&rig, "PATCH", &path, Some(serde_json::json!({ "arrangement": "ring" }))).await;
    assert_eq!(status, StatusCode::OK);
    let start = Instant::now();
    loop {
        if first(&rig).is_some_and(|(head, tail)| head.b < 255 && tail.r < 200 && head.r > 0 && tail.b > 0) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "the engine never read the light as a ring: {:?}", first(&rig));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
