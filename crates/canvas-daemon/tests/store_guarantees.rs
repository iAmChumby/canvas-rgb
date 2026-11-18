//! #241: the guarantees the store keeps now that its saves run off the runtime worker.
//!
//! `tests/blocking_saves.rs` proves a reader is not made to wait for a stalled write. These tests
//! prove what the rewrite must not give up to get there:
//!
//! 1. **A read-modify-write is atomic.** Two writes queued behind a stalled one both start from
//!    what the stalled one left: neither preset create is lost, a patch lands on the light it
//!    named after a delete of another light, and of two replaces holding the same `expected` only
//!    the first wins.
//! 2. **Memory is never ahead of a save that failed.** A displays save and a peripheral-access
//!    save that fail leave the store as it was, with the same 500 text as before.
//! 3. **A save outlives its caller.** A client that drops mid-save leaves memory equal to disk.
//!
//! Every test stalls or fails the daemon's writes with the folder-scoped writer gate
//! (`storage::write_gate_in`), so tests that run side by side in one process do not meet each
//! other's gate. The tests wait on conditions, never on a sleep.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_daemon::storage::{write_gate_in, Store, WriteGate};
use canvas_proto::api::routes;
use canvas_proto::color::Rgb8;
use canvas_proto::config::Settings;
use canvas_proto::device::DeviceId;
use canvas_proto::display::{DisplayConfig, DisplaySource, DisplayUnit};
use canvas_proto::layout::LedSlice;
use canvas_proto::pattern::{BuiltOn, Frame, Motion, Pattern};
use canvas_proto::props::{PropValue, Props};
use canvas_proto::stack::{BlendMode, LayerArea, Stack, StackLayer};
use canvas_proto::scene::{Arrangement, Light, LightsFile, LIGHTS_SCHEMA};
use common::{FakeAdapter, FakeEngine};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A device the daemon does not drive, so a light of it can be removed (`is_driven` is false).
const UNDRIVEN: &str = "openrgb:HID: hid#VID_3151&PID_4015&MI_02#7&abc";

/// A temporary test home, removed when the test ends.
struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir()
            .join(format!("canvas-rgb-store-guarantees-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp home is created");
        Self { path }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Lets a parked write go even if an assertion panics.
struct Release(WriteGate);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn build(home: &TempHome, mut store: Store) -> (axum::Router, Arc<App>) {
    store.home = home.path.clone();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        Arc::new(FakeEngine::default()),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    app.fakes = true;
    app.pipe = Arc::new(canvas_daemon::fakes::FakePipeLink::default());
    app.inputs = Arc::new(canvas_daemon::capture::DaemonInputs::disabled());
    let app = Arc::new(app);
    (api::router(app.clone()), app)
}

fn request(method: &str, uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("the request builds")
}

/// One request's answer: the status and the JSON body.
type Answer = (StatusCode, serde_json::Value);

async fn answer(response: axum::response::Response) -> Answer {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.expect("a bounded body");
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

async fn send(router: &axum::Router, method: &str, uri: &str, body: &str) -> Answer {
    answer(router.clone().oneshot(request(method, uri, body)).await.expect("the router answers")).await
}

/// Starts a request on its own task, so it runs while the test goes on.
fn spawn_send(
    router: &axum::Router,
    method: &'static str,
    uri: String,
    body: String,
) -> tokio::task::JoinHandle<Answer> {
    let router = router.clone();
    tokio::spawn(async move { send(&router, method, &uri, &body).await })
}

/// Parks the next write under `home`, starts a settings PUT that meets it, and waits until that
/// write is parked. Everything that saves after this queues behind it.
async fn park_a_settings_write(
    home: &TempHome,
    router: &axum::Router,
) -> (WriteGate, tokio::task::JoinHandle<Answer>) {
    let gate = write_gate_in(&home.path);
    gate.hold();
    let body = serde_json::to_string(&Settings { fps: 45, ..Default::default() }).expect("settings");
    let put = spawn_send(router, "PUT", routes::SETTINGS.to_string(), body);
    wait_until_parked(&gate).await;
    (gate, put)
}

/// Yields to the runtime until a write is parked in `gate`, with a deadline.
async fn wait_until_parked(gate: &WriteGate) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !gate.wait_until_entered(Duration::ZERO) {
        assert!(Instant::now() < deadline, "no write reached the gate");
        tokio::task::yield_now().await;
    }
}

/// Lets every request that was just started run to the point where it waits for the writer.
async fn let_them_queue() {
    for _ in 0..400 {
        tokio::task::yield_now().await;
    }
}

fn num(value: f64) -> PropValue {
    PropValue::Number(value)
}

fn aurora_props(scale: f64) -> Props {
    let mut props = Props::new();
    props.insert("scale".into(), num(scale));
    props
}

fn preset_names_on_disk(home: &TempHome) -> Vec<String> {
    Store::load_from(&home.path)
        .effects
        .presets
        .get("aurora")
        .map(|list| list.iter().map(|preset| preset.name.clone()).collect())
        .unwrap_or_default()
}

fn light(id: &str, name: &str) -> Light {
    Light {
        id: id.to_string(),
        device_id: DeviceId(UNDRIVEN.to_string()),
        zone_index: 0,
        leds: Some(LedSlice { start: 0, count: 8 }),
        name: Some(name.to_string()),
        arrangement: Arrangement::Strip,
        reversed: false,
        kind: None,
        copies: None,
        first_led: None,
    }
}

fn light_path(id: &str) -> String {
    routes::LIGHT.replace("{id}", &id.replace('#', "%23").replace(' ', "%20"))
}

fn lights_on_disk(home: &TempHome) -> LightsFile {
    let bytes = std::fs::read(home.path.join(canvas_proto::scene::LIGHTS_FILE)).expect("lights.json");
    serde_json::from_slice(&bytes).expect("lights.json parses")
}

// ---- 1. read-modify-write is atomic ----

/// Two creates queued behind a stalled write both land: the second starts from the list the
/// first left, not from the list both saw before either was saved.
#[tokio::test]
async fn two_concurrent_preset_creates_keep_both() {
    let home = TempHome::new("presets-create");
    let (router, _app) = build(&home, Store::at(&home.path));
    let (gate, stalled) = park_a_settings_write(&home, &router).await;
    let _release = Release(gate.clone());

    let create = |name: &str| {
        let body = serde_json::json!({ "name": name, "props": aurora_props(2.0) }).to_string();
        spawn_send(&router, "POST", "/api/v1/effects/aurora/presets".to_string(), body)
    };
    let alpha = create("Alpha");
    let bravo = create("Bravo");
    let_them_queue().await;
    gate.release();

    let (status, _) = stalled.await.expect("the settings PUT finished");
    assert_eq!(status, StatusCode::OK);
    for task in [alpha, bravo] {
        let (status, body) = task.await.expect("the create finished");
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let mut names = preset_names_on_disk(&home);
    names.sort();
    assert_eq!(names, vec!["Alpha".to_string(), "Bravo".to_string()], "effects.json lost a preset");
    let list = send(&router, "GET", "/api/v1/effects/aurora", "").await.1;
    let mut shown: Vec<String> = list["user_presets"]
        .as_array()
        .map(|presets| presets.iter().filter_map(|p| p["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    shown.sort();
    assert_eq!(shown, names, "the API and the file disagree");
}

/// Of two replaces that hold the same `expected`, the second is refused: "changed in another
/// window" needs the check and the write to be one step.
#[tokio::test]
async fn of_two_replaces_with_the_same_expected_only_one_wins() {
    let home = TempHome::new("presets-replace");
    let (router, _app) = build(&home, Store::at(&home.path));
    let created = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        &serde_json::json!({ "name": "Base", "props": aurora_props(1.0) }).to_string(),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    let expected = created.1["user_presets"][0]["props"].clone();

    let (gate, stalled) = park_a_settings_write(&home, &router).await;
    let _release = Release(gate.clone());
    let replace = |scale: f64| {
        let body = serde_json::json!({ "props": aurora_props(scale), "expected": expected }).to_string();
        spawn_send(&router, "PUT", "/api/v1/effects/aurora/presets/Base".to_string(), body)
    };
    let first = replace(2.0);
    let second = replace(3.0);
    let_them_queue().await;
    gate.release();

    assert_eq!(stalled.await.expect("the settings PUT finished").0, StatusCode::OK);
    let mut statuses = vec![
        first.await.expect("the first replace finished").0,
        second.await.expect("the second replace finished").0,
    ];
    statuses.sort();
    assert_eq!(
        statuses,
        vec![StatusCode::OK, StatusCode::CONFLICT],
        "one replace wins and the other is told it was changed in another window"
    );
}

/// A patch of one light that is queued behind a delete of another light lands on the light it
/// named. A patch that found its light by index before the delete would hit the wrong one.
#[tokio::test]
async fn a_patch_racing_a_delete_of_another_light_patches_the_right_light() {
    let home = TempHome::new("patch-delete");
    let a = format!("{UNDRIVEN}#z0p1");
    let b = format!("{UNDRIVEN}#z0p2");
    let mut store = Store::at(&home.path);
    store.lights = LightsFile {
        chains: Vec::new(),
        schema_version: LIGHTS_SCHEMA,
        lights: vec![light(&a, "First"), light(&b, "Second")],
    };
    let (router, app) = build(&home, store);
    let (gate, stalled) = park_a_settings_write(&home, &router).await;
    let _release = Release(gate.clone());

    // The delete goes first and takes a snapshot before it reaches the writer: wait for the
    // snapshot folder, then for the delete to queue.
    let delete = spawn_send(&router, "DELETE", light_path(&a), String::new());
    let deadline = Instant::now() + Duration::from_secs(20);
    let snapshot_taken = || {
        std::fs::read_dir(canvas_daemon::paths::backups_dir_in(&home.path))
            .map(|entries| {
                entries.flatten().any(|e| e.file_name().to_string_lossy().contains("before-removing"))
            })
            .unwrap_or(false)
    };
    while !snapshot_taken() {
        assert!(Instant::now() < deadline, "the delete never took its snapshot");
        tokio::task::yield_now().await;
    }
    let_them_queue().await;
    let patch = spawn_send(&router, "PATCH", light_path(&b), r#"{"name":"Renamed"}"#.to_string());
    let_them_queue().await;
    gate.release();

    assert_eq!(stalled.await.expect("the settings PUT finished").0, StatusCode::OK);
    let (status, body) = delete.await.expect("the delete finished");
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = patch.await.expect("the patch finished");
    assert_eq!(status, StatusCode::OK, "{body}");

    let remaining: Vec<(String, Option<String>)> =
        lights_on_disk(&home).lights.into_iter().map(|l| (l.id, l.name)).collect();
    assert_eq!(remaining, vec![(b.clone(), Some("Renamed".to_string()))], "lights.json");
    let in_memory: Vec<(String, Option<String>)> = app
        .store
        .read()
        .await
        .lights
        .lights
        .iter()
        .map(|l| (l.id.clone(), l.name.clone()))
        .collect();
    assert_eq!(in_memory, remaining, "memory and lights.json agree");
}

// ---- 2. memory is never ahead of a failed save ----

/// A displays save that fails changes nothing: the store and the API answer as before the PUT,
/// and the 500 says what it said before #241.
#[tokio::test]
async fn a_failed_displays_save_leaves_memory_as_it_was() {
    let home = TempHome::new("displays-fail");
    let (router, app) = build(&home, Store::at(&home.path));
    let id = "chione:1b80:b53c";
    let config = |decimals: u8| {
        serde_json::to_string(&DisplayConfig {
            device: DeviceId(id.into()),
            source: DisplaySource::BoardTemp { sensor: "board_temp0".into() },
            unit: DisplayUnit::Celsius,
            decimals,
            blink_above: None,
        })
        .expect("the config serialises")
    };
    let path = routes::DISPLAY.replace("{id}", id);
    let first = send(&router, "PUT", &path, &config(0)).await;
    assert_eq!(first.0, StatusCode::OK, "the first save: {}", first.1);
    let before = serde_json::to_string(&app.store.read().await.displays).expect("displays");

    write_gate_in(&home.path).fail_next();
    let failed = send(&router, "PUT", &path, &config(1)).await;
    assert_eq!(failed.0, StatusCode::INTERNAL_SERVER_ERROR, "{}", failed.1);
    assert!(
        failed.1["message"].as_str().is_some_and(|m| m.starts_with("could not save the display config:")),
        "{}",
        failed.1
    );
    let after = serde_json::to_string(&app.store.read().await.displays).expect("displays");
    assert_eq!(after, before, "a display config that failed to save is in memory");
    let on_disk = Store::load_from(&home.path);
    assert_eq!(serde_json::to_string(&on_disk.displays).expect("displays"), before, "displays.json");
}

/// A peripheral access that fails to save does not take effect: the store keeps the access it had
/// (fail closed), and the 500 text is unchanged.
#[tokio::test]
async fn a_failed_peripheral_access_save_leaves_the_access_as_it_was() {
    let home = TempHome::new("peripherals-fail");
    let (router, app) = build(&home, Store::at(&home.path));
    let path = routes::PERIPHERAL_ACCESS.replace("{id}", "fake:mouse");
    let off = send(&router, "PUT", &path, r#"{"reads":false,"writes":false}"#).await;
    assert_eq!(off.0, StatusCode::OK, "{}", off.1);
    let before = serde_json::to_string(&app.store.read().await.peripherals).expect("peripherals");

    write_gate_in(&home.path).fail_next();
    let failed = send(&router, "PUT", &path, r#"{"reads":true,"writes":true}"#).await;
    assert_eq!(failed.0, StatusCode::INTERNAL_SERVER_ERROR, "{}", failed.1);
    assert!(
        failed.1["message"].as_str().is_some_and(|m| m.starts_with("could not save the peripheral access:")),
        "{}",
        failed.1
    );
    let after = serde_json::to_string(&app.store.read().await.peripherals).expect("peripherals");
    assert_eq!(after, before, "an access that failed to save took effect in memory");
}

/// Makes a file read-only, and writable again when dropped, so the temp home can be removed.
#[cfg(windows)]
struct ReadOnly(std::path::PathBuf);

#[cfg(windows)]
impl ReadOnly {
    fn new(path: std::path::PathBuf) -> Self {
        let mut permissions = std::fs::metadata(&path).expect("the file exists").permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).expect("the file is made read-only");
        Self(path)
    }
}

#[cfg(windows)]
impl Drop for ReadOnly {
    fn drop(&mut self) {
        if let Ok(metadata) = std::fs::metadata(&self.0) {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            let _ = std::fs::set_permissions(&self.0, permissions);
        }
    }
}

/// A light delete writes `lights.json`, then `scenes.json`. When `scenes.json` cannot be written
/// (a read-only file refuses the rename on Windows), the blocking thread puts `lights.json` back
/// itself: the answer is a 503, memory keeps both lights, and `lights.json` is byte for byte what
/// it was (#241: the rollback no longer writes from the store-locked `failed` closure).
#[cfg(windows)]
#[tokio::test]
async fn a_light_delete_whose_scenes_write_fails_puts_lights_json_back() {
    let home = TempHome::new("delete-rollback");
    let a = format!("{UNDRIVEN}#z0p1");
    let b = format!("{UNDRIVEN}#z0p2");
    let mut store = Store::at(&home.path);
    store.lights = LightsFile {
        chains: Vec::new(),
        schema_version: LIGHTS_SCHEMA,
        lights: vec![light(&a, "First"), light(&b, "Second")],
    };
    store.save_lights().expect("lights.json is written");
    store.save_scenes().expect("scenes.json is written");
    let lights_before = std::fs::read(home.path.join(canvas_proto::scene::LIGHTS_FILE)).expect("lights.json");
    let (router, app) = build(&home, store);

    let _read_only = ReadOnly::new(home.path.join(canvas_proto::scene::SCENES_FILE));
    let (status, body) = send(&router, "DELETE", &light_path(&a), "").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");

    let in_memory: Vec<String> =
        app.store.read().await.lights.lights.iter().map(|l| l.id.clone()).collect();
    assert_eq!(in_memory, vec![a.clone(), b.clone()], "memory keeps both lights");
    let lights_after = std::fs::read(home.path.join(canvas_proto::scene::LIGHTS_FILE)).expect("lights.json");
    assert_eq!(lights_after, lights_before, "lights.json is byte for byte what it was");
}

// ---- 3. a save outlives its caller ----

/// A client that drops while its preset write is stalled, then a write that fails: the rollback
/// still runs, so memory holds what the disk holds (no preset).
#[tokio::test]
async fn a_dropped_preset_create_whose_write_fails_leaves_memory_equal_to_disk() {
    let home = TempHome::new("cancel-preset");
    let (router, app) = build(&home, Store::at(&home.path));
    let gate = write_gate_in(&home.path);
    gate.hold();
    gate.fail_next();
    let _release = Release(gate.clone());
    let body = serde_json::json!({ "name": "Gone", "props": aurora_props(2.0) }).to_string();
    let create = spawn_send(&router, "POST", "/api/v1/effects/aurora/presets".to_string(), body);
    wait_until_parked(&gate).await;

    // The client goes away mid-write; then the write fails.
    create.abort();
    assert!(create.await.is_err(), "the request task was cancelled");
    gate.release();

    // A following save takes the writer only after the first one finished its commit or rollback.
    let (status, _) = send(
        &router,
        "PUT",
        routes::SETTINGS,
        &serde_json::to_string(&Settings { fps: 50, ..Default::default() }).expect("settings"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let in_memory: Vec<String> = app
        .store
        .read()
        .await
        .effects
        .presets
        .get("aurora")
        .map(|list| list.iter().map(|preset| preset.name.clone()).collect())
        .unwrap_or_default();
    assert_eq!(in_memory, preset_names_on_disk(&home), "memory and effects.json disagree");
    assert!(in_memory.is_empty(), "a preset whose write failed is in memory: {in_memory:?}");
}

// ---- 3b. a delete of two halves outlives its caller as a whole ----

/// How long a test waits for a detached delete to finish its second half.
const SETTLE: Duration = Duration::from_secs(20);

/// A pattern of four LEDs, saved straight into the daemon's own pattern store.
fn save_pattern(app: &Arc<App>, name: &str) -> String {
    let id = format!("pattern:{name}");
    let pattern = Pattern {
        id: id.clone(),
        built_on: BuiltOn { led_count: 4, arrangement: Arrangement::Ring, grid: None },
        frames: vec![Frame {
            colors: vec![Rgb8::BLACK; 4],
            hold: 0,
            transition: None,
            extra: serde_json::Map::new(),
        }],
        motion: Motion::none(),
        playback: None,
        extra: serde_json::Map::new(),
    };
    app.patterns.put(pattern).expect("the pattern is saved");
    id
}

/// A stack of one solid layer, saved straight into the daemon's own stack store.
fn save_stack(app: &Arc<App>, name: &str) -> String {
    let id = format!("stack:{name}");
    let stack = Stack {
        id: id.clone(),
        layers: vec![StackLayer {
            id: "l1".into(),
            name: "Layer".into(),
            effect: "solid".into(),
            props: Props::new(),
            area: LayerArea::Canvas,
            blend: BlendMode::Normal,
            opacity: 100,
            visible: true,
            mask: None,
        }],
    };
    let known = canvas_daemon::stacks::KnownEffects::from_user(&app.user_effects);
    app.stacks.put(stack, &known).expect("the stack is saved");
    id
}

/// A delete is two writes (effects.json, then the pattern or stack file). A client that drops
/// while the first is parked must still get both or neither: never the favourite gone and the
/// pattern left standing.
#[tokio::test]
async fn a_dropped_pattern_delete_removes_the_favourite_and_the_pattern_together() {
    let home = TempHome::new("cancel-delete-pattern");
    let (router, app) = build(&home, Store::at(&home.path));
    let id = save_pattern(&app, "Doomed");
    app.store.write().await.effects.favorites.push(id.clone());

    let gate = write_gate_in(&home.path);
    gate.hold();
    let _release = Release(gate.clone());
    let delete = spawn_send(&router, "DELETE", format!("/api/v1/patterns/{id}"), String::new());
    wait_until_parked(&gate).await;

    delete.abort();
    assert!(delete.await.is_err(), "the request task was cancelled");
    gate.release();

    let deadline = Instant::now() + SETTLE;
    while app.patterns.get(&id).is_some() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pattern_there = app.patterns.get(&id).is_some();
    let favourite_there = app.store.read().await.effects.favorites.contains(&id);
    assert_eq!(
        pattern_there, favourite_there,
        "half a delete: pattern_there={pattern_there} favourite_there={favourite_there}"
    );
    assert!(!pattern_there, "the delete finished: both are gone");
}

/// The same for a stack: its presets must not go while the stack stays.
#[tokio::test]
async fn a_dropped_stack_delete_removes_the_presets_and_the_stack_together() {
    let home = TempHome::new("cancel-delete-stack");
    let (router, app) = build(&home, Store::at(&home.path));
    let id = save_stack(&app, "Doomed");
    app.store.write().await.effects.presets.insert(id.clone(), Vec::new());

    let gate = write_gate_in(&home.path);
    gate.hold();
    let _release = Release(gate.clone());
    let delete = spawn_send(&router, "DELETE", format!("/api/v1/stacks/{id}"), String::new());
    wait_until_parked(&gate).await;

    delete.abort();
    assert!(delete.await.is_err(), "the request task was cancelled");
    gate.release();

    let deadline = Instant::now() + SETTLE;
    while app.stacks.get(&id).is_some() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let stack_there = app.stacks.get(&id).is_some();
    let presets_there = app.store.read().await.effects.presets.contains_key(&id);
    assert_eq!(
        stack_there, presets_there,
        "half a delete: stack_there={stack_there} presets_there={presets_there}"
    );
    assert!(!stack_there, "the delete finished: both are gone");
}
