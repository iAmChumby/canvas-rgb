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
use std::time::Duration;
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// How long a held read may take to reach its next step, and how long the read itself may take
/// to end. A test that waits for good is a test that never finishes: every wait here is
/// bounded, and a timeout fails the test with the step it was waiting for.
const WAIT: Duration = Duration::from_secs(10);

/// Waits for the next step of a held read, and fails the test when it does not come.
async fn next_step(reached: &mut tokio::sync::mpsc::UnboundedReceiver<()>, what: &str) {
    tokio::time::timeout(WAIT, reached.recv())
        .await
        .unwrap_or_else(|_| panic!("{what}: no step within {WAIT:?}"))
        .unwrap_or_else(|| panic!("{what}: the channel closed"));
}

/// Waits for a read to end, and fails the test when it does not.
async fn ends<T>(reader: tokio::task::JoinHandle<T>, what: &str) -> T {
    tokio::time::timeout(WAIT, reader)
        .await
        .unwrap_or_else(|_| panic!("{what}: the read did not end within {WAIT:?}"))
        .unwrap_or_else(|e| panic!("{what}: the read task failed: {e}"))
}

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
/// The state lives in a temporary directory, so the tests never touch the user's real
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

/// #127: a plain GET never touches the device. With nothing kept it says so; **Read again**
/// (`?fresh=true`) is the one ask that reads.
#[tokio::test]
async fn a_get_with_reads_on_reads_nothing_and_a_fresh_read_reads_the_device() {
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

    let body = json(call(&router, "GET", &path_of(&id)).await).await;
    assert!(
        body.get("values").and_then(|v| v.as_object()).expect("the values").is_empty(),
        "no values are invented on a view open"
    );
    assert!(
        body.get("problem").and_then(|v| v.as_str()).is_some_and(|p| p.contains("not been read yet")),
        "the response says nothing is read yet: {body}"
    );
    assert_eq!(fake.log().reads, 0, "opening the view is no device read");

    let fresh = json(call(&router, "GET", &format!("{}?fresh=true", path_of(&id))).await).await;
    assert_eq!(
        fresh.get("values").and_then(|v| v.as_object()).map(|v| v.len()),
        Some(3),
        "Read again reports the device's own values"
    );
    assert_eq!(fake.log().reads, 1, "Read again is the one ask that reads");
}

/// #127: the daemon start reads the kept values file, so the first open after a restart (or an
/// update, or a logon) is answered from it and makes zero device reads.
#[tokio::test]
async fn a_restart_answers_the_first_open_from_the_kept_values_file_and_reads_nothing() {
    let fake = FakePeripheral::default();
    let home = TempHome::new();
    let id = fake.id().0.clone();
    // The file the daemon wrote before it restarted, with the values the fake read then.
    // An older build wrote it: it names another app version, and the file is kept all the same.
    let mut file = serde_json::json!({"format": 1, "devices": {}});
    file["devices"][id.as_str()] = serde_json::json!({
        "adapter": "fake-peripheral",
        "adapter_version": "0.0.1",
        "read_at_ms": 1_700_000_000_000u64,
        "values": fake.log().values,
    });
    std::fs::write(home.path.join("peripheral-values.json"), serde_json::to_vec(&file).unwrap())
        .expect("the kept values file is written");
    std::fs::write(
        home.path.join("peripherals.json"),
        format!(r#"{{"schema_version":1,"peripherals":{{"{id}":{{"reads":true,"writes":false}}}}}}"#),
    )
    .expect("the access file is written");

    let (router, app, _home) = build_in(fake.clone(), home.path.clone(), home);
    canvas_daemon::peripherals::apply_saved(&app).await;

    let body = json(call(&router, "GET", &path_of(&id)).await).await;
    assert_eq!(
        body.get("values").and_then(|v| v.as_object()).map(|v| v.len()),
        Some(3),
        "the first open after the restart carries the file's values: {body}"
    );
    assert_eq!(body.get("problem"), Some(&serde_json::Value::Null));
    assert_eq!(
        body.get("read_at_ms").and_then(|v| v.as_u64()),
        Some(1_700_000_000_000),
        "the open says when the values were read: {body}"
    );
    assert_eq!(fake.log().reads, 0, "the restart and the first open read nothing");
}

/// #127: with no kept values file the first open still reads nothing, and the response says the
/// settings have not been read yet instead of inventing values.
#[tokio::test]
async fn a_first_open_with_no_kept_values_reads_nothing_and_says_so() {
    let fake = FakePeripheral::default();
    let (router, app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let access = call_with_body(&router, "PUT", &access_path_of(&id), r#"{"reads":true,"writes":false}"#).await;
    assert_eq!(access.status(), StatusCode::OK);
    canvas_daemon::peripherals::apply_saved(&app).await;

    let body = json(call(&router, "GET", &path_of(&id)).await).await;
    assert!(
        body.get("values").and_then(|v| v.as_object()).expect("the values").is_empty(),
        "no value is invented: {body}"
    );
    assert!(
        body.get("problem").and_then(|v| v.as_str()).is_some_and(|p| p.contains("not been read yet")),
        "the response says nothing is read yet: {body}"
    );
    assert_eq!(fake.log().reads, 0, "a first open with no file reads nothing");
}

/// #127: a kept values file that does not parse is ignored and logged; the daemon starts, the
/// route answers, and nothing is read.
#[tokio::test]
async fn a_corrupt_kept_values_file_is_ignored_logged_and_not_fatal() {
    let fake = FakePeripheral::default();
    let home = TempHome::new();
    let id = fake.id().0.clone();
    std::fs::write(home.path.join("peripheral-values.json"), b"{ this is not json")
        .expect("the corrupt file is written");
    let logs = common::LogBuf::default();
    let (router, _app, _home) = {
        let _capture = logs.install();
        build_in(fake.clone(), home.path.clone(), home)
    };
    let text = logs.text();
    assert!(
        text.contains("peripheral-values.json did not parse"),
        "the ignored file is logged: {text}"
    );

    let access = call_with_body(&router, "PUT", &access_path_of(&id), r#"{"reads":true,"writes":false}"#).await;
    assert_eq!(access.status(), StatusCode::OK);
    let body = json(call(&router, "GET", &path_of(&id)).await).await;
    assert!(
        body.get("problem").and_then(|v| v.as_str()).is_some_and(|p| p.contains("not been read yet")),
        "the daemon starts from nothing kept: {body}"
    );
    assert_eq!(fake.log().reads, 0, "a corrupt file never leads to a read on open");
    assert_eq!(
        corrupt_copy(&_home.path).and_then(|p| std::fs::read(p).ok()).as_deref(),
        Some(&b"{ this is not json"[..]),
        "a copy of the corrupt file is kept before the next save overwrites it"
    );
}

/// #127: a file with another format number is ignored too, and a copy of it is kept as
/// `peripheral-values.json.<stamp>.corrupt` before the first save overwrites it.
#[tokio::test]
async fn a_kept_values_file_of_another_format_is_copied_aside_and_ignored() {
    let fake = FakePeripheral::default();
    let home = TempHome::new();
    let id = fake.id().0.clone();
    let other = format!(
        r#"{{"format":999,"devices":{{"{id}":{{"adapter":"fake-peripheral","read_at_ms":1,"values":{{}}}}}}}}"#
    );
    std::fs::write(home.path.join("peripheral-values.json"), &other).expect("the file is written");
    let (router, _app, home) = build_in(fake.clone(), home.path.clone(), home);
    let path = reads_on(&router, &fake, false).await;
    let body = json(call(&router, "GET", &path).await).await;
    assert!(
        body.get("problem").and_then(|v| v.as_str()).is_some_and(|p| p.contains("not been read yet")),
        "the other format's values are not served: {body}"
    );
    assert_eq!(body.get("read_at_ms"), None, "no read time without a read: {body}");
    assert_eq!(fake.log().reads, 0);
    assert_eq!(
        corrupt_copy(&home.path).and_then(|p| std::fs::read_to_string(p).ok()).as_deref(),
        Some(other.as_str()),
        "the other-format file is kept as a copy"
    );
    // Read again keeps what it reads in the usual file; the copy stays.
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    let saved = std::fs::read_to_string(home.path.join("peripheral-values.json")).unwrap();
    assert!(saved.contains("\"format\": 1"), "the new file has the current format: {saved}");
    assert!(corrupt_copy(&home.path).is_some(), "the copy stays");
}

/// The stamped copy of an unusable kept values file (`peripheral-values.json.<stamp>.corrupt`).
fn corrupt_copy(home: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(home).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
        p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("peripheral-values.json.") && n.ends_with(".corrupt"))
    })
}

/// Builds a second daemon on the same home, the way a restart does: new hub, new adapter.
fn restart(home: TempHome) -> (axum::Router, Arc<App>, TempHome, FakePeripheral) {
    let fake = FakePeripheral::default();
    let (router, app, home) = build_in(fake.clone(), home.path.clone(), home);
    (router, app, home, fake)
}

/// #127: what a daemon saves, the next one loads. The file is written by the hub (a Read again),
/// not by the test, and the restarted daemon answers its first open from it with the same read
/// time and no device read. The file names no app version, so an update keeps it.
#[tokio::test]
async fn saved_values_come_back_after_a_restart_with_the_time_they_were_read() {
    let fake = FakePeripheral::default();
    let (router, _app, home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    let first = json(call(&router, "GET", &format!("{path}?fresh=true")).await).await;
    let read_at = first.get("read_at_ms").and_then(|v| v.as_u64()).expect("a read has a time");
    assert!(read_at > 1_700_000_000_000, "the time is Unix milliseconds: {read_at}");
    let text = std::fs::read_to_string(home.path.join("peripheral-values.json")).unwrap();
    assert!(
        !text.contains(env!("CARGO_PKG_VERSION")),
        "the kept values file names no app version, so an update keeps it: {text}"
    );

    let (router, app, home, fake2) = restart(home);
    canvas_daemon::peripherals::apply_saved(&app).await;
    let again = json(call(&router, "GET", &path).await).await;
    assert_eq!(again.get("values"), first.get("values"), "the same values come back");
    assert_eq!(again.get("read_at_ms").and_then(|v| v.as_u64()), Some(read_at), "with the same read time");
    assert_eq!(again.get("problem"), Some(&serde_json::Value::Null));
    assert_eq!(fake2.log().reads, 0, "the restart and the first open read nothing");
    drop(home);
}

/// #127: a write is kept across a restart, and it does not change when the device was read.
#[tokio::test]
async fn a_write_survives_a_restart_and_keeps_the_last_real_read_time() {
    let fake = FakePeripheral::default();
    let (router, _app, home) = build(fake.clone());
    let path = reads_on(&router, &fake, true).await;
    let read = json(call(&router, "GET", &format!("{path}?fresh=true")).await).await;
    let read_at = read.get("read_at_ms").and_then(|v| v.as_u64()).expect("a read has a time");
    tokio::time::sleep(Duration::from_millis(30)).await;
    let patched = call_with_body(
        &router,
        "PATCH",
        &path,
        r#"{"dpi-stage-1":{"type":"number","value":1600}}"#,
    )
    .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let after_write = json(call(&router, "GET", &path).await).await;
    assert_eq!(
        after_write.get("read_at_ms").and_then(|v| v.as_u64()),
        Some(read_at),
        "a write is not a read: the time stays the last real read's"
    );

    let (router, app, home, fake2) = restart(home);
    canvas_daemon::peripherals::apply_saved(&app).await;
    let again = json(call(&router, "GET", &path).await).await;
    assert_eq!(again.pointer("/values/dpi-stage-1/value").and_then(|v| v.as_f64()), Some(1600.0));
    assert_eq!(again.get("read_at_ms").and_then(|v| v.as_u64()), Some(read_at));
    assert_eq!(fake2.log().reads, 0);
    drop(home);
}

/// #127: two plain GETs at once (a reload, a second window) read the device zero times, with
/// nothing kept and with values kept.
#[tokio::test]
async fn two_concurrent_plain_gets_read_the_device_zero_times() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    let (a, b) = tokio::join!(call(&router, "GET", &path), call(&router, "GET", &path));
    let (a, b) = (json(a).await, json(b).await);
    for body in [&a, &b] {
        assert!(
            body.get("problem").and_then(|v| v.as_str()).is_some_and(|p| p.contains("not been read yet")),
            "{body}"
        );
        assert_eq!(body.get("read_at_ms"), None);
    }
    assert_eq!(fake.log().reads, 0, "two opens with nothing kept read nothing");
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    let (a, b) = tokio::join!(call(&router, "GET", &path), call(&router, "GET", &path));
    assert_eq!(json(a).await.pointer("/values/dpi-stage-1/value").and_then(|v| v.as_f64()), Some(800.0));
    assert!(json(b).await.get("read_at_ms").and_then(|v| v.as_u64()).is_some());
    assert_eq!(fake.log().reads, 1, "two opens with values kept read nothing more");
}

/// #127: a device that is still listed but unplugged keeps answering from its kept values, and
/// says when they were read.
#[tokio::test]
async fn an_unplugged_but_still_listed_device_serves_its_kept_values_with_the_read_time() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    {
        let mut log = fake.0.lock().unwrap();
        log.connected = false;
        for info in &mut log.found {
            info.connected = false;
        }
    }
    let list = json(call(&router, "GET", routes::PERIPHERALS).await).await;
    assert_eq!(list.pointer("/peripherals/0/id").and_then(|v| v.as_str()), Some(fake.id().0.as_str()));
    let detail = json(call(&router, "GET", &path).await).await;
    assert_eq!(detail.get("values").and_then(|v| v.as_object()).map(|v| v.len()), Some(3), "{detail}");
    assert!(
        detail.get("read_at_ms").and_then(|v| v.as_u64()).is_some_and(|ms| ms > 0),
        "the kept values say when they were read: {detail}"
    );
    assert_eq!(detail.get("problem"), Some(&serde_json::Value::Null));
    assert_eq!(fake.log().reads, 1, "the unplugged device is not read");
}

/// The path of the fake peripheral's read stage.
fn read_path_of(id: &str) -> String {
    format!("{}/{id}/read", routes::PERIPHERALS)
}

/// The stage the read route reports now, as `(stage, done, total)`.
async fn stage_now(router: &axum::Router, id: &str) -> (String, u64, u64) {
    let response = call(router, "GET", &read_path_of(id)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    (
        body.get("stage").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
        body.get("done").and_then(|v| v.as_u64()).unwrap_or(u64::MAX),
        body.get("total").and_then(|v| v.as_u64()).unwrap_or(u64::MAX),
    )
}

#[tokio::test]
async fn a_read_reports_its_stages_in_order_while_the_device_read_is_still_open() {
    use canvas_proto::peripheral::{ReadProgress, ReadStage};

    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    assert_eq!(stage_now(&router, &id).await, ("idle".into(), 0, 0), "nothing is reading yet");
    let access = call_with_body(&router, "PUT", &access_path_of(&id), r#"{"reads":true,"writes":false}"#).await;
    assert_eq!(access.status(), StatusCode::OK);

    // The device dropped off: the read has to open it again, and every step is held so the test can
    // look at the stage while the read is open.
    let (reached_tx, mut reached_rx) = tokio::sync::mpsc::unbounded_channel();
    let go = Arc::new(tokio::sync::Semaphore::new(0));
    let sent = [
        ReadProgress::at(ReadStage::Waking),
        ReadProgress::reading(0, 2),
        ReadProgress::reading(1, 2),
        ReadProgress::reading(2, 2),
    ];
    {
        let mut log = fake.0.lock().unwrap();
        log.connected = false;
        log.stages = sent.to_vec();
        log.gate = Some(common::StageGate { reached: reached_tx, go: go.clone() });
        log.gate_connect = true;
    }

    let reader = {
        let router = router.clone();
        let path = format!("{}?fresh=true", path_of(&id));
        tokio::spawn(async move { call(&router, "GET", &path).await.status() })
    };

    let mut seen = Vec::new();
    // The open, then the four reported steps.
    for _ in 0..5 {
        next_step(&mut reached_rx, "the read reaches its next step").await;
        seen.push(stage_now(&router, &id).await);
        go.add_permits(1);
    }
    assert_eq!(ends(reader, "the read ends").await, StatusCode::OK);

    let want: Vec<(String, u64, u64)> = vec![
        ("opening".into(), 0, 0),
        ("waking".into(), 0, 0),
        ("reading".into(), 0, 2),
        ("reading".into(), 1, 2),
        ("reading".into(), 2, 2),
    ];
    assert_eq!(seen, want, "the stages come in the order the read went through them");
    assert_eq!(stage_now(&router, &id).await, ("idle".into(), 0, 0), "the stage is cleared when the read ends");
}

#[tokio::test]
async fn a_read_that_reports_nothing_is_one_stage_and_then_idle() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    call_with_body(&router, "PUT", &access_path_of(&id), r#"{"reads":true,"writes":false}"#).await;
    let (reached_tx, mut reached_rx) = tokio::sync::mpsc::unbounded_channel();
    let go = Arc::new(tokio::sync::Semaphore::new(0));
    {
        let mut log = fake.0.lock().unwrap();
        // One step held by the gate, with no stage reported by the adapter.
        log.stages = vec![canvas_proto::peripheral::ReadProgress::at(canvas_proto::peripheral::ReadStage::Reading)];
        log.gate = Some(common::StageGate { reached: reached_tx, go: go.clone() });
    }
    let reader = {
        let router = router.clone();
        let path = format!("{}?fresh=true", path_of(&id));
        tokio::spawn(async move { call(&router, "GET", &path).await.status() })
    };
    next_step(&mut reached_rx, "the read is open").await;
    assert_eq!(stage_now(&router, &id).await, ("reading".into(), 0, 0));
    go.add_permits(1);
    assert_eq!(ends(reader, "the read ends").await, StatusCode::OK);
    assert_eq!(stage_now(&router, &id).await, ("idle".into(), 0, 0));
    // An id that is not a device is idle too: asking opens nothing.
    assert_eq!(stage_now(&router, "nothing-here").await, ("idle".into(), 0, 0));
}

/// Starts a read of the fake that stops at its first reported stage, and waits for it to get there.
async fn start_held_read(
    router: &axum::Router,
    fake: &FakePeripheral,
) -> (tokio::task::JoinHandle<serde_json::Value>, tokio::sync::mpsc::UnboundedReceiver<()>, Arc<tokio::sync::Semaphore>) {
    use canvas_proto::peripheral::{ReadProgress, ReadStage};
    let id = fake.id().0.clone();
    let access = call_with_body(router, "PUT", &access_path_of(&id), r#"{"reads":true,"writes":false}"#).await;
    assert_eq!(access.status(), StatusCode::OK);
    let (reached_tx, mut reached_rx) = tokio::sync::mpsc::unbounded_channel();
    let go = Arc::new(tokio::sync::Semaphore::new(0));
    {
        let mut log = fake.0.lock().unwrap();
        log.stages = vec![ReadProgress::at(ReadStage::Reading), ReadProgress::reading(1, 2)];
        log.gate = Some(common::StageGate { reached: reached_tx, go: go.clone() });
    }
    let reader = {
        let router = router.clone();
        let path = format!("{}?fresh=true", path_of(&id));
        tokio::spawn(async move { json(call(&router, "GET", &path).await).await })
    };
    next_step(&mut reached_rx, "the read is open").await;
    (reader, reached_rx, go)
}

#[tokio::test]
async fn a_cancelled_read_leaves_the_stage_idle_even_when_the_device_still_reports() {
    use canvas_proto::peripheral::ReadProgress;

    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let (reader, _reached, _go) = start_held_read(&router, &fake).await;
    assert_eq!(stage_now(&router, &id).await.0, "reading", "the read is open and says so");

    // The editor navigated away: the request is dropped while the device thread is mid-read.
    reader.abort();
    let _ = tokio::time::timeout(WAIT, reader).await.expect("the dropped read ends within the timeout");
    assert_eq!(stage_now(&router, &id).await, ("idle".into(), 0, 0), "a dropped read clears its stage");

    // The device thread does not know yet and reports once more through the sink it was given.
    let sink = fake.0.lock().unwrap().sink.0.clone().expect("the daemon gave the adapter a sink");
    sink(ReadProgress::reading(2, 2));
    assert_eq!(
        stage_now(&router, &id).await,
        ("idle".into(), 0, 0),
        "a report that arrives after the read ended must not bring the stage back"
    );
}

#[tokio::test]
async fn a_read_that_fails_leaves_the_stage_idle() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    let (reader, mut reached, go) = start_held_read(&router, &fake).await;
    assert_eq!(stage_now(&router, &id).await.0, "reading");
    fake.0.lock().unwrap().fail_read = 1;
    go.add_permits(1);
    next_step(&mut reached, "the second stage").await;
    go.add_permits(1);
    let body = ends(reader, "the read ends").await;
    assert!(body.get("problem").is_some_and(|p| p.is_string()), "the read failed and says so: {body}");
    assert_eq!(stage_now(&router, &id).await, ("idle".into(), 0, 0), "an error ends in idle, not in reading");
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

/// #127: one view open makes zero device reads. The view-open here is the Rail's list, the
/// view's list, and the detail asked for twice at once (a reload or a second window) and again
/// right after.
#[tokio::test]
async fn one_view_open_reads_the_device_zero_times() {
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

    call(&router, "GET", routes::PERIPHERALS).await;
    call(&router, "GET", routes::PERIPHERALS).await;
    let path = path_of(&id);
    let (a, b) = tokio::join!(call(&router, "GET", &path), call(&router, "GET", &path));
    assert_eq!(a.status(), StatusCode::OK);
    assert_eq!(b.status(), StatusCode::OK);
    let again = json(call(&router, "GET", &path).await).await;
    assert!(
        again.get("values").and_then(|v| v.as_object()).expect("the values").is_empty(),
        "the first open has nothing to show yet: {again}"
    );
    assert!(
        again.get("problem").and_then(|v| v.as_str()).is_some_and(|p| p.contains("not been read yet")),
        "the first open says nothing is read yet: {again}"
    );
    assert_eq!(fake.log().reads, 0, "opening the view reads nothing");
    assert_eq!(fake.log().connects, 1, "the access opens the device; a read is what the idle rule gates");
}

/// Opens reads for the fake and returns its id path.
async fn reads_on(router: &axum::Router, fake: &FakePeripheral, writes: bool) -> String {
    let id = fake.id().0.clone();
    let body = format!(r#"{{"reads":true,"writes":{writes}}}"#);
    let access = call_with_body(router, "PUT", &access_path_of(&id), &body).await;
    assert_eq!(access.status(), StatusCode::OK);
    path_of(&id)
}

/// Issue #7/#127: the daemon keeps what it read. A second view-open, however much later, makes
/// no device read (a read stalls the pointer for seconds on the receiver's radio link).
#[tokio::test]
async fn kept_values_answer_a_get_more_than_two_seconds_later_without_a_device_read() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    // Read again is the one ask that reads; the opens after it are answered from the values.
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 1);
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let again = json(call(&router, "GET", &path).await).await;
    assert_eq!(
        again.get("values").and_then(|v| v.as_object()).map(|v| v.len()),
        Some(3),
        "the kept values are the answer"
    );
    assert_eq!(again.get("problem"), Some(&serde_json::Value::Null));
    assert_eq!(fake.log().reads, 1, "one Read again and a view open 2.2 s later: one device read");
}

/// A write is merged into the kept values: the GET after it answers with the written value and
/// the device is not read again.
#[tokio::test]
async fn a_write_is_merged_into_the_kept_values_and_no_read_follows() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, true).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 1);
    let patched = call_with_body(
        &router,
        "PATCH",
        &path,
        r#"{"dpi-stage-1":{"type":"number","value":1600}}"#,
    )
    .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let detail = json(call(&router, "GET", &path).await).await;
    assert_eq!(fake.log().reads, 1, "no device read after the write");
    assert_eq!(
        detail.pointer("/values/dpi-stage-1/value").and_then(|v| v.as_f64()),
        Some(1600.0),
        "the written value is the kept value"
    );
    assert_eq!(
        detail.pointer("/values/dpi-stage-2/value").and_then(|v| v.as_f64()),
        Some(1600.0),
        "the keys that were not written stay"
    );
}

/// `?fresh=true` reads the device even when values are kept, and keeps what it read.
#[tokio::test]
async fn fresh_reads_the_device_even_with_kept_values() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    call(&router, "GET", &path).await;
    assert_eq!(fake.log().reads, 1);
    // The mouse changed on its own (its DPI button).
    fake.0.lock().unwrap().values.insert("dpi-stage-1".into(), PropValue::Number(1200.0));
    let fresh = json(call(&router, "GET", &format!("{path}?fresh=true")).await).await;
    assert_eq!(fake.log().reads, 2, "fresh made a device read");
    assert_eq!(fresh.pointer("/values/dpi-stage-1/value").and_then(|v| v.as_f64()), Some(1200.0));
    let after = json(call(&router, "GET", &path).await).await;
    assert_eq!(fake.log().reads, 2, "the plain GET after it is answered from what it read");
    assert_eq!(after.pointer("/values/dpi-stage-1/value").and_then(|v| v.as_f64()), Some(1200.0));
}

/// Turning reads off and on again closes and reopens the device: the kept values are stale, so
/// the view open says nothing is read yet and only Read again reads.
#[tokio::test]
async fn turning_reads_off_and_on_forgets_the_kept_values() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 1);
    let id = fake.id().0.clone();
    call_with_body(&router, "PUT", &access_path_of(&id), r#"{"reads":false,"writes":false}"#).await;
    reads_on(&router, &fake, false).await;
    let body = json(call(&router, "GET", &path).await).await;
    assert!(
        body.get("problem").and_then(|v| v.as_str()).is_some_and(|p| p.contains("not been read yet")),
        "the reopened device has nothing kept: {body}"
    );
    assert_eq!(fake.log().reads, 1, "the view open still reads nothing");
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 2, "Read again reads the reopened device");
}

/// F2: a change of `writes` alone, or the same access again, makes no device read.
#[tokio::test]
async fn a_writes_only_toggle_or_a_repeat_put_keeps_the_values_and_reads_nothing() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 1);
    reads_on(&router, &fake, true).await; // writes on
    call(&router, "GET", &path).await;
    reads_on(&router, &fake, true).await; // the same again
    call(&router, "GET", &path).await;
    reads_on(&router, &fake, false).await; // writes off
    let detail = json(call(&router, "GET", &path).await).await;
    assert_eq!(fake.log().reads, 1, "no device read for any of them");
    assert_eq!(detail.pointer("/values/dpi-stage-1/value").and_then(|v| v.as_f64()), Some(800.0));
}

/// F1: a key the device changes by itself on a write is in the kept values, not the stale one.
#[tokio::test]
async fn a_key_the_device_changed_by_itself_is_kept_as_the_device_reports_it() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, true).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    fake.0.lock().unwrap().side_effect = Some(("dpi-stage-2".into(), PropValue::Number(3200.0)));
    let patched = call_with_body(
        &router,
        "PATCH",
        &path,
        r#"{"dpi-stage-1":{"type":"number","value":1600}}"#,
    )
    .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let detail = json(call(&router, "GET", &path).await).await;
    assert_eq!(fake.log().reads, 1, "no device read after the write");
    assert_eq!(detail.pointer("/values/dpi-stage-1/value").and_then(|v| v.as_f64()), Some(1600.0));
    assert_eq!(
        detail.pointer("/values/dpi-stage-2/value").and_then(|v| v.as_f64()),
        Some(3200.0),
        "the key the device changed itself is what the device now holds"
    );
}

/// F4: `fresh` takes true, false, 1 and 0; anything else is a JSON validation error.
#[tokio::test]
async fn fresh_takes_true_false_one_and_zero_and_refuses_anything_else_as_json() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 1);
    for (query, reads) in [("fresh=0", 1), ("fresh=false", 1), ("fresh=1", 2), ("fresh=true", 3)] {
        let response = call(&router, "GET", &format!("{path}?{query}")).await;
        assert_eq!(response.status(), StatusCode::OK, "{query}");
        assert_eq!(fake.log().reads, reads, "{query}");
    }
    let response = call(&router, "GET", &format!("{path}?fresh=yes")).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some(canvas_proto::api::error_codes::VALIDATION)
    );
    assert_eq!(fake.log().reads, 3, "a refused ask reads nothing");
}

/// Opens a hub on the fake, reads once, and gates the next reads, so a test can queue asks
/// behind one that is open. Returns the hub, the id, the gate's permits and its step channel.
async fn gated_hub(
    fake: &FakePeripheral,
) -> (
    Arc<canvas_daemon::peripherals::PeripheralHub>,
    DeviceId,
    Arc<tokio::sync::Semaphore>,
    tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    use canvas_daemon::peripherals::{Peripheral, PeripheralHub};
    use canvas_proto::peripheral::{ReadProgress, ReadStage};
    let hub = Arc::new(PeripheralHub::new(vec![Box::new(fake.clone()) as Box<dyn Peripheral>]));
    let id = fake.id();
    hub.apply(&id, PeripheralAccess { reads: true, writes: false }).await.unwrap();
    hub.read_fresh(&id).await.unwrap();
    assert_eq!(fake.log().reads, 1);
    let (reached_tx, reached_rx) = tokio::sync::mpsc::unbounded_channel();
    let go = Arc::new(tokio::sync::Semaphore::new(0));
    {
        let mut log = fake.0.lock().unwrap();
        log.stages = vec![ReadProgress::at(ReadStage::Reading)];
        log.gate = Some(common::StageGate { reached: reached_tx, go: go.clone() });
    }
    (hub, id, go, reached_rx)
}

/// #127: a Read again that arrives while a read is already open does not take that read's
/// result: the read began before the click, so it may not show what the mouse holds now. The
/// ask waits for the open read to end and then reads for itself.
#[tokio::test]
async fn a_fresh_read_asked_while_a_read_is_open_reads_again_because_that_one_started_before_the_click() {
    let fake = FakePeripheral::default();
    let (hub, id, go, mut reached_rx) = gated_hub(&fake).await;
    let first = {
        let (hub, id) = (hub.clone(), id.clone());
        tokio::spawn(async move { hub.read_fresh(&id).await })
    };
    next_step(&mut reached_rx, "the first fresh read is open").await;
    let second = {
        let (hub, id) = (hub.clone(), id.clone());
        tokio::spawn(async move { hub.read_fresh(&id).await })
    };
    // The second ask is queued behind the first one's lock.
    tokio::time::sleep(Duration::from_millis(200)).await;
    go.add_permits(10);
    ends(first, "the first").await.unwrap();
    ends(second, "the second").await.unwrap();
    assert_eq!(fake.log().reads, 3, "the initial read, the open one, and one that started after the click");
}

/// F5, kept: two asks queued behind one open read are one read, because it started after both
/// arrived.
#[tokio::test]
async fn two_fresh_reads_queued_behind_an_open_read_share_the_read_that_starts_after_them() {
    let fake = FakePeripheral::default();
    let (hub, id, go, mut reached_rx) = gated_hub(&fake).await;
    let first = {
        let (hub, id) = (hub.clone(), id.clone());
        tokio::spawn(async move { hub.read_fresh(&id).await })
    };
    next_step(&mut reached_rx, "the first fresh read is open").await;
    let mut queued = Vec::new();
    for _ in 0..2 {
        let (hub, id) = (hub.clone(), id.clone());
        queued.push(tokio::spawn(async move { hub.read_fresh(&id).await }));
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    go.add_permits(10);
    ends(first, "the first").await.unwrap();
    for ask in queued {
        ends(ask, "a queued ask").await.unwrap();
    }
    assert_eq!(fake.log().reads, 3, "the initial read, the open one, and one read for both queued asks");
}

/// A write that failed on the device may have gone part of the way: the kept values are stale.
#[tokio::test]
async fn a_write_that_failed_on_the_device_drops_the_kept_values() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, true).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 1);
    fake.0.lock().unwrap().fail_write = 1;
    let failed = call_with_body(
        &router,
        "PATCH",
        &path,
        r#"{"dpi-stage-1":{"type":"number","value":1600}}"#,
    )
    .await;
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = json(call(&router, "GET", &path).await).await;
    assert!(
        body.get("problem").and_then(|v| v.as_str()).is_some_and(|p| p.contains("not been read yet")),
        "nothing is kept after the failed write: {body}"
    );
    assert_eq!(fake.log().reads, 1, "the view open after it still reads nothing");
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 2, "Read again reads the device");
}

/// A write the adapter refuses before sending anything changed nothing: the values stand.
#[tokio::test]
async fn a_refused_write_keeps_the_values_and_the_next_get_reads_nothing() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, true).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 1);
    for body in [
        r#"{"dpi-stage-99":{"type":"number","value":1600}}"#,
        r#"{"profile":{"type":"choice","value":"2"}}"#,
        r#"{"dpi-stage-1":{"type":"number","value":99999999}}"#,
    ] {
        let refused = call_with_body(&router, "PATCH", &path, body).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    let detail = json(call(&router, "GET", &path).await).await;
    assert_eq!(fake.log().reads, 1, "nothing was read again");
    assert_eq!(detail.pointer("/values/dpi-stage-1/value").and_then(|v| v.as_f64()), Some(800.0));
}

/// `fresh` given twice is the JSON validation error, not axum's plain-text rejection.
#[tokio::test]
async fn fresh_given_twice_is_a_json_validation_error() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    let response = call(&router, "GET", &format!("{path}?fresh=1&fresh=0")).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some(canvas_proto::api::error_codes::VALIDATION)
    );
    assert_eq!(fake.log().reads, 1);
}

/// A fresh read that fails leaves nothing kept: the view open says so and reads nothing, and
/// the next Read again reads.
#[tokio::test]
async fn a_failed_read_drops_the_kept_values() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let path = reads_on(&router, &fake, false).await;
    call(&router, "GET", &format!("{path}?fresh=true")).await;
    assert_eq!(fake.log().reads, 1);
    fake.0.lock().unwrap().fail_read = 1;
    let failed = json(call(&router, "GET", &format!("{path}?fresh=true")).await).await;
    assert!(failed.get("problem").and_then(|v| v.as_str()).is_some(), "the failure is reported");
    assert_eq!(fake.log().reads, 2);
    let again = json(call(&router, "GET", &path).await).await;
    assert_eq!(fake.log().reads, 2, "nothing was kept, and the plain GET still reads nothing");
    assert!(
        again.get("problem").and_then(|v| v.as_str()).is_some_and(|p| p.contains("not been read yet")),
        "the view open says nothing is read yet: {again}"
    );
    let fresh = json(call(&router, "GET", &format!("{path}?fresh=true")).await).await;
    assert_eq!(fake.log().reads, 3, "Read again reads");
    assert_eq!(fresh.get("problem"), Some(&serde_json::Value::Null));
}


/// #127: the values kept from a read are served while the device is not open. This is what a
/// restart looks like: the file's values answer the first open before the adapter is connected.
#[tokio::test]
async fn kept_values_are_served_while_the_device_is_not_open() {
    let fake = FakePeripheral::default();
    let (router, _app, _home) = build(fake.clone());
    let id = fake.id().0.clone();
    call_with_body(&router, "PUT", &access_path_of(&id), r#"{"reads":true,"writes":false}"#).await;
    call(&router, "GET", &format!("{}?fresh=true", path_of(&id))).await;
    assert_eq!(fake.log().reads, 1);
    fake.0.lock().unwrap().connected = false;
    let detail = json(call(&router, "GET", &path_of(&id)).await).await;
    assert_eq!(
        detail.get("values").and_then(|v| v.as_object()).map(|v| v.len()),
        Some(3),
        "the open is answered from the values, not the closed device"
    );
    assert_eq!(fake.log().reads, 1, "the view open reads nothing");
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
    let id = fake.id().0.clone();
    // The list, one device, its access and its read stage: none answers without the token.
    for path in [routes::PERIPHERALS.to_string(), path_of(&id), access_path_of(&id), read_path_of(&id)] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&path)
                    .header("host", "127.0.0.1:9430")
                    .body(Body::empty())
                    .expect("the request builds"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "GET {path} needs the token");
    }
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
    // A read has to happen first; Read again (fresh) is the one that does it (#127).
    let detail = json(call(&router, "GET", &format!("{}?fresh=true", path_of(&id))).await).await;
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
