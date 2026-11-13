//! The lights API (Scenes plan, Task 6 part A): `GET /api/v1/lights` and
//! `PATCH /api/v1/lights/{id}`.
//!
//! The rig is the one `tests/api.rs` uses: the daemon's state built from the in-test fakes,
//! a temporary home, and a router driven with `oneshot`. These tests never touch hardware.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_daemon::storage::Store;
use canvas_proto::api::{routes, ApiError, LightsResponse, LightView};
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::LedSlice;
use canvas_proto::scene::{
    Arrangement, Light, LightsFile, GROUP_NAME_MAX, LIGHTS_SCHEMA,
};
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
            "canvas-rgb-lights-test-{}-{:?}",
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

/// Builds the router, the app and the temp home, with `lights` in the store and `devices` as the
/// discovered list.
async fn build(
    lights: LightsFile,
    devices: Vec<DeviceInfo>,
) -> (axum::Router, Arc<App>, TempHome) {
    let home = TempHome::new();
    let mut store = Store::at(&home.path);
    store.lights = lights;
    build_with(store, devices, home).await
}

async fn build_with(
    store: Store,
    devices: Vec<DeviceInfo>,
    home: TempHome,
) -> (axum::Router, Arc<App>, TempHome) {
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    *app.devices.write().await = devices;
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

/// A request with a JSON body and the token.
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

/// The body of a response, as JSON.
async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("the body is bounded");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// The path of one light. A light id holds `#`, which a URI would read as a fragment: the id is
/// sent percent-encoded, as the editor's own client sends it.
fn light_path(id: &str) -> String {
    routes::LIGHT.replace("{id}", &id.replace('#', "%23"))
}

fn zone(index: u32, name: &str, leds: u32) -> ZoneInfo {
    ZoneInfo {
        index,
        name: name.to_string(),
        kind: ZoneKind::Linear,
        led_count: leds,
        min_leds: 1,
        max_leds: 512,
        resizable: true,
    }
}

fn device(id: &str, name: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.to_string()),
        name: name.to_string(),
        vendor: "fake".to_string(),
        kind: DeviceKind::Motherboard,
        adapter: "fake".to_string(),
        adapter_index: 0,
        serial: id.to_string(),
        location: String::new(),
        zones,
        connected: true,
    }
}

fn light(
    id: &str,
    device_id: &str,
    zone_index: u32,
    leds: Option<(u32, u32)>,
    name: Option<&str>,
) -> Light {
    Light {
        id: id.to_string(),
        device_id: DeviceId(device_id.to_string()),
        zone_index,
        leds: leds.map(|(start, count)| LedSlice { start, count }),
        name: name.map(str::to_string),
        arrangement: Arrangement::Strip,
        reversed: false,
    }
}

fn lights_file(lights: Vec<Light>) -> LightsFile {
    LightsFile { schema_version: LIGHTS_SCHEMA, lights }
}

async fn get_lights(router: axum::Router) -> (StatusCode, serde_json::Value) {
    let response = router
        .oneshot(request("GET", routes::LIGHTS))
        .await
        .expect("the router answers");
    let status = response.status();
    (status, json(response).await)
}

fn views(body: &serde_json::Value) -> LightsResponse {
    serde_json::from_value(body.clone()).expect("the body is a LightsResponse")
}

#[tokio::test]
async fn lights_lists_every_light_with_its_count_and_label() {
    let devices = vec![
        device(
            "fake:mb",
            "MSI B850",
            vec![zone(0, "JARGB 1", 12), zone(1, "JARGB 2", 8)],
        ),
        device("fake:keys", "ROYUAN", vec![zone(0, "Keys", 84)]),
    ];
    let lights = lights_file(vec![
        light("fake:mb#z0", "fake:mb", 0, None, None),
        light("fake:mb#z1p1", "fake:mb", 1, Some((2, 4)), Some("Top fan")),
        // 6 + 8 runs past the zone's 8 LEDs: the count is what is left.
        light("fake:mb#z1p2", "fake:mb", 1, Some((6, 8)), None),
        light("fake:keys#z0", "fake:keys", 0, None, None),
    ]);

    let (router, _app, _home) = build(lights, devices).await;
    let (status, body) = get_lights(router).await;

    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 4, "{body}");
    let ids: Vec<&str> = answer.lights.iter().map(|v| v.light.id.as_str()).collect();
    assert_eq!(ids, vec!["fake:mb#z0", "fake:mb#z1p1", "fake:mb#z1p2", "fake:keys#z0"]);
    let counts: Vec<u32> = answer.lights.iter().map(|v| v.led_count).collect();
    assert_eq!(counts, vec![12, 4, 2, 84], "{body}");
    let labels: Vec<&str> = answer.lights.iter().map(|v| v.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["MSI B850 JARGB 1", "Top fan", "MSI B850 JARGB 2", "ROYUAN Keys"]
    );
    assert!(answer.lights.iter().all(|v| v.connected));
}

#[tokio::test]
async fn a_light_of_an_absent_device_is_listed_disconnected_with_no_leds() {
    let lights = lights_file(vec![light("fake:gone#z0", "fake:gone", 0, None, None)]);
    let (router, _app, _home) = build(lights, Vec::new()).await;
    let (status, body) = get_lights(router).await;

    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 1, "{body}");
    let view = &answer.lights[0];
    assert!(!view.connected, "{body}");
    assert_eq!(view.led_count, 0, "{body}");
    // There is no device name to fall back to, so the light keeps its id.
    assert_eq!(view.label, "fake:gone#z0");
}

#[tokio::test]
async fn light_patch_renames_clears_and_sets_ring_and_reversed() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, Some("Old"))]);
    let (router, app, home) = build(lights, devices).await;

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"name":"  Top fan  ","arrangement":"ring","reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let view: LightView = serde_json::from_value(body.clone()).expect("the body is a LightView");
    assert_eq!(view.light.name.as_deref(), Some("Top fan"), "{body}");
    assert_eq!(view.light.arrangement, Arrangement::Ring);
    assert!(view.light.reversed);
    assert_eq!(view.label, "Top fan", "{body}");
    assert_eq!(view.led_count, 12);

    // The file holds the change, so it survives a restart.
    let saved: LightsFile =
        serde_json::from_str(&std::fs::read_to_string(home.path.join("lights.json")).unwrap())
            .expect("lights.json is a lights file");
    assert_eq!(saved.lights[0].name.as_deref(), Some("Top fan"));
    assert_eq!(saved.lights[0].arrangement, Arrangement::Ring);
    assert!(saved.lights[0].reversed);

    // A null name clears it; the fields left out stay as they were.
    let response = router
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"name":null}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let view: LightView = serde_json::from_value(body.clone()).expect("the body is a LightView");
    assert_eq!(view.light.name, None, "{body}");
    assert_eq!(view.label, "MSI B850 JARGB 1", "{body}");
    assert_eq!(view.light.arrangement, Arrangement::Ring);
    assert!(view.light.reversed);
    assert_eq!(app.store.read().await.lights.lights[0].name, None);
}

#[tokio::test]
async fn light_patch_refuses_an_empty_or_too_long_name() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;

    for bad in [
        r#"{"name":""}"#.to_string(),
        r#"{"name":"   "}"#.to_string(),
        // A name over the limit in characters, not bytes.
        serde_json::json!({ "name": "x".repeat(GROUP_NAME_MAX + 1) }).to_string(),
        serde_json::json!({ "name": "é".repeat(GROUP_NAME_MAX + 1) }).to_string(),
    ] {
        let response = router
            .clone()
            .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), &bad))
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        let body = json(response).await;
        let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
        assert_eq!(error.error, canvas_proto::api::error_codes::VALIDATION);
        assert_eq!(app.store.read().await.lights.lights[0].name, None, "nothing was saved");
    }

    // Exactly the limit is accepted.
    let at_max = serde_json::json!({ "name": "x".repeat(GROUP_NAME_MAX) }).to_string();
    let response = router
        .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), &at_max))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn light_patch_of_an_unknown_light_is_404() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, _app, _home) = build(lights, devices).await;

    let response = router
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z9"),
            r#"{"reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::NOT_FOUND);
}

#[tokio::test]
async fn a_refused_lights_save_is_a_503_with_the_message() {
    // `lights.json` could not be read at start: the store refuses to write over it.
    let home = TempHome::new();
    let mut store = Store::at(&home.path);
    store.lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    store.lights_blocked = true;
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let (router, _app, _home) = build_with(store, devices, home).await;

    let response = router
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::SERVICE_UNAVAILABLE);
    assert!(error.message.contains("lights.json"), "{}", error.message);
}

#[tokio::test]
async fn the_lights_routes_need_the_token() {
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, _app, _home) = build(lights, Vec::new()).await;

    // The routes are there: with the token they answer.
    let with_token = request("GET", routes::LIGHTS);
    let response = router.clone().oneshot(with_token).await.expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    for (method, path) in [
        ("GET", routes::LIGHTS.to_string()),
        ("PATCH", light_path("fake:mb#z0")),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .header("host", "127.0.0.1:9430")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reversed":true}"#.to_string()))
                    .expect("the request builds"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method} {path}");
        let body = json(response).await;
        let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
        assert_eq!(error.error, canvas_proto::api::error_codes::UNAUTHORIZED);
    }
}

/// The guard is on the whole router, so the lights routes answer a browser from another site and
/// a request that names another host the way the other routes do.
#[tokio::test]
async fn the_lights_routes_refuse_a_foreign_origin_and_a_foreign_host() {
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, _app, _home) = build(lights, Vec::new()).await;

    for (method, path) in [
        ("GET", routes::LIGHTS.to_string()),
        ("PATCH", light_path("fake:mb#z0")),
    ] {
        // A browser from another site is refused.
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .header("host", "127.0.0.1:9430")
                    .header("origin", "https://evil.example")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reversed":true}"#.to_string()))
                    .expect("the request builds"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "origin: {method} {path}");
        let body = json(response).await;
        let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
        assert_eq!(error.error, canvas_proto::api::error_codes::FORBIDDEN_ORIGIN);

        // A request that names another host is refused too.
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .header("host", "evil.example")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reversed":true}"#.to_string()))
                    .expect("the request builds"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "host: {method} {path}");
        let body = json(response).await;
        let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
        assert_eq!(error.error, canvas_proto::api::error_codes::BAD_HOST);
    }
}

/// A run that starts past the end of its zone owns no LED at all: there is nothing to light.
#[tokio::test]
async fn a_light_whose_run_starts_past_its_zone_has_no_leds() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    // The run starts at LED 20 of a zone with 12 LEDs.
    let lights = lights_file(vec![light("fake:mb#z0p2", "fake:mb", 0, Some((20, 4)), None)]);
    let (router, _app, _home) = build(lights, devices).await;

    let (status, body) = get_lights(router).await;
    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 1, "{body}");
    assert_eq!(answer.lights[0].led_count, 0, "{body}");
}

/// A patch that leaves the name out keeps the name the light had.
#[tokio::test]
async fn a_patch_without_a_name_keeps_the_name_the_light_had() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, Some("Top fan"))]);
    let (router, app, _home) = build(lights, devices).await;

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let view: LightView = serde_json::from_value(body.clone()).expect("the body is a LightView");
    assert_eq!(view.light.name.as_deref(), Some("Top fan"), "{body}");
    assert!(view.light.reversed, "{body}");
    assert_eq!(app.store.read().await.lights.lights[0].name.as_deref(), Some("Top fan"));
}

/// A save that is refused leaves the light as it was: the list answers the old values.
#[tokio::test]
async fn a_refused_save_leaves_the_light_as_it_was() {
    // `lights.json` could not be read at start: the store refuses to write over it.
    let home = TempHome::new();
    let mut store = Store::at(&home.path);
    store.lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, Some("Top fan"))]);
    store.lights_blocked = true;
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let (router, _app, _home) = build_with(store, devices, home).await;

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"name":"New fan","reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    // The light is as it was: the refused save changed nothing.
    let (status, body) = get_lights(router).await;
    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 1, "{body}");
    assert_eq!(answer.lights[0].light.name.as_deref(), Some("Top fan"), "{body}");
    assert!(!answer.lights[0].light.reversed, "{body}");
}

/// The name's limit is on characters, not bytes: forty multi-byte characters are forty
/// characters and are accepted, forty-one are refused.
#[tokio::test]
async fn a_name_is_counted_in_characters_not_bytes() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;

    let at_max = serde_json::json!({ "name": "é".repeat(GROUP_NAME_MAX) }).to_string();
    let response = router
        .clone()
        .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), &at_max))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK, "40 characters are 40 characters");
    let body = json(response).await;
    let view: LightView = serde_json::from_value(body.clone()).expect("the body is a LightView");
    let expected = "é".repeat(GROUP_NAME_MAX);
    assert_eq!(view.light.name.as_deref(), Some(expected.as_str()), "{body}");

    // One more is over the limit.
    let over = serde_json::json!({ "name": "é".repeat(GROUP_NAME_MAX + 1) }).to_string();
    let response = router
        .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), &over))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::VALIDATION);
    assert_eq!(
        app.store.read().await.lights.lights[0].name.as_deref(),
        Some(expected.as_str()),
        "the refused name was not saved"
    );
}

/// A body the route cannot read is refused in the API's own error shape, like every other
/// validation error, and nothing is saved.
#[tokio::test]
async fn an_unknown_arrangement_is_refused_in_the_api_error_shape() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, app, _home) = build(lights, devices).await;

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"arrangement":"spiral"}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::VALIDATION);
    assert_eq!(
        app.store.read().await.lights.lights[0].arrangement,
        Arrangement::Strip,
        "nothing was saved"
    );
}

/// A patch that changes nothing leaves the file on disk alone.
#[tokio::test]
async fn a_patch_that_changes_nothing_does_not_rewrite_the_lights_file() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    let lights = lights_file(vec![light("fake:mb#z0", "fake:mb", 0, None, None)]);
    let (router, _app, home) = build(lights, devices).await;

    // A change, so the file is there.
    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            &light_path("fake:mb#z0"),
            r#"{"reversed":true}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    // The file's time is put in the past, so a rewrite of the same bytes would show as a new
    // time. The clock is not read to decide anything: the time is set, then compared.
    let path = home.path.join("lights.json");
    let stamp = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    let file = std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("the lights file is there");
    file.set_times(std::fs::FileTimes::new().set_modified(stamp))
        .expect("the file's time is set");
    assert_eq!(modified_at(&path), stamp, "the file's time was set");

    // A patch that changes nothing is answered, and the file is not written.
    let response = router
        .oneshot(request_with_body("PATCH", &light_path("fake:mb#z0"), r#"{}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(modified_at(&path), stamp, "the empty patch rewrote lights.json");
}

/// When the file was last written.
fn modified_at(path: &std::path::Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .expect("the file is there")
        .modified()
        .expect("the file has a time")
}

/// A light of a device that is here but has no such zone is named after the device.
#[tokio::test]
async fn a_light_of_a_zone_the_device_does_not_have_is_labelled_with_the_device_name() {
    let devices = vec![device("fake:mb", "MSI B850", vec![zone(0, "JARGB 1", 12)])];
    // The light names zone 7; the device has only zone 0.
    let lights = lights_file(vec![light("fake:mb#z7", "fake:mb", 7, None, None)]);
    let (router, _app, _home) = build(lights, devices).await;

    let (status, body) = get_lights(router).await;
    assert_eq!(status, StatusCode::OK);
    let answer = views(&body);
    assert_eq!(answer.lights.len(), 1, "{body}");
    assert_eq!(answer.lights[0].label, "MSI B850", "{body}");
    assert_eq!(answer.lights[0].led_count, 0, "{body}");
    assert!(answer.lights[0].connected, "{body}");
}
