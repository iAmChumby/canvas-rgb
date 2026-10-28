//! The peripheral routes, tested against a fake adapter.
//!
//! No hardware is touched: the daemon's state is built from the fake in `common`, and the
//! fake records every connect, read and write, so the tests can check that the daemon opened
//! nothing it was not told to.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::cli::{Api, CliError, Output};
use canvas_daemon::state::App;
use canvas_proto::api::routes;
use canvas_proto::device::DeviceId;
use canvas_proto::peripheral::PeripheralAccess;
use canvas_proto::props::PropValue;
use common::{FakeAdapter, FakeEngine, FakePeripheral};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
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
            "canvas-rgb-peripheral-test-{}-{:?}",
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

/// Builds the router and the state the tests share.
///
/// The state lives in a temporary directory, so the tests never touch Luke's real
/// configuration.
fn build(fixture: FakePeripheral) -> (axum::Router, Arc<App>, TempHome) {
    let home = TempHome::new();
    build_in(fixture, home.path.clone(), home)
}

/// Builds the app from the files in `home`, so a file that was written there first is read.
fn build_in(
    fixture: FakePeripheral,
    path: std::path::PathBuf,
    home: TempHome,
) -> (axum::Router, Arc<App>, TempHome) {
    let store = canvas_daemon::storage::Store::load_from(&path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(
        App::with_displays(
            store,
            engine,
            Box::new(FakeAdapter::default()),
            TOKEN.to_string(),
            "0.1.0-test".to_string(),
            tx,
            Box::new(canvas_daemon::displays::NoDisplays),
        )
        .with_peripherals(vec![
            Box::new(fixture) as Box<dyn canvas_daemon::peripherals::Peripheral>
        ]),
    );
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

/// A request with a body.
fn request_with_body(method: &str, path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("the request builds")
}

/// Asks the router something. The router is cloned, so one router answers many requests.
async fn call(router: &axum::Router, method: &str, path: &str) -> axum::response::Response {
    router
        .clone()
        .oneshot(request(method, path))
        .await
        .expect("the router answers")
}

/// Asks the router something with a body.
async fn call_with_body(
    router: &axum::Router,
    method: &str,
    path: &str,
    body: &str,
) -> axum::response::Response {
    router
        .clone()
        .oneshot(request_with_body(method, path, body))
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

/// The path of the fake peripheral.
fn path_of(id: &str) -> String {
    format!("{}/{id}", routes::PERIPHERALS)
}

/// The path of the fake peripheral's access.
fn access_path_of(id: &str) -> String {
    format!("{}/{id}/access", routes::PERIPHERALS)
}

#[tokio::test]
async fn the_peripherals_route_lists_what_the_adapters_see_without_opening_anything() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let response = call(&router, "GET", routes::PERIPHERALS).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let peripherals = body
        .get("peripherals")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(peripherals.len(), 1);
    assert_eq!(
        peripherals[0].get("id").and_then(|v| v.as_str()),
        Some(fake.id().0.as_str())
    );
    assert_eq!(
        peripherals[0].get("kind").and_then(|v| v.as_str()),
        Some("mouse")
    );
    let log = fake.log();
    assert_eq!(log.connects, 0, "the enumeration opens nothing");
    assert_eq!(log.disconnects, 0);
    assert_eq!(log.reads, 0);
}

#[tokio::test]
async fn a_get_with_reads_off_never_reads_the_device() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let response = call(&router, "GET", &path_of(&fake.id().0)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    // The info and the schema are there; the values are not.
    assert_eq!(
        body.get("info").and_then(|i| i.get("id")).and_then(|v| v.as_str()),
        Some(fake.id().0.as_str())
    );
    assert!(body.get("schema").and_then(|s| s.get("sections")).is_some());
    assert!(body
        .get("values")
        .and_then(|v| v.as_object())
        .expect("the values are an object")
        .is_empty());
    let problem = body.get("problem").and_then(|v| v.as_str()).unwrap_or_default();
    assert!(problem.contains("reads are off"), "{problem}");
    let log = fake.log();
    assert_eq!(log.reads, 0, "the device is not read");
    assert_eq!(log.connects, 0, "the device is not opened");
}

#[tokio::test]
async fn a_get_with_reads_on_reads_the_device() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let access = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":true,"writes":false}"#,
    )
    .await;
    assert_eq!(access.status(), StatusCode::OK);

    let response = call(&router, "GET", &path_of(&id)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let values = body.get("values").and_then(|v| v.as_object()).expect("the values");
    assert_eq!(values.len(), 3, "the device's own values are reported");
    assert!(
        body.get("problem").expect("the problem is there").is_null(),
        "a good read has no problem"
    );
    assert_eq!(fake.log().reads, 1);
}

#[tokio::test]
async fn the_access_is_saved_and_survives_a_restart() {
    let fake = FakePeripheral::default();
    let (router, _app, home) = build(fake.clone());
    let id = fake.id().0.clone();
    let response = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":true,"writes":true}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // The file holds it.
    let file = home.path.join("peripherals.json");
    assert!(file.exists(), "the access is saved in peripherals.json");
    let text = std::fs::read_to_string(&file).expect("the file is readable");
    println!("$ cat peripherals.json\n{text}");
    assert!(text.contains(&id), "{text}");
    assert!(text.contains("\"reads\": true"), "{text}");
    assert!(text.contains("\"writes\": true"), "{text}");

    // The adapter's gate follows the access.
    assert!(fake.log().writes_enabled, "the write gate is on");

    // A restart: the store reads the file again, and the daemon applies the access.
    let store = canvas_daemon::storage::Store::load_from(&home.path);
    let saved = store.peripherals.access(&DeviceId(id.clone()));
    assert_eq!(saved, PeripheralAccess { reads: true, writes: true });

    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(
        App::with_displays(
            store,
            engine,
            Box::new(FakeAdapter::default()),
            TOKEN.to_string(),
            "0.1.0-test".to_string(),
            tx,
            Box::new(canvas_daemon::displays::NoDisplays),
        )
        .with_peripherals(vec![
            Box::new(fake.clone()) as Box<dyn canvas_daemon::peripherals::Peripheral>
        ]),
    );
    canvas_daemon::peripherals::apply_saved(&app).await;
    let log = fake.log();
    assert!(log.connected, "the device the owner allowed is open again");
    assert!(log.writes_enabled, "the write gate is on again");
}

#[tokio::test]
async fn writes_without_reads_are_refused() {
    let fake = FakePeripheral::default();
    let (router, _app, home) = build(fake.clone());
    let id = fake.id().0.clone();
    let response = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":false,"writes":true}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some(canvas_proto::api::error_codes::VALIDATION)
    );
    // Nothing was saved, so the device is still untouched.
    assert!(!home.path.join("peripherals.json").exists());
    assert!(!fake.log().writes_enabled);
}

#[tokio::test]
async fn a_patch_with_writes_off_is_refused_and_nothing_reaches_the_device() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let response = call_with_body(
        &router,
        "PATCH",
        &path_of(&id),
        r#"{"dpi-stage-1":{"type":"number","value":1600}}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some(canvas_proto::api::error_codes::VALIDATION)
    );
    assert!(fake.log().writes.is_empty(), "nothing reaches the device");
}

#[tokio::test]
async fn a_patch_with_writes_on_reaches_the_device_with_exactly_the_given_keys() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let access = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":true,"writes":true}"#,
    )
    .await;
    assert_eq!(access.status(), StatusCode::OK);

    let response = call_with_body(
        &router,
        "PATCH",
        &path_of(&id),
        r#"{"dpi-stage-1":{"type":"number","value":1600}}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let writes = fake.log().writes;
    assert_eq!(writes.len(), 1, "the device was written once");
    let keys: Vec<&String> = writes[0].keys().collect();
    assert_eq!(keys, vec!["dpi-stage-1"], "exactly the given keys");
    assert_eq!(writes[0].get("dpi-stage-1"), Some(&PropValue::Number(1600.0)));
}

#[tokio::test]
async fn the_adapter_refuses_an_unknown_key_and_nothing_is_sent() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let access = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":true,"writes":true}"#,
    )
    .await;
    assert_eq!(access.status(), StatusCode::OK);

    let response = call_with_body(
        &router,
        "PATCH",
        &path_of(&id),
        r#"{"dpi-stage-99":{"type":"number","value":1600}}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(fake.log().writes.is_empty(), "nothing reaches the device");
}

#[tokio::test]
async fn the_adapter_refuses_a_read_only_key_and_nothing_is_sent() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let access = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":true,"writes":true}"#,
    )
    .await;
    assert_eq!(access.status(), StatusCode::OK);

    let response = call_with_body(
        &router,
        "PATCH",
        &path_of(&id),
        r#"{"profile":{"type":"choice","value":"2"}}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(fake.log().writes.is_empty(), "nothing reaches the device");
}

#[tokio::test]
async fn turning_writes_off_turns_the_write_gate_off() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let on = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":true,"writes":true}"#,
    )
    .await;
    assert_eq!(on.status(), StatusCode::OK);
    assert!(fake.log().writes_enabled, "the gate is on");

    let off = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":true,"writes":false}"#,
    )
    .await;
    assert_eq!(off.status(), StatusCode::OK);
    assert!(!fake.log().writes_enabled, "the gate is off again");

    // And a write is refused now.
    let response = call_with_body(
        &router,
        "PATCH",
        &path_of(&id),
        r#"{"dpi-stage-1":{"type":"number","value":1600}}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(fake.log().writes.is_empty());
}

#[tokio::test]
async fn turning_reads_off_disconnects_the_device() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let on = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":true,"writes":false}"#,
    )
    .await;
    assert_eq!(on.status(), StatusCode::OK);
    let log = fake.log();
    assert!(log.connected, "the device is open");
    assert_eq!(log.connects, 1);

    let off = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":false,"writes":false}"#,
    )
    .await;
    assert_eq!(off.status(), StatusCode::OK);
    let log = fake.log();
    assert!(!log.connected, "the device is closed, so the vendor app can take it back");
    assert_eq!(log.disconnects, 1);
    assert!(!log.writes_enabled, "the write gate is off with it");
}

#[tokio::test]
async fn a_saved_access_with_writes_but_no_reads_does_not_open_the_device() {
    let fake = FakePeripheral::default();
    let home = TempHome::new();
    let id = fake.id().0.clone();
    // A hand-edited file: the writes are on, the reads are not.
    std::fs::write(
        home.path.join("peripherals.json"),
        format!(
            "{{\"schema_version\":1,\"peripherals\":{{\"{id}\":{{\"reads\":false,\"writes\":true}}}}}}"
        ),
    )
    .expect("the file is written");
    let (router, _app, _home) = build_in(fake.clone(), home.path.clone(), home);

    // The daemon reads it, and the inconsistent access keeps the write gate off.
    canvas_daemon::peripherals::apply_saved(&_app).await;
    assert!(!fake.log().writes_enabled, "writes without reads keep the gate off");

    // A write is refused, and the device is never opened.
    let response = call_with_body(
        &router,
        "PATCH",
        &path_of(&id),
        r#"{"dpi-stage-1":{"type":"number","value":1600}}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let log = fake.log();
    assert!(log.writes.is_empty(), "nothing reaches the device");
    assert_eq!(log.connects, 0, "the device is not opened");
}

#[tokio::test]
async fn an_unknown_peripheral_is_not_found() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let response = call(&router, "GET", &path_of("nope:0000:0000")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_peripheral_routes_need_the_token() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::PERIPHERALS)
                .header("host", "127.0.0.1:9430")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// An API that answers with what it was given, and records what it was asked for.
struct FakeApi {
    answers: BTreeMap<String, serde_json::Value>,
    default: serde_json::Value,
    calls: Mutex<Vec<(String, String, Option<String>)>>,
}

impl FakeApi {
    fn new(default: serde_json::Value) -> Self {
        Self { answers: BTreeMap::new(), default, calls: Mutex::new(Vec::new()) }
    }

    fn with(mut self, key: &str, answer: serde_json::Value) -> Self {
        self.answers.insert(key.to_string(), answer);
        self
    }
}

impl Api for FakeApi {
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<serde_json::Value, CliError> {
        self.calls
            .lock()
            .unwrap()
            .push((method.to_string(), path.to_string(), body.map(str::to_string)));
        // A write is answered with what was written, the way the daemon answers a PUT with
        // the access it saved.
        if let Some(text) = body {
            if method == "PUT" || method == "PATCH" {
                return Ok(serde_json::from_str(text).unwrap_or(serde_json::Value::Null));
            }
        }
        Ok(self
            .answers
            .get(&format!("{method} {path}"))
            .cloned()
            .unwrap_or_else(|| self.default.clone()))
    }
}

#[tokio::test]
async fn the_cli_prints_the_peripherals_and_the_detail() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();

    // The answers the daemon's own routes gave.
    let list = json(call(&router, "GET", routes::PERIPHERALS).await).await;
    let detail = json(call(&router, "GET", &path_of(&id)).await).await;

    let api = FakeApi::new(serde_json::Value::Null)
        .with(&format!("GET {}", routes::PERIPHERALS), list)
        .with(&format!("GET {}/{id}", routes::PERIPHERALS), detail);

    let Output::Text(text) = canvas_daemon::cli::peripherals_list(&api, false)
        .expect("the list prints")
    else {
        panic!("the list is text");
    };
    println!("$ canvas-rgb peripherals list\n{text}");
    assert!(text.contains("Fake Mouse"), "{text}");
    assert!(text.contains(&id), "{text}");

    let Output::Text(text) = canvas_daemon::cli::peripherals_show(&api, &id, false)
        .expect("the detail prints")
    else {
        panic!("the detail is text");
    };
    println!("$ canvas-rgb peripherals show {id}\n{text}");
    assert!(text.contains("reads off"), "{text}");
    assert!(text.contains("reads are off"), "{text}");
    assert!(text.contains("dpi-stage-1"), "{text}");

    // With reads on, the detail carries the device's own values.
    let access = call_with_body(
        &router,
        "PUT",
        &access_path_of(&id),
        r#"{"reads":true,"writes":false}"#,
    )
    .await;
    assert_eq!(access.status(), StatusCode::OK);
    let detail = json(call(&router, "GET", &path_of(&id)).await).await;
    let api = FakeApi::new(serde_json::Value::Null)
        .with(&format!("GET {}/{id}", routes::PERIPHERALS), detail);
    let Output::Text(text) = canvas_daemon::cli::peripherals_show(&api, &id, false)
        .expect("the detail prints")
    else {
        panic!("the detail is text");
    };
    println!("$ canvas-rgb peripherals show {id}   (reads on)\n{text}");
    assert!(text.contains("reads on"), "{text}");
    assert!(text.contains("800"), "the device's own values are shown: {text}");
}

#[tokio::test]
async fn the_cli_access_command_asks_the_daemon_for_the_right_thing() {
    let id = "fake-peripheral:0001:0001";
    let api = FakeApi::new(serde_json::json!({"reads": false, "writes": false}));
    let Output::Text(text) =
        canvas_daemon::cli::peripherals_access(&api, id, Some(true), Some(true), false)
            .expect("the access prints")
    else {
        panic!("the access is text");
    };
    println!("$ canvas-rgb peripherals access {id} --reads on --writes on\n{text}");
    assert_eq!(text, format!("{id}: reads on, writes on\n"));
    let calls = api.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "PUT");
    assert_eq!(calls[0].1, format!("{}/{id}/access", routes::PERIPHERALS));
    let body: serde_json::Value =
        serde_json::from_str(calls[0].2.as_deref().expect("the body")).expect("the body is JSON");
    assert_eq!(body.get("reads").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(body.get("writes").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test]
async fn the_cli_access_command_keeps_the_other_flag_when_only_one_is_given() {
    let id = "fake-peripheral:0001:0001";
    let detail = serde_json::json!({"access": {"reads": true, "writes": false}});
    let api = FakeApi::new(serde_json::json!({"reads": false, "writes": false}))
        .with(&format!("GET {}/{id}", routes::PERIPHERALS), detail);
    let Output::Text(text) =
        canvas_daemon::cli::peripherals_access(&api, id, None, Some(true), false)
            .expect("the access prints")
    else {
        panic!("the access is text");
    };
    assert_eq!(text, format!("{id}: reads on, writes on\n"));
    let calls = api.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "GET");
    assert_eq!(calls[1].0, "PUT");
    let body: serde_json::Value =
        serde_json::from_str(calls[1].2.as_deref().expect("the body")).expect("the body is JSON");
    assert_eq!(body.get("reads").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(body.get("writes").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test]
async fn the_cli_refuses_writes_without_reads() {
    let id = "fake-peripheral:0001:0001";
    let api = FakeApi::new(serde_json::json!({"reads": false, "writes": false}));
    let error = canvas_daemon::cli::peripherals_access(&api, id, Some(false), Some(true), false)
        .expect_err("writes without reads are refused");
    assert!(error.to_string().contains("reads"), "{error}");
    assert!(api.calls.lock().unwrap().is_empty(), "nothing is asked for");
}
