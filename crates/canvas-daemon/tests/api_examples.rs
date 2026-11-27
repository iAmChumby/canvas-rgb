//! Every curl example in the published API reference reaches a real handler.
//!
//! The reference is the same file the developer docs publish. Each curl example is read from it,
//! its URL is filled with the values the fakes carry, and the request goes through the real router
//! with the same in-process fakes the rest of the API tests use. Nothing binds a port, opens a
//! device or reaches the real service pipe.
//!
//! Two URL values are filled before a request is sent. Path and query placeholders such as
//! `<id>` and `<lease_id>` take the values mapped in `fixture` below; `<lease_id>` takes the id
//! the audio test PUT example answered with, so the GET and DELETE examples use a lease that
//! really exists. The example's own headers are sent, with `$TOKEN` resolved to the test token;
//! only the host the guard needs is added. Every example must answer 2xx; the allow-list below is
//! empty, and any entry added to it must fire on this run so a reason cannot go stale.
//!
//! One body value is filled before a request is sent: the session id in a setup body. It comes
//! from the answer of the setup POST example, which is sent again when the request before it
//! ended the session, so every setup body carries a session that is really open. No other body
//! value is changed.
//!
//! The examples write to the store. The config import example writes the settings, zones,
//! effects, displays, lights and scenes files its bundle carries, with the stacks and patterns
//! files beside them and the daemon's own copy of the fan settings. The effect examples write the
//! effects folder. The scene, stack, pattern and light examples write the files they name. The
//! store is left as the last example left it.
//!
//! The devices example with `?rescan` is answered by a stand-in for the OpenRGB supervisor: the
//! fakes carry no OpenRGB link, so the stand-in answers the rescan with the device list the app
//! holds.
//!
//! Set CANVAS_RGB_API_REFERENCE to point the test at another copy of the reference. That is how
//! the guard was shown to fail on a published body made invalid before it was restored.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use common::{FakeAdapter, FakeEngine, FakePeripheral};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

/// The value the published examples' `$TOKEN` resolves to.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The neutral ids the published examples use. They are the ids in the reference, so no body
/// has to be edited before it is sent.
const DEVICE_A: &str = "openrgb:2a3b4c5d";
const DEVICE_B: &str = "openrgb:6e7f8091";

/// The reusable in-process service stand-in answers every fan, fail-sensor and quit request.
/// This wrapper adds the one answer it does not carry: the update example needs `UpdateStarted`
/// where the reusable fake would answer `Ok`. No request leaves the process.
#[derive(Default)]
struct ExamplePipe(canvas_daemon::fakes::FakePipeLink);

#[async_trait::async_trait]
impl canvas_daemon::pipe_client::PipeLink for ExamplePipe {
    async fn exchange(
        &self,
        request: &canvas_proto::pipe::PipeRequest,
    ) -> Result<canvas_proto::pipe::PipeResponse, canvas_daemon::pipe_client::PipeError> {
        if matches!(request, canvas_proto::pipe::PipeRequest::InstallUpdate { .. }) {
            return Ok(canvas_proto::pipe::PipeResponse::UpdateStarted {
                version: "0.18.1".to_string(),
            });
        }
        self.0.exchange(request).await
    }
}

/// No example settles for a refusal. Every request has to answer 2xx; any entry added here would
/// have to name the status it expects and fire on this run, so a reason cannot go stale.
const ALLOWED_NON_2XX: &[(&str, StatusCode, &str)] = &[];

struct TempHome {
    path: PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-examples-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
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

fn reference_path() -> PathBuf {
    std::env::var_os("CANVAS_RGB_API_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site/content/docs/api-reference.json")
        })
}

/// A device with one of the neutral ids the published examples carry.
fn neutral_device(name: &str, id: &str) -> canvas_proto::device::DeviceInfo {
    let mut device = common::device(name, canvas_proto::device::DeviceKind::LedStrip, 12);
    device.id = canvas_proto::device::DeviceId(id.to_string());
    device
}

/// Builds the router with one scene, two devices with zones, a peripheral and a few effects,
/// so the examples reach their handlers with ids that exist. The import example brings the rest
/// of the fixtures with it. The receiver is returned so the quit example's notice has somewhere
/// to go: a dropped receiver would make that example answer 500.
async fn build() -> (axum::Router, Arc<App>, TempHome, tokio::sync::mpsc::UnboundedReceiver<()>) {
    let home = TempHome::new();
    let mut store = canvas_daemon::storage::Store::at(&home.path);
    let (mut lights, mut scenes) = common::a_scene("solid", &canvas_proto::props::Props::new());
    lights.lights[0].id = format!("{DEVICE_A}#z0");
    lights.lights[0].device_id = canvas_proto::device::DeviceId(DEVICE_A.to_string());
    for scene in &mut scenes.scenes {
        for group in &mut scene.groups {
            // The published group value is the id the default scene really gives its only group.
            group.id = "everything".to_string();
            for member in &mut group.members {
                if member == "fake:x#z0" {
                    *member = format!("{DEVICE_A}#z0");
                }
            }
        }
    }
    // A second light gives the delete example its own fixture and leaves the first for the
    // examples that follow it. Both ids are the published ones; no body is edited to reach them.
    lights.lights.push(canvas_proto::scene::Light {
        id: format!("{DEVICE_B}#z0"),
        device_id: canvas_proto::device::DeviceId(DEVICE_B.to_string()),
        zone_index: 0,
        leds: None,
        name: None,
        arrangement: canvas_proto::scene::Arrangement::Strip,
        reversed: false,
        kind: None,
        copies: None,
        first_led: None,
    });
    store.lights = lights;
    store.scenes = scenes;
    let engine = Arc::new(FakeEngine::default());
    let adapter = FakeAdapter::default();
    for (name, id) in [("x", DEVICE_A), ("y", DEVICE_B)] {
        adapter.devices.lock().unwrap().push(neutral_device(name, id));
    }
    let mut peripheral = FakePeripheral::default();
    // The peripheral example writes a `brightness` number. The fake carries that setting so the
    // published body is accepted as written; no body is swapped for a different property.
    {
        let mut log = peripheral.0.lock().unwrap();
        log.schema.sections[0].properties.push(canvas_proto::props::PropertyDef {
            key: "brightness".into(),
            label: "Brightness".into(),
            kind: canvas_proto::props::PropKind::Number {
                min: 0.0,
                max: 100.0,
                step: 1.0,
                default: 30.0,
            },
        });
        log.values.insert(
            "brightness".into(),
            canvas_proto::props::PropValue::Number(30.0),
        );
    }
    // The write example runs before the access example, so the fake starts writable.
    canvas_daemon::peripherals::Peripheral::set_config_writes(&mut peripheral, true);
    let (tx, quit_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::with_displays(
        store,
        engine,
        Box::new(adapter),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
        Box::new(canvas_daemon::displays::NoDisplays),
    )
    .with_peripherals(vec![
        Box::new(peripheral) as Box<dyn canvas_daemon::peripherals::Peripheral>
    ]);
    // The chain routes read the devices again from the adapter, so the adapter carries the same
    // two devices the app starts with.
    app.devices = tokio::sync::RwLock::new(
        [("x", DEVICE_A), ("y", DEVICE_B)]
            .iter()
            .map(|(name, id)| neutral_device(name, id))
            .collect(),
    );
    // The fan read routes answer from the cache the service would have filled. Give them the
    // fake's status so the published fan examples reach their handlers.
    *app.fans.write().await = Some(canvas_daemon::fakes::fake_fans_status());
    // Every pipe request stays in this process: the fan, fail-sensor, quit and install examples
    // must never reach the real service pipe.
    app.pipe = Arc::new(ExamplePipe::default());
    // Effect reads and writes stay inside the temp home.
    let effects_dir = home.path.join("effects");
    std::fs::create_dir_all(&effects_dir).expect("the effects folder is created");
    for stem in ["demo", "example"] {
        std::fs::write(effects_dir.join(format!("{stem}.html")), include_str!("../stock-effects/ink-drop.html"))
            .expect("the effect file is written");
    }
    app.effects_dir = effects_dir.clone();
    app.user_effects.rescan(&effects_dir);
    let peripheral = common::fake_peripheral_id();
    app.store.write().await.peripherals.set(
        &peripheral,
        canvas_proto::peripheral::PeripheralAccess { reads: true, writes: true },
    );
    let app = Arc::new(app);
    // The `?rescan` flag asks the supervisor to read the device list again. The fakes have no
    // OpenRGB link, so this stands in for the supervisor: it answers the rescan with the list the
    // app holds, which is the list the fake adapter would be read for.
    tokio::spawn(answer_rescans(app.clone()));
    (api::router(app.clone()), app, home, quit_rx)
}

/// Answers the rescan requests with the device list the app holds.
async fn answer_rescans(app: Arc<App>) {
    let Some(mut inbox) = app.take_rescan_inbox() else { return };
    while let Some(request) = inbox.recv().await {
        let devices = app.devices.read().await.clone();
        let _ = request.reply.send(Ok(devices));
    }
}

/// The value of one path or query placeholder. This is the only edit made to a published URL: a
/// neutral id is mapped to the fixture that carries it, and `<lease_id>` takes the id the audio
/// test PUT example answered with. A request body is edited in one place only, and that is the
/// session id of a setup body.
fn fixture(url: &str, method: &str, name: &str, lease: Option<&str>) -> String {
    let with_query = url.find("/api/v1").map(|at| &url[at..]).unwrap_or(url);
    let path = with_query.split('?').next().unwrap_or(with_query);
    if name == "lease_id" {
        return lease.expect("the audio test PUT ran before this example").to_string();
    }
    // Some routes delete their fixture. A second fixture of each kind is in the import bundle so
    // the delete has its own id and the later examples still find the first one.
    if method == "DELETE" {
        match path {
            "/api/v1/displays/<id>" if name == "id" => return "display:5f6a:7b8c".to_string(),
            "/api/v1/effects/<id>" if name == "id" => return "html:example".to_string(),
            "/api/v1/effects/<id>/presets/<name>" if name == "name" => return "Cold".to_string(),
            "/api/v1/lights/<id>" if name == "id" => return "openrgb:HID:VID_3151%26PID_4015%23z0".to_string(),
            "/api/v1/patterns/<id>" if name == "id" => return "pattern:Other".to_string(),
            "/api/v1/stacks/<id>" if name == "id" => return "stack:other".to_string(),
            "/api/v1/scenes/<id>" if name == "id" => return "scene:retired".to_string(),
            _ => {}
        }
    }
    let value = match name {
        "zone" => "0",
        "device" => DEVICE_A,
        "h" => "cpu_fan",
        // A hardware note's dismissal key is a hex FNV-1a value (#580).
        "key" => "0123456789abcdef",
        "group" => "everything",
        "name" if path.contains("/fans/profiles/") => "Default",
        "name" => "Warm",
        "stack" => "stack:example",
        "id" if path.starts_with("/api/v1/displays/") => "display:2b1c:3d4e",
        "id" if path.starts_with("/api/v1/lights/") => "openrgb:2a3b4c5d%23z0",
        "id" if path.starts_with("/api/v1/scenes/") => "everything",
        "id" if path.starts_with("/api/v1/peripherals/") => "fake-peripheral:0001:0001",
        "id" if path.starts_with("/api/v1/effects/") => "html:demo",
        "id" if path.starts_with("/api/v1/devices/") => DEVICE_A,
        "id" if path.starts_with("/api/v1/patterns/") => "pattern:Example",
        "id" if path.starts_with("/api/v1/stacks/") => "stack:example",
        other => panic!("no fixture for <{other}> in {path}"),
    };
    value.to_string()
}

/// Replaces every `<name>` in an example URL, in path and query alike.
fn with_fixtures(url: &str, method: &str, lease: Option<&str>) -> String {
    let mut out = String::new();
    let mut rest = url;
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>').expect("a placeholder closes") + start;
        let name = &rest[start + 1..end];
        out.push_str(&rest[..start]);
        out.push_str(&fixture(url, method, name, lease));
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Rewrites the path placeholders of a raw example URL into the braces of its entry's template.
fn path_template(raw_url: &str) -> String {
    let at = raw_url.find("/api/v1").expect("the example has an API path");
    let path = raw_url[at..].split('?').next().unwrap_or(&raw_url[at..]);
    let mut out = String::new();
    let mut rest = path;
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>').expect("a placeholder closes") + start;
        out.push_str(&rest[..start]);
        out.push('{');
        out.push_str(&rest[start + 1..end]);
        out.push('}');
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Splits a curl line on unquoted whitespace, keeping quoted values whole.
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            None => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

struct Example {
    method: String,
    raw_url: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
}

fn parse_example(example: &str, lease: Option<&str>) -> Example {
    let line = example
        .lines()
        .find(|line| line.starts_with("curl "))
        .expect("the example has a curl command");
    let words = tokens(line);
    let mut method = String::from("GET");
    let mut raw_url = None;
    let mut headers = Vec::new();
    let mut body = None;
    let mut i = 1; // token 0 is the curl command itself
    while i < words.len() {
        match words[i].as_str() {
            "-X" => {
                method = words[i + 1].clone();
                i += 2;
            }
            "-H" => {
                let header = &words[i + 1];
                if let Some((name, value)) = header.split_once(':') {
                    headers.push((
                        name.trim().to_string(),
                        value.trim().replace("$TOKEN", TOKEN),
                    ));
                } else {
                    panic!("a header has no value in {line}");
                }
                i += 2;
            }
            "-d" => {
                body = Some(words[i + 1].clone());
                i += 2;
            }
            "-s" => i += 1,
            value if value.starts_with("http://") || value.starts_with("ws://") => {
                raw_url = Some(value.to_string());
                i += 1;
            }
            other => panic!("unexpected curl argument {other} in {line}"),
        }
    }
    let raw_url = raw_url.expect("the example has a URL");
    let url = with_fixtures(&raw_url, &method, lease);
    Example { method, raw_url, url, headers, body }
}

/// Sends one published example through the router and returns its status and body.
async fn send_example(
    router: &axum::Router,
    parsed: &Example,
    body: Option<String>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder()
        .method(parsed.method.as_str())
        .uri(&parsed.url)
        // The guard needs a host; no example carries one and no token is added here.
        .header("host", "127.0.0.1:9430");
    for (name, value) in &parsed.headers {
        request = request.header(name, value);
    }
    let request = request
        .body(match &body {
            Some(body) => Body::from(body.clone()),
            None => Body::empty(),
        })
        .expect("the request builds");
    let response = router.clone().oneshot(request).await.expect("the router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("the body is bounded");
    (status, bytes.to_vec())
}

/// The body of a setup example, with the session id the setup POST answered with in it.
fn setup_body(published: &str, session: &str) -> String {
    let mut body: serde_json::Value =
        serde_json::from_str(published).expect("the setup body is JSON");
    body["session"] = serde_json::json!(session);
    body.to_string()
}

/// Ends the open setup with the published DELETE example, so the header goes back to its earlier
/// length and the zone resize that follows is not refused.
async fn close_the_setup(router: &axum::Router, example: Option<&str>, session: &str) {
    let Some(example) = example else { return };
    let parsed = parse_example(example, None);
    let published = parsed.body.as_deref().expect("the delete setup body");
    let body = setup_body(published, session);
    let (status, bytes) = send_example(router, &parsed, Some(body)).await;
    assert!(
        status.is_success(),
        "the setup DELETE example ends the session: HTTP {status} {}",
        String::from_utf8_lossy(&bytes)
    );
}

/// Starts an audio test lease with the published PUT example and returns its id, for the two
/// examples that renew and end it.
async fn open_audio_lease(router: &axum::Router, example: &str) -> String {
    let parsed = parse_example(example, None);
    let body = parsed.body.clone();
    let (status, bytes) = send_example(router, &parsed, body).await;
    assert!(
        status.is_success(),
        "an audio test lease started: HTTP {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    let answer: serde_json::Value = serde_json::from_slice(&bytes).expect("the lease answer is JSON");
    answer["lease_id"].as_str().expect("the answer names the lease").to_string()
}

#[tokio::test]
async fn every_published_example_reaches_its_handler() {
    let text = std::fs::read_to_string(reference_path()).expect("the reference is readable");
    let reference: serde_json::Value = serde_json::from_str(&text).expect("the reference is JSON");
    let (router, app, _home, _quit_rx) = build().await;
    let mut rejected = Vec::new();
    let mut allowed = Vec::new();
    let mut fired = vec![false; ALLOWED_NON_2XX.len()];
    let mut sent = 0usize;
    let mut skipped = 0usize;
    // The id the setup POST example answered with, and the two setup examples that carry it.
    let mut session: Option<String> = None;
    let mut setup_post: Option<String> = None;
    let mut setup_delete: Option<String> = None;

    for entry in reference["entries"].as_array().expect("the reference has entries") {
        let template = entry["path"].as_str().expect("an entry has a path");
        // A WebSocket handshake cannot be driven with a oneshot request: the three preview
        // routes are covered by their own tests.
        if template.starts_with("/api/v1/preview") {
            skipped += 1;
            continue;
        }
        // The setup POST example opens the session the other setup examples carry. It is read
        // from the entry itself, so the order the examples are read in does not matter.
        if template == "/api/v1/chains/{device}/{zone}/setup" {
            setup_post = entry["examples"]["POST"]
                .as_str()
                .map(|example| example.to_string());
        }
        for (method, example) in entry["examples"].as_object().expect("an entry has examples") {
            let example = example.as_str().expect("an example is text");
            // The earlier documented step runs before the examples that use its answer: the audio
            // test PUT example starts the lease the GET and DELETE examples renew and end.
            let lease = if template == "/api/v1/audio/test" && (method == "GET" || method == "DELETE") {
                let put = entry["examples"]["PUT"].as_str().expect("the audio PUT example");
                Some(open_audio_lease(&router, put).await)
            } else {
                None
            };
            let parsed = parse_example(example, lease.as_deref());
            assert_eq!(&parsed.method, method, "the example's method matches its key");
            assert_eq!(
                path_template(&parsed.raw_url),
                template,
                "the example's URL path matches its entry's template"
            );

            if template == "/api/v1/devices/{id}/zones/{zone}" && method == "PATCH" {
                // A resize is refused while a setup holds the zone. The setup the examples opened
                // is ended with the published DELETE example, and the resizable device list is put
                // back first.
                if let Some(open) = session.clone() {
                    close_the_setup(&router, setup_delete.as_deref(), &open).await;
                    sent += 1;
                }
                session = None;
                *app.devices.write().await = [("x", DEVICE_A), ("y", DEVICE_B)]
                    .iter()
                    .map(|(name, id)| neutral_device(name, id))
                    .collect();
            }

            let setup_route = template.starts_with("/api/v1/chains/")
                && template.contains("/setup");
            let starts_session = template == "/api/v1/chains/{device}/{zone}/setup" && method == "POST";
            let mut body = parsed.body.clone();
            if setup_route && !starts_session {
                // The session comes from the answer of the setup POST example. A request that ends
                // it (the cancel, the commit, the empty) leaves the next one without a session, so
                // the POST example is sent again to open the next one. The device list is put back
                // first, as the commit and the cancel change it.
                if app.identify.setup_session().is_none() {
                    let post = setup_post.as_deref().expect("the setup POST example ran first");
                    *app.devices.write().await = [("x", DEVICE_A), ("y", DEVICE_B)]
                        .iter()
                        .map(|(name, id)| neutral_device(name, id))
                        .collect();
                    let (status, bytes) = send_example(&router, &parse_example(post, None), None).await;
                    assert!(
                        status.is_success(),
                        "the setup POST example opens a session: HTTP {status} {}",
                        String::from_utf8_lossy(&bytes)
                    );
                    sent += 1;
                    session = serde_json::from_slice::<serde_json::Value>(&bytes)
                        .ok()
                        .and_then(|answer| answer["session"].as_str().map(|id| id.to_string()));
                    assert!(session.is_some(), "the setup POST answer names the session");
                }
                let id = session.clone().expect("the setup session is open");
                let published = parsed.body.as_deref().expect("a setup body");
                body = Some(setup_body(published, &id));
            }

            let (status, bytes) = send_example(&router, &parsed, body).await;
            sent += 1;
            // The POST setup example opens the session the next setup requests carry.
            if starts_session && status.is_success() {
                if let Ok(answer) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if let Some(id) = answer["session"].as_str() {
                        session = Some(id.to_string());
                    }
                }
            }
            // The DELETE setup example ends the session the next resize would be refused by.
            if template == "/api/v1/chains/{device}/{zone}/setup" && method == "DELETE" {
                setup_delete = Some(example.to_string());
            }
            // The import swaps the store from disk. Restore the peripheral's saved permission,
            // which a config bundle does not carry, so the write example reaches its handler.
            if template == "/api/v1/config/import" {
                app.store.write().await.peripherals.set(
                    &common::fake_peripheral_id(),
                    canvas_proto::peripheral::PeripheralAccess { reads: true, writes: true },
                );
            }
            if status.is_success() {
                continue;
            }

            let sent_path = &parsed.url[parsed.url.find("/api/v1").expect("an API path")..];
            let key = format!("{method} {sent_path}");
            match ALLOWED_NON_2XX
                .iter()
                .enumerate()
                .find(|(_, (allowed_key, allowed_status, _))| *allowed_key == key && *allowed_status == status)
            {
                Some((index, (_, _, reason))) => {
                    fired[index] = true;
                    allowed.push(format!("{key}: HTTP {status} — {reason}"));
                }
                None => rejected.push(format!(
                    "{key}: HTTP {status} {}",
                    String::from_utf8_lossy(&bytes)
                )),
            }
        }
    }

    assert!(
        rejected.is_empty(),
        "{sent} examples sent, {} rejected:\n{}",
        rejected.len(),
        rejected.join("\n")
    );
    // An entry that never fired is a dead reason: drop it from the list.
    let dead: Vec<&str> = fired
        .iter()
        .zip(ALLOWED_NON_2XX)
        .filter(|(did_fire, _)| !**did_fire)
        .map(|(_, (key, _, _))| *key)
        .collect();
    assert!(dead.is_empty(), "allow-list entries that never fired: {}", dead.join(", "));
    // The allowed non-2xx answers are printed with their reasons so the list stays deliberate.
    eprintln!("{sent} examples sent, {} allowed non-2xx answers:\n{}", allowed.len(), allowed.join("\n"));
    assert!(sent > 80, "only {sent} examples were sent; {skipped} WebSocket routes skipped");
}

/// A raw-byte route answers an unparseable body with JSON 422, not axum's plain-text 400.
#[tokio::test]
async fn a_bad_raw_body_answers_validation() {
    let (router, _app, _home, _quit_rx) = build().await;
    let request = Request::builder()
        .method("POST")
        .uri("http://127.0.0.1:9430/api/v1/effects/import")
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from("this is not JSON"))
        .expect("the request builds");
    let response = router.oneshot(request).await.expect("the router answers");
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unparseable raw body is a validation failure"
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("the answer is bounded");
    let answer: serde_json::Value = serde_json::from_slice(&bytes).expect("the answer is JSON");
    assert_eq!(answer["error"], "validation", "the answer uses the API error shape");
}

/// The handshake headers a browser sends with a preview socket.
const UPGRADE_HEADERS: &str = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==";

/// Sends a raw handshake over an in-memory connection and returns the status line.
async fn handshake(server: &common::MemoryServer, target: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = server.connect();
    let request = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:9430\r\n{UPGRADE_HEADERS}\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("the handshake is written");
    let mut answer = Vec::new();
    let mut buf = [0u8; 512];
    while !answer.contains(&b'\n') {
        let n = stream.read(&mut buf).await.expect("the answer is read");
        assert!(n > 0, "the server closed before it answered");
        answer.extend_from_slice(&buf[..n]);
    }
    String::from_utf8_lossy(&answer).lines().next().unwrap_or("").to_string()
}

#[tokio::test]
async fn every_preview_example_completes_a_websocket_handshake() {
    let text = std::fs::read_to_string(reference_path()).expect("the reference is readable");
    let reference: serde_json::Value = serde_json::from_str(&text).expect("the reference is JSON");
    let (router, _app, _home, _quit_rx) = build().await;
    let server = common::serve_in_memory(router);
    let mut checked = 0;
    for entry in reference["entries"].as_array().expect("the reference has entries") {
        let template = entry["path"].as_str().expect("an entry has a path");
        if !template.starts_with("/api/v1/preview") {
            continue;
        }
        let example = entry["examples"]["GET"].as_str().expect("a preview example is text");
        let line = example.lines().find(|line| line.starts_with("curl ")).expect("a curl command");
        let url = tokens(line).into_iter().find(|word| word.starts_with("ws://")).expect("a WebSocket URL");
        let url = url.replace("$TOKEN", TOKEN);
        let target = &url[url.find("/api/v1").expect("an API path")..];
        let status = handshake(&server, target).await;
        assert!(status.contains("101"), "{template} example did not open a socket: {status}");
        checked += 1;
    }
    assert_eq!(checked, 3, "the three preview examples were not all exercised");
}
