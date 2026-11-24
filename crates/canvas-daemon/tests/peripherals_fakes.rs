//! The peripherals of the fake daemon (#243): `--fakes` reports one mouse shaped like the
//! Attack Shark X6 and one controller shaped like the 8BitDo Ultimate 2, so the Peripherals
//! view has something to show in both editors.
//!
//! The tests drive the daemon's own routes over an in-memory router: no socket, no port, no
//! HID device is opened. The adapters are the fakes `canvas_daemon::fakes` wires in.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::api::routes;
use common::{FakeAdapter, FakeEngine};
use std::sync::Arc;
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A temporary directory that is removed when it is dropped.
struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-peripheral-fakes-test-{}-{:?}",
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

/// Builds the fake daemon's router and state, in a home of its own: the fakes-mode wiring, with
/// the fake peripherals and a temp home so `peripherals.json` never touches the real one.
fn build() -> (axum::Router, Arc<App>, TempHome) {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::load_from(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::with_displays(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
        Box::new(canvas_daemon::displays::NoDisplays),
    )
    .with_peripherals(canvas_daemon::fakes::fake_peripherals());
    app.fakes = true;
    let app = Arc::new(app);
    (api::router(app.clone()), app, home)
}

/// A request with the token.
fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

/// A request with a JSON body.
fn request_with_json(method: &str, path: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("the request builds")
}

/// Asks the router something.
async fn call(router: &axum::Router, method: &str, path: &str) -> axum::response::Response {
    router.clone().oneshot(request(method, path)).await.expect("the router answers")
}

/// Asks the router something with a JSON body.
async fn call_json(
    router: &axum::Router,
    method: &str,
    path: &str,
    body: &serde_json::Value,
) -> axum::response::Response {
    router
        .clone()
        .oneshot(request_with_json(method, path, body))
        .await
        .expect("the router answers")
}

/// The body of a response, as JSON.
async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("the body is bounded");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// The path of one peripheral.
fn path_of(id: &str) -> String {
    format!("{}/{id}", routes::PERIPHERALS)
}

/// The path of one peripheral's access.
fn access_path_of(id: &str) -> String {
    format!("{}/{id}/access", routes::PERIPHERALS)
}

/// The id of the one listed peripheral of this kind.
async fn id_of(router: &axum::Router, kind: &str) -> String {
    let list = json(call(router, "GET", routes::PERIPHERALS).await).await;
    list.get("peripherals")
        .and_then(|v| v.as_array())
        .and_then(|peripherals| {
            peripherals
                .iter()
                .find(|p| p.get("kind").and_then(|k| k.as_str()) == Some(kind))
        })
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("a {kind} is listed: {list}"))
        .to_string()
}

/// Gives the device reads and writes, as the person does with the Apply controls.
async fn give_full_access(router: &axum::Router, id: &str) {
    let response = call_json(
        router,
        "PUT",
        &access_path_of(id),
        &serde_json::json!({ "reads": true, "writes": true }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "the access is accepted");
}

/// `GET /peripherals/{id}`, with or without `fresh`.
async fn detail(router: &axum::Router, id: &str, fresh: bool) -> serde_json::Value {
    let path = if fresh { format!("{}?fresh=true", path_of(id)) } else { path_of(id) };
    let response = call(router, "GET", &path).await;
    assert_eq!(response.status(), StatusCode::OK, "the detail answers");
    json(response).await
}

/// Every property key of a detail's schema.
fn schema_keys(detail: &serde_json::Value) -> Vec<String> {
    detail
        .pointer("/schema/sections")
        .and_then(|v| v.as_array())
        .map(|sections| {
            sections
                .iter()
                .flat_map(|section| {
                    section
                        .get("properties")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default()
                })
                .filter_map(|property| property.get("key").and_then(|v| v.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The peripherals list in fakes mode reports both devices, with the shape of the real ones.
#[tokio::test]
async fn the_fakes_list_a_mouse_and_a_controller() {
    let (router, _app, _home) = build();
    let response = call(&router, "GET", routes::PERIPHERALS).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let peripherals = body
        .get("peripherals")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(peripherals.len(), 2, "fakes mode lists the fake mouse and controller: {body}");

    let mouse = peripherals
        .iter()
        .find(|p| p.get("kind").and_then(|k| k.as_str()) == Some("mouse"))
        .unwrap_or_else(|| panic!("a mouse is listed: {body}"));
    let pad = peripherals
        .iter()
        .find(|p| p.get("kind").and_then(|k| k.as_str()) == Some("game_controller"))
        .unwrap_or_else(|| panic!("a controller is listed: {body}"));

    for device in [mouse, pad] {
        let name = device.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        assert!(name.contains("(fake)"), "the device is marked fake: {name}");
    }

    let mouse_id = mouse.get("id").and_then(|v| v.as_str()).expect("the mouse has an id");
    let pad_id = pad.get("id").and_then(|v| v.as_str()).expect("the controller has an id");
    assert!(mouse_id.starts_with("attack-shark-x6:"), "the mouse is the X6's id: {mouse_id}");
    assert!(
        pad_id.starts_with("eightbitdo-ultimate-2:"),
        "the controller is the 8BitDo's id: {pad_id}"
    );

    // The mouse carries the DPI stages the real adapter reports (#229).
    let mouse_detail = detail(&router, mouse_id, false).await;
    let mouse_keys = schema_keys(&mouse_detail);
    for key in ["dpi-stage-count", "dpi-stage-1", "dpi-stage-2", "dpi-stage-3", "dpi-active-stage"] {
        assert!(mouse_keys.iter().any(|k| k == key), "the mouse schema has {key}: {mouse_keys:?}");
    }
    assert_eq!(
        mouse_detail.pointer("/schema/kind").and_then(|v| v.as_str()),
        Some("mouse")
    );

    // The controller carries the profiles and button map the real adapter reports.
    let pad_detail = detail(&router, pad_id, false).await;
    let pad_keys = schema_keys(&pad_detail);
    for key in ["active-profile", "profile-1-in-use", "button-a", "profile-1-vibration-left"] {
        assert!(pad_keys.iter().any(|k| k == key), "the controller schema has {key}: {pad_keys:?}");
    }
    assert_eq!(
        pad_detail.pointer("/schema/kind").and_then(|v| v.as_str()),
        Some("game_controller")
    );
}

/// A settings write to each fake is read back, and never reaches HID: the fake holds it.
#[tokio::test]
async fn a_write_to_each_fake_is_read_back() {
    let (router, _app, _home) = build();
    let list = json(call(&router, "GET", routes::PERIPHERALS).await).await;
    let peripherals = list.get("peripherals").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mouse_id = peripherals
        .iter()
        .find(|p| p.get("kind").and_then(|k| k.as_str()) == Some("mouse"))
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
        .expect("a mouse is listed")
        .to_string();
    let pad_id = peripherals
        .iter()
        .find(|p| p.get("kind").and_then(|k| k.as_str()) == Some("game_controller"))
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
        .expect("a controller is listed")
        .to_string();

    // The mouse: a DPI stage, then read back through both the kept values and a fresh read.
    give_full_access(&router, &mouse_id).await;
    let response = call_json(
        &router,
        "PATCH",
        &path_of(&mouse_id),
        &serde_json::json!({ "dpi-stage-1": { "type": "number", "value": 1600.0 } }),
    )
    .await;
    let status = response.status();
    if status != StatusCode::OK {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("bounded");
        panic!(
            "the mouse takes the write: {status} {}",
            String::from_utf8_lossy(&bytes)
        );
    }
    let written_dpi = serde_json::json!({ "type": "number", "value": 1600.0 });
    for fresh in [true, false] {
        let detail = detail(&router, &mouse_id, fresh).await;
        assert_eq!(
            detail.pointer("/values/dpi-stage-1"),
            Some(&written_dpi),
            "the written DPI stage is read back (fresh={fresh}): {detail}"
        );
        assert_eq!(detail.get("problem"), Some(&serde_json::Value::Null));
    }

    // The controller: a stick range the adapter can write, then read back.
    give_full_access(&router, &pad_id).await;
    let response = call_json(
        &router,
        "PATCH",
        &path_of(&pad_id),
        &serde_json::json!({ "profile-1-stick-left-start": { "type": "number", "value": 10.0 } }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "the controller takes the write");
    let written_range = serde_json::json!({ "type": "number", "value": 10.0 });
    for fresh in [true, false] {
        let detail = detail(&router, &pad_id, fresh).await;
        assert_eq!(
            detail.pointer("/values/profile-1-stick-left-start"),
            Some(&written_range),
            "the written stick range is read back (fresh={fresh}): {detail}"
        );
        assert_eq!(detail.get("problem"), Some(&serde_json::Value::Null));
    }

    // A read-only key is refused and changes nothing, as the real adapters refuse it.
    let response = call_json(
        &router,
        "PATCH",
        &path_of(&mouse_id),
        &serde_json::json!({ "dpi-active-stage": { "type": "choice", "value": "2" } }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "a read-only key is refused");
    let detail = detail(&router, &mouse_id, false).await;
    assert_eq!(
        detail.pointer("/values/dpi-stage-1"),
        Some(&written_dpi),
        "the refused write kept the earlier values: {detail}"
    );
}

/// A write the fake's schema check refuses changes nothing: an out-of-range DPI, an off-step DPI,
/// an option the property does not have, a string where a number belongs, and a key the schema
/// does not have are all refused, and the values the fake holds stand.
///
/// These go through the same PATCH route as the accepted writes. An adapter refusal is a 422
/// `validation` here: `adapter_failure` in `crates/canvas-daemon/src/api/peripherals.rs` maps
/// `AdapterError::Protocol` to `ApiFailure::validation`.
#[tokio::test]
async fn a_refused_write_is_refused_and_changes_nothing() {
    let (router, _app, _home) = build();
    let mouse_id = id_of(&router, "mouse").await;
    give_full_access(&router, &mouse_id).await;

    // A write the mouse takes first, so the values it holds are known and a change would show.
    let accepted = call_json(
        &router,
        "PATCH",
        &path_of(&mouse_id),
        &serde_json::json!({ "dpi-stage-1": { "type": "number", "value": 1600.0 } }),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK, "the mouse takes a valid DPI stage");
    let held_dpi = serde_json::json!({ "type": "number", "value": 1600.0 });

    let refusal = StatusCode::UNPROCESSABLE_ENTITY;
    for (why, props, needle) in [
        // Above the X6's 26000 maximum.
        (
            "an out-of-range DPI",
            serde_json::json!({ "dpi-stage-1": { "type": "number", "value": 30000.0 } }),
            "it takes 50 to 26000",
        ),
        // Inside the range but off the 50 DPI step: (1025 - 50) / 50 is not a whole number.
        (
            "a DPI off the 50 step",
            serde_json::json!({ "dpi-stage-1": { "type": "number", "value": 1025.0 } }),
            "it takes steps of 50",
        ),
        // `led-mode` is a choice of off, static, breathing, neon, ...; "sparkle" is none of them.
        (
            "an option the property does not have",
            serde_json::json!({ "led-mode": { "type": "choice", "value": "sparkle" } }),
            "has no option",
        ),
        // A choice value is a string: the wrong kind for a number.
        (
            "a string for a number",
            serde_json::json!({ "dpi-stage-1": { "type": "choice", "value": "1600" } }),
            "does not take that kind of value",
        ),
        (
            "an unknown key",
            serde_json::json!({ "dpi-stage-99": { "type": "number", "value": 1600.0 } }),
            "has no setting called dpi-stage-99",
        ),
    ] {
        let response = call_json(&router, "PATCH", &path_of(&mouse_id), &props).await;
        assert_eq!(response.status(), refusal, "{why} is refused: {props}");
        // The message names the check that refused it, so each case is refused for its own
        // reason and not by a neighbour's check.
        let body = json(response).await;
        assert_eq!(body.get("error").and_then(|e| e.as_str()), Some("validation"), "{why}: {body}");
        let message = body.get("message").and_then(|m| m.as_str()).unwrap_or_default();
        assert!(message.contains(needle), "{why} is refused with {needle:?}, got {message:?}");
        // A fresh read comes from the fake itself, so this is what it holds.
        let after = detail(&router, &mouse_id, true).await;
        assert_eq!(
            after.pointer("/values/dpi-stage-1"),
            Some(&held_dpi),
            "{why} left the held DPI stage as it was: {after}"
        );
        assert_eq!(after.get("problem"), Some(&serde_json::Value::Null), "{why}: {after}");
    }
}

/// The peripherals fakes mode wires in are the in-memory fakes, never a real adapter: each reports
/// one of the fakes' own adapter names and never the real adapters' names, and each device is
/// marked `(fake)`.
#[tokio::test]
async fn every_wired_peripheral_is_an_in_memory_fake() {
    use canvas_proto::traits::PeripheralAdapter;
    let fake_names = ["fake-attack-shark-x6", "fake-eightbitdo-ultimate-2"];
    let real_names = [
        canvas_adapters::attack_shark_x6::AttackSharkX6::new().name(),
        canvas_adapters::eightbitdo_ultimate2::EightBitDoUltimate2::new().name(),
    ];
    let mut fakes = canvas_daemon::fakes::fake_peripherals();
    assert_eq!(fakes.len(), 2, "one fake per real device");
    for fake in fakes.iter_mut() {
        let name = fake.name();
        assert!(
            fake_names.contains(&name),
            "{name} is one of the fakes' adapter names: {fake_names:?}"
        );
        assert!(
            !real_names.contains(&name),
            "{name} is never a real adapter ({real_names:?}): a real one would open HID"
        );
        let found = fake.discover().await.expect("the fake enumerates");
        assert_eq!(found.len(), 1);
        assert!(
            found[0].name.contains("(fake)"),
            "the device is marked fake: {}",
            found[0].name
        );
        assert!(!found[0].connected, "a listed fake is not one the daemon has opened");
    }
}
