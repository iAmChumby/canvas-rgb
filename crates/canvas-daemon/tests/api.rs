//! The API's behaviour, tested against the in-test fakes.
//!
//! These tests never touch the real hardware and never call the real engine, effects or
//! adapters: the daemon's state is built from the fakes in `common`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::api::{routes, ApiError};
use canvas_proto::traits::Effect;
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
            "canvas-rgb-test-{}-{:?}",
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
fn build() -> (axum::Router, Arc<App>, TempHome) {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.path);
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

/// The body of a response, as JSON.
async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("the body is bounded");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

#[tokio::test]
async fn status_needs_no_token() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::STATUS)
                .header("host", "127.0.0.1:9430")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert!(body.get("version").is_some());
}

#[tokio::test]
async fn a_request_without_a_token_is_refused() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::DEVICES)
                .header("host", "127.0.0.1:9430")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::UNAUTHORIZED);
}

#[tokio::test]
async fn a_request_with_a_wrong_token_is_refused() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::DEVICES)
                .header("host", "127.0.0.1:9430")
                .header("authorization", "Bearer not-the-token")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_browser_origin_is_refused() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::DEVICES)
                .header("host", "127.0.0.1:9430")
                .header("origin", "https://evil.example")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::FORBIDDEN_ORIGIN);
}

#[tokio::test]
async fn a_foreign_host_is_refused() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::DEVICES)
                .header("host", "evil.example")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::BAD_HOST);
}

#[tokio::test]
async fn the_tauri_origin_is_allowed_and_gets_cors_headers() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::DEVICES)
                .header("host", "127.0.0.1:9430")
                .header("origin", "http://tauri.localhost")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("access-control-allow-origin").map(|v| v.to_str().unwrap()),
        Some("http://tauri.localhost")
    );
}

#[tokio::test]
async fn a_preflight_from_the_tauri_origin_is_answered() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri(routes::DEVICES)
                .header("host", "127.0.0.1:9430")
                .header("origin", "http://tauri.localhost")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.headers().contains_key("access-control-allow-methods"));
}

#[tokio::test]
async fn a_preflight_from_an_unknown_origin_is_refused() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri(routes::DEVICES)
                .header("host", "127.0.0.1:9430")
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_fps_outside_the_range_is_refused_with_422() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request_with_body("PATCH", routes::RENDER, r#"{"fps":5}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::VALIDATION);
}

#[tokio::test]
async fn the_fps_changes_live() {
    let (router, app, _home) = build();
    let response = router
        .oneshot(request_with_body("PATCH", routes::RENDER, r#"{"fps":20}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert_eq!(body.get("fps").and_then(|v| v.as_u64()), Some(20));

    // The engine was told, without a restart.
    let engine = app.engine.clone();
    assert_eq!(engine.stats().fps_measured, 0.0);
    // The setting was persisted in the store.
    let fps = app.store.read().await.config.settings.fps;
    assert_eq!(fps, 20);
}

#[tokio::test]
async fn the_scale_and_sampling_round_trip_through_the_active_layout() {
    let (router, _app, _home) = build();
    // Set the scale and the sampling.
    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            routes::RENDER,
            r#"{"scale":50,"sampling":"bilinear"}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert_eq!(body.get("scale").and_then(|v| v.as_u64()), Some(50));
    assert_eq!(body.get("sampling").and_then(|v| v.as_str()), Some("bilinear"));

    // Read them back.
    let response = router
        .oneshot(request("GET", routes::RENDER))
        .await
        .expect("the router answers");
    let body = json(response).await;
    assert_eq!(body.get("scale").and_then(|v| v.as_u64()), Some(50));
    assert_eq!(body.get("sampling").and_then(|v| v.as_str()), Some("bilinear"));
}

#[tokio::test]
async fn the_output_can_be_paused_and_resumed() {
    let (router, _app, _home) = build();
    let response = router
        .clone()
        .oneshot(request_with_body("PATCH", routes::OUTPUT, r#"{"power":"paused"}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert_eq!(body.get("power").and_then(|v| v.as_str()), Some("paused"));

    let response = router
        .oneshot(request_with_body("PATCH", routes::OUTPUT, r#"{"power":"running"}"#))
        .await
        .expect("the router answers");
    let body = json(response).await;
    assert_eq!(body.get("power").and_then(|v| v.as_str()), Some("running"));
}

#[tokio::test]
async fn the_fans_are_unavailable_when_the_service_is_down() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", routes::FANS))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn a_fan_value_below_the_floor_is_refused_with_422() {
    let (router, _app, _home) = build();
    // The service is down, but the validation happens first, so the answer is 422 and
    // not 503.
    let response = router
        .oneshot(request_with_body(
            "PATCH",
            "/api/v1/fans/headers/sys_fan1",
            r#"{"mode":"fixed","percent":5}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    let error: ApiError = serde_json::from_value(body).expect("the body is an ApiError");
    assert_eq!(error.error, canvas_proto::api::error_codes::VALIDATION);
    assert!(error.message.contains("floor"), "the message says why: {}", error.message);
}

#[tokio::test]
async fn the_cpu_fan_never_goes_below_30_percent() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request_with_body(
            "PATCH",
            "/api/v1/fans/headers/cpu_fan",
            r#"{"mode":"fixed","percent":25}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_effects_are_listed() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", routes::EFFECTS))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let effects = body.get("effects").and_then(|v| v.as_array()).expect("the effects list");
    let ids: Vec<&str> =
        effects.iter().filter_map(|e| e.get("id").and_then(|v| v.as_str())).collect();
    // Exactly these: the 11 built-ins, then the 3 stock paint effects that ship in the daemon.
    let built_ins: Vec<String> = canvas_effects::registry().into_iter().map(|make| make().schema().id).collect();
    assert_eq!(built_ins.len(), 11, "there are 11 built-in effects");
    let stock = ["html:ink-drop", "html:palette-knife", "html:wet-smear"];
    let mut expected: Vec<&str> = built_ins.iter().map(|s| s.as_str()).chain(stock).collect();
    let mut got = ids.clone();
    expected.sort_unstable();
    got.sort_unstable();
    assert_eq!(got, expected, "the list is the built-ins plus the stock effects, nothing else");
}

/// The app with a user-effects folder holding one HTML effect. The builder is a fake, so
/// the canvas-html stub is never called.
fn build_with_user_effect() -> (axum::Router, Arc<App>, TempHome, std::path::PathBuf) {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    let fake: canvas_daemon::user_effects::Builder =
        Arc::new(|id: &str, _source: &str| {
            Ok(Box::new(common::FakeEffect::new(id)) as Box<dyn canvas_proto::traits::Effect>)
        });
    app.user_effects = Arc::new(canvas_daemon::user_effects::UserEffects::new(fake));

    let folder = home.path.join("effects");
    std::fs::create_dir_all(&folder).expect("the effects folder is created");
    std::fs::write(folder.join("fancy.html"), "Fancy\n<canvas></canvas>")
        .expect("the effect file is written");
    app.user_effects.rescan(&folder);

    let app = Arc::new(app);
    (api::router(app.clone()), app, home, folder)
}

#[tokio::test]
async fn user_html_effects_are_listed_after_the_built_ins() {
    let (router, _app, _home, _folder) = build_with_user_effect();
    let response = router
        .oneshot(request("GET", routes::EFFECTS))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let ids: Vec<String> = body
        .get("effects")
        .and_then(|v| v.as_array())
        .expect("the effects list")
        .iter()
        .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    let built_ins: Vec<String> =
        canvas_effects::registry().iter().map(|f| f().schema().id).collect();
    assert_eq!(ids.len(), built_ins.len() + 1);
    for (i, id) in built_ins.iter().enumerate() {
        assert_eq!(&ids[i], id, "built-in {i} comes first");
    }
    assert_eq!(ids[built_ins.len()], "html:fancy");
}

#[tokio::test]
async fn a_user_html_effect_can_be_detailed_and_activated() {
    let (router, _app, _home, _folder) = build_with_user_effect();
    let response = router
        .clone()
        .oneshot(request("GET", "/api/v1/effects/html:fancy"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .oneshot(request_with_body("POST", "/api/v1/effects/html:fancy/activate", "{}"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert_eq!(body.get("id").and_then(|v| v.as_str()), Some("html:fancy"));
}

/// The body of a response, as bytes.
async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is read")
        .to_vec()
}

/// Checks the header of a thumbnail body and returns its length.
fn assert_thumbnail_format(bytes: &[u8]) {
    use canvas_daemon::thumbnail::{FRAMES, HEIGHT, WIDTH};
    assert!(bytes.len() >= 6, "the header is there");
    assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), WIDTH);
    assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), HEIGHT);
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), FRAMES);
    let expected = 6 + FRAMES as usize * WIDTH as usize * HEIGHT as usize * 3;
    assert_eq!(bytes.len(), expected, "the body is the header plus every frame");
}

#[tokio::test]
async fn the_thumbnail_of_solid_has_the_binary_format() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", "/api/v1/effects/solid/thumbnail"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/octet-stream")
    );
    assert_thumbnail_format(&body_bytes(response).await);
}

#[tokio::test]
async fn the_thumbnail_of_aurora_has_the_binary_format() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", "/api/v1/effects/aurora/thumbnail"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = body_bytes(response).await;
    assert_thumbnail_format(&bytes);
    // Aurora is not a flat colour after a second of warm-up.
    let first = &bytes[6..6 + 3];
    assert!(bytes[6..].chunks(3).any(|px| px != first), "the frames are not all one colour");
}

#[tokio::test]
async fn the_thumbnail_is_served_identically_from_the_cache() {
    let (router, app, _home) = build();
    let first = router
        .clone()
        .oneshot(request("GET", "/api/v1/effects/solid/thumbnail"))
        .await
        .expect("the router answers");
    let first = body_bytes(first).await;
    assert_eq!(app.thumbnails.len(), 1, "the render is cached");

    let second = router
        .oneshot(request("GET", "/api/v1/effects/solid/thumbnail"))
        .await
        .expect("the router answers");
    assert_eq!(body_bytes(second).await, first, "the cached bytes are identical");
}

#[tokio::test]
async fn the_thumbnail_of_an_unknown_effect_is_a_404() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", "/api/v1/effects/not-an-effect/thumbnail"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// The app with a user-effects folder holding the named HTML effects, built by the fake
/// builder the test gives.
fn build_with_user_effects(
    names: &[&str],
    builder: canvas_daemon::user_effects::Builder,
) -> (axum::Router, Arc<App>, TempHome) {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    app.user_effects = Arc::new(canvas_daemon::user_effects::UserEffects::new(builder));
    let folder = home.path.join("effects");
    std::fs::create_dir_all(&folder).expect("the effects folder is created");
    for name in names {
        std::fs::write(folder.join(format!("{name}.html")), format!("{name}\n<canvas></canvas>"))
            .expect("the effect file is written");
    }
    app.user_effects.rescan(&folder);
    let app = Arc::new(app);
    (api::router(app.clone()), app, home)
}

/// A slow effect that counts the renders running at the same time.
struct SlowEffect {
    running: Arc<std::sync::atomic::AtomicUsize>,
    peak: Arc<std::sync::atomic::AtomicUsize>,
}

impl Effect for SlowEffect {
    fn schema(&self) -> canvas_proto::props::PropertySchema {
        canvas_proto::props::PropertySchema {
            id: "slow".into(),
            name: "Slow".into(),
            description: String::new(),
            needs: Default::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }
    fn init(&mut self, _props: &canvas_proto::props::Props) {}
    fn update(
        &mut self,
        _dt: f32,
        _canvas: &mut canvas_proto::traits::Canvas,
        _inputs: &canvas_proto::traits::Inputs,
    ) {
        // Every update is slow, so a render takes a while.
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
}

impl Drop for SlowEffect {
    fn drop(&mut self) {
        // The effect lives as long as its render, so the count is the number of renders
        // running at the same time.
        let now = self.running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        self.peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The render cap holds even when a client goes away.
///
/// The permit belongs to the render, not to the request: a client that disconnects while
/// its tile is rendering must not hand a slot back, or more renders run at once than the
/// cap allows.
#[tokio::test]
async fn at_most_two_tiles_render_at_once_even_when_a_client_goes_away() {
    let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let builder: canvas_daemon::user_effects::Builder = {
        let running = running.clone();
        let peak = peak.clone();
        Arc::new(move |_id: &str, _source: &str| {
            let now = running.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::new(SlowEffect { running: running.clone(), peak: peak.clone() })
                as Box<dyn canvas_proto::traits::Effect>)
        })
    };
    let names = ["slow-0", "slow-1", "slow-2", "slow-3"];
    let (router, _app, _home) = build_with_user_effects(&names, builder);

    // The first client goes away while its tile is rendering.
    let abandoned = router
        .clone()
        .oneshot(request("GET", "/api/v1/effects/html:slow-0/thumbnail"));
    let _ = tokio::time::timeout(std::time::Duration::from_millis(40), abandoned).await;

    // Three more ask for a tile at the same time.
    let mut waiting = Vec::new();
    for name in &names[1..] {
        let router = router.clone();
        let path = format!("/api/v1/effects/html:{name}/thumbnail");
        waiting.push(tokio::spawn(async move {
            let _ = router.oneshot(request("GET", &path)).await;
        }));
    }
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    let peak = peak.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        peak <= 2,
        "{peak} tiles rendered at once; the cap is {}",
        canvas_daemon::thumbnail::CONCURRENT_RENDERS
    );
}

/// A tile that cannot be rendered is remembered, so the library does not ask for it again
/// on every open, and the answer says the tile was given up.
#[tokio::test]
async fn a_tile_that_cannot_be_rendered_says_so_and_is_not_rendered_again() {
    let builds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let builder: canvas_daemon::user_effects::Builder = {
        let builds = builds.clone();
        Arc::new(move |_id: &str, _source: &str| {
            let count = builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if count > 1 {
                // The effect built when the folder was scanned, but it cannot be built
                // again, so the tile render fails.
                return Err("the effect could not be built".to_string());
            }
            Ok(Box::new(common::FakeEffect::new("broken")) as Box<dyn canvas_proto::traits::Effect>)
        })
    };
    let (router, _app, _home) = build_with_user_effects(&["broken"], builder);

    let response = router
        .clone()
        .oneshot(request("GET", "/api/v1/effects/html:broken/thumbnail"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json(response).await;
    let message = body.get("message").and_then(|m| m.as_str()).unwrap_or_default();
    assert!(message.contains("given up"), "the answer does not say the tile was given up: {message}");

    // The second request is answered from the remembered failure: the effect is not built
    // again, so the library does not pay for the render on every open.
    let after_the_first = builds.load(std::sync::atomic::Ordering::SeqCst);
    let response = router
        .oneshot(request("GET", "/api/v1/effects/html:broken/thumbnail"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        builds.load(std::sync::atomic::Ordering::SeqCst),
        after_the_first,
        "the effect was built again for the second request"
    );
}

/// A fake service. It answers every request the same way.
struct FakePipe {
    reply: FakeReply,
}

#[derive(Clone, Copy)]
enum FakeReply {
    Ok,
    Started(&'static str),
    Error(&'static str),
    Down,
    /// The pipe opened and then broke: the request may have been acted on.
    Broke,
}

#[async_trait::async_trait]
impl canvas_daemon::pipe_client::PipeLink for FakePipe {
    async fn exchange(
        &self,
        _request: &canvas_proto::pipe::PipeRequest,
    ) -> Result<canvas_proto::pipe::PipeResponse, canvas_daemon::pipe_client::PipeError> {
        match self.reply {
            FakeReply::Ok => Ok(canvas_proto::pipe::PipeResponse::Ok),
            FakeReply::Started(version) => Ok(canvas_proto::pipe::PipeResponse::UpdateStarted {
                version: version.to_string(),
            }),
            FakeReply::Error(code) => Ok(canvas_proto::pipe::PipeResponse::Error {
                code: code.to_string(),
                message: format!("the service said {code}"),
            }),
            FakeReply::Down => Err(canvas_daemon::pipe_client::PipeError::Io(
                std::io::Error::new(std::io::ErrorKind::NotFound, "the pipe is not up"),
            )),
            FakeReply::Broke => Err(canvas_daemon::pipe_client::PipeError::Io(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the pipe broke"),
            )),
        }
    }
}

/// The app with a fake service behind it.
fn build_with_pipe(reply: FakeReply) -> (axum::Router, Arc<App>, TempHome) {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    app.pipe = Arc::new(FakePipe { reply });
    let app = Arc::new(app);
    (api::router(app.clone()), app, home)
}

#[tokio::test]
async fn deleting_a_fan_profile_the_service_removes_is_200() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Ok);
    let response = router
        .oneshot(request("DELETE", "/api/v1/fans/profiles/quiet"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn deleting_the_active_or_last_fan_profile_is_a_409() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Error("conflict"));
    let response = router
        .oneshot(request("DELETE", "/api/v1/fans/profiles/balanced"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = json(response).await;
    assert_eq!(body.get("error").and_then(|v| v.as_str()), Some("conflict"));
}

#[tokio::test]
async fn deleting_an_unknown_fan_profile_is_a_404() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Error("not_found"));
    let response = router
        .oneshot(request("DELETE", "/api/v1/fans/profiles/nope"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_a_fan_profile_with_the_service_down_is_a_503() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Down);
    let response = router
        .oneshot(request("DELETE", "/api/v1/fans/profiles/quiet"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

fn install_body(installer: &str, signature: &str) -> String {
    serde_json::json!({ "installer": installer, "signature": signature }).to_string()
}

#[tokio::test]
async fn an_update_the_service_starts_is_a_202_with_the_version() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Started("0.2.1"));
    let response = router
        .oneshot(request_with_body("POST", routes::UPDATE_INSTALL, &install_body(r"C:\x\setup.exe", "c2ln")))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(json(response).await.get("version").and_then(|v| v.as_str()), Some("0.2.1"));
}

#[tokio::test]
async fn an_update_the_service_refuses_is_a_422_and_the_message_comes_through() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Error("validation"));
    let response = router
        .oneshot(request_with_body("POST", routes::UPDATE_INSTALL, &install_body(r"C:\x\setup.exe", "c2ln")))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    assert_eq!(body.get("error").and_then(|v| v.as_str()), Some("validation"));
    assert_eq!(body.get("message").and_then(|v| v.as_str()), Some("the service said validation"));
}

#[tokio::test]
async fn an_update_while_one_runs_is_a_409() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Error("conflict"));
    let response = router
        .oneshot(request_with_body("POST", routes::UPDATE_INSTALL, &install_body(r"C:\x\setup.exe", "c2ln")))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn an_update_with_the_service_down_is_a_503() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Down);
    let response = router
        .oneshot(request_with_body("POST", routes::UPDATE_INSTALL, &install_body(r"C:\x\setup.exe", "c2ln")))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json(response).await.get("error").and_then(|v| v.as_str()), Some("service_unavailable"));
}

/// The tray falls back to the permission prompt only on 503. Every other answer below must be a
/// status it does not fall back on.
#[tokio::test]
async fn only_an_absent_pipe_is_a_503_for_an_update() {
    let cases: [(FakeReply, StatusCode); 6] = [
        (FakeReply::Down, StatusCode::SERVICE_UNAVAILABLE),
        (FakeReply::Broke, StatusCode::BAD_GATEWAY),
        (FakeReply::Error("unauthorized"), StatusCode::FORBIDDEN),
        (FakeReply::Error("internal"), StatusCode::INTERNAL_SERVER_ERROR),
        (FakeReply::Error("service_unavailable"), StatusCode::INTERNAL_SERVER_ERROR),
        (FakeReply::Error("something_new"), StatusCode::INTERNAL_SERVER_ERROR),
    ];
    for (reply, want) in cases {
        let (router, _app, _home) = build_with_pipe(reply);
        let response = router
            .oneshot(request_with_body("POST", routes::UPDATE_INSTALL, &install_body(r"C:\x\setup.exe", "c2ln")))
            .await
            .expect("the router answers");
        assert_eq!(response.status(), want);
    }
}

#[tokio::test]
async fn an_answer_that_is_not_for_an_update_is_a_502() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Ok);
    let response = router
        .oneshot(request_with_body("POST", routes::UPDATE_INSTALL, &install_body(r"C:\x\setup.exe", "c2ln")))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn a_bad_update_body_is_a_422_before_the_service_hears_of_it() {
    // The fake would answer 202; a 422 proves the daemon refused first.
    for body in [
        install_body("setup.exe", "c2ln"),
        install_body(r"C:\x\setup.exe", ""),
        install_body(r"C:\x\setup.exe", &"a".repeat(4097)),
    ] {
        let (router, _app, _home) = build_with_pipe(FakeReply::Started("0.2.1"));
        let response = router
            .oneshot(request_with_body("POST", routes::UPDATE_INSTALL, &body))
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn the_update_route_needs_the_token() {
    let (router, _app, _home) = build_with_pipe(FakeReply::Started("0.2.1"));
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(routes::UPDATE_INSTALL)
                .header("host", "127.0.0.1:9430")
                .header("content-type", "application/json")
                .body(Body::from(install_body(r"C:\x\setup.exe", "c2ln")))
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unknown_effect_is_a_404() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", "/api/v1/effects/not-an-effect"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_layouts_are_listed_with_an_active_one() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", routes::LAYOUTS))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert!(body.get("active").and_then(|v| v.as_str()).is_some());
    let layouts = body.get("layouts").and_then(|v| v.as_array()).expect("the layout list");
    assert!(!layouts.is_empty());
}

#[tokio::test]
async fn the_devices_are_listed() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", routes::DEVICES))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert!(body.get("devices").and_then(|v| v.as_array()).is_some());
}

#[tokio::test]
async fn the_settings_round_trip() {
    let (router, _app, _home) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "PUT",
            routes::SETTINGS,
            r#"{"fps":25,"black_on_lock":true,"on_quit":"leave_last_frame","boot_color":null,"brightness":80,"power":"running"}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert_eq!(body.get("fps").and_then(|v| v.as_u64()), Some(25));
    assert_eq!(body.get("black_on_lock").and_then(|v| v.as_bool()), Some(true));

    let response = router
        .oneshot(request("GET", routes::SETTINGS))
        .await
        .expect("the router answers");
    let body = json(response).await;
    assert_eq!(body.get("fps").and_then(|v| v.as_u64()), Some(25));
    assert_eq!(body.get("brightness").and_then(|v| v.as_u64()), Some(80));
}

#[tokio::test]
async fn the_settings_refuse_an_out_of_range_fps() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request_with_body(
            "PUT",
            routes::SETTINGS,
            r#"{"fps":200,"black_on_lock":false,"on_quit":"black","boot_color":null,"brightness":100,"power":"running"}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_zone_panel_is_dismissed() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("POST", routes::DISMISS_ZONE_PANEL))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn onboarding_complete_is_saved_and_reported_in_status() {
    let (router, app, _home) = build();
    // A fresh install is not complete.
    let status = json(
        router.clone().oneshot(request("GET", routes::STATUS)).await.expect("the router answers"),
    )
    .await;
    assert_eq!(status.get("onboarding_complete").and_then(|v| v.as_bool()), Some(false));
    assert!(!app.store.read().await.config.onboarding_complete);

    // The route needs the token.
    let refused = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(routes::ONBOARDING_COMPLETE)
                .header("host", "127.0.0.1:9430")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(!app.store.read().await.config.onboarding_complete);

    let response = router
        .clone()
        .oneshot(request("POST", routes::ONBOARDING_COMPLETE))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(app.store.read().await.config.onboarding_complete);

    let status = json(
        router.oneshot(request("GET", routes::STATUS)).await.expect("the router answers"),
    )
    .await;
    assert_eq!(status.get("onboarding_complete").and_then(|v| v.as_bool()), Some(true));

    // It was written to disk, not only held in memory.
    let reloaded = canvas_daemon::storage::Store::load_from(&_home.path);
    assert!(reloaded.config.onboarding_complete, "the flag survives a reload");
}

#[tokio::test]
async fn the_onboarding_effects_are_hidden_from_the_list_but_activate() {
    let (router, app, _home) = build();
    let list = json(
        router.clone().oneshot(request("GET", routes::EFFECTS)).await.expect("the router answers"),
    )
    .await;
    let ids: Vec<&str> = list
        .get("effects")
        .and_then(|v| v.as_array())
        .expect("the effects list")
        .iter()
        .filter_map(|e| e.get("id").and_then(|v| v.as_str()))
        .collect();
    assert!(!ids.contains(&"onboard-hello") && !ids.contains(&"onboard-sweep"));

    for id in ["onboard-hello", "onboard-sweep"] {
        let path = routes::EFFECT_ACTIVATE.replace("{id}", id);
        let response = router
            .clone()
            .oneshot(request_with_body("POST", &path, "{}"))
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::OK, "{id} activates");
        // The onboarding effects are transient: the layout keeps its own effect.
        let store = app.store.read().await;
        let active = store.active_layout().and_then(|l| l.effect.as_ref().map(|e| e.id.clone()));
        assert_eq!(active, None, "{id} is never saved as the layout's effect");
    }
}

/// An app whose fake engine the test can read back.
fn build_with_engine() -> (axum::Router, Arc<App>, Arc<FakeEngine>, TempHome) {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    (api::router(app.clone()), app, engine, home)
}

async fn engine_effect(engine: &FakeEngine) -> Option<String> {
    engine.effect_id.lock().unwrap().clone()
}

#[tokio::test]
async fn an_onboarding_effect_restores_the_previous_effect_and_is_never_saved() {
    let (router, app, engine, home) = build_with_engine();
    // Waits run 100 times faster: hello's 2 s + 8 s grace is 100 ms.
    app.transient.set_time_scale(0.01);

    // The owner's effect.
    let path = routes::EFFECT_ACTIVATE.replace("{id}", "rainbow_wave");
    let r = router.clone().oneshot(request_with_body("POST", &path, "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(engine_effect(&engine).await.as_deref(), Some("rainbow_wave"));

    // Hello plays: it runs on the engine, but the layout and the files keep rainbow-wave.
    let path = routes::EFFECT_ACTIVATE.replace("{id}", "onboard-hello");
    let r = router.clone().oneshot(request_with_body("POST", &path, "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(engine_effect(&engine).await.as_deref(), Some("onboard-hello"));
    assert!(app.transient.is_pending());
    {
        let store = app.store.read().await;
        let saved = store.active_layout().and_then(|l| l.effect.as_ref().map(|e| e.id.clone()));
        assert_eq!(saved.as_deref(), Some("rainbow_wave"));
        assert!(!store.effects.saved.contains_key("onboard-hello"));
    }
    let on_disk = std::fs::read_to_string(home.path.join("layouts.json")).unwrap_or_default()
        + &std::fs::read_to_string(home.path.join("effects.json")).unwrap_or_default();
    assert!(!on_disk.contains("onboard-"), "no onboard-* id reaches the files");

    // A second activation (the sweep) restarts the wait: hello's timer is outdated.
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    let path = routes::EFFECT_ACTIVATE.replace("{id}", "onboard-sweep");
    let r = router.clone().oneshot(request_with_body("POST", &path, "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(70)).await;
    assert_eq!(
        engine_effect(&engine).await.as_deref(),
        Some("onboard-sweep"),
        "hello's old timer (100 ms) must not fire at 130 ms: the sweep re-armed it to 105 ms from 60 ms"
    );

    // Silence: the sweep (25 ms) + 80 ms grace after its start, the previous effect returns.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert_eq!(engine_effect(&engine).await.as_deref(), Some("rainbow_wave"));
    assert!(!app.transient.is_pending());
}

#[tokio::test]
async fn the_onboarding_restore_waits_while_a_session_is_active() {
    let (router, app, engine, _home) = build_with_engine();
    app.transient.set_time_scale(0.01);
    let path = routes::EFFECT_ACTIVATE.replace("{id}", "rainbow_wave");
    router.clone().oneshot(request_with_body("POST", &path, "{}")).await.unwrap();
    let r = router
        .clone()
        .oneshot(request_with_body("POST", routes::ONBOARDING_SESSION, r#"{"active":true}"#))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(app.identify.session_active());
    let path = routes::EFFECT_ACTIVATE.replace("{id}", "onboard-hello");
    router.clone().oneshot(request_with_body("POST", &path, "{}")).await.unwrap();
    // The timer (100 ms) fires and finds the session: no restore.
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    assert_eq!(engine_effect(&engine).await.as_deref(), Some("onboard-hello"));
    // The session ends: the next check restores the saved effect.
    let r = router
        .clone()
        .oneshot(request_with_body("POST", routes::ONBOARDING_SESSION, r#"{"active":false}"#))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(!app.identify.session_active());
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert_eq!(engine_effect(&engine).await.as_deref(), Some("rainbow_wave"));
}

#[tokio::test]
async fn activating_a_normal_effect_cancels_the_onboarding_restore() {
    let (router, app, engine, _home) = build_with_engine();
    app.transient.set_time_scale(0.01);
    for id in ["onboard-hello", "solid"] {
        let path = routes::EFFECT_ACTIVATE.replace("{id}", id);
        let r = router.clone().oneshot(request_with_body("POST", &path, "{}")).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(engine_effect(&engine).await.as_deref(), Some("solid"), "the owner's choice stays");
}

#[tokio::test]
async fn the_config_is_exported_and_imported() {
    let (router, _app, _home) = build();
    let response = router
        .clone()
        .oneshot(request("GET", routes::CONFIG_EXPORT))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let bundle = json(response).await;
    assert!(bundle.get("schema_version").is_some());
    assert!(bundle.get("layouts").is_some());
    assert!(bundle.get("zones").is_some());
    assert!(bundle.get("effects").is_some());
    assert!(bundle.get("fans").is_some());

    // The token is never in the bundle.
    let text = serde_json::to_string(&bundle).unwrap();
    assert!(!text.contains(TOKEN), "the bundle never carries the API token");

    // Import it back.
    let response = router
        .oneshot(request_with_body("POST", routes::CONFIG_IMPORT, &text))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_import_with_a_fan_value_below_the_floor_is_refused() {
    let (router, _app, _home) = build();
    let bad = serde_json::json!({
        "schema_version": 1,
        "exported_at": "2026-10-26T00:00:00Z",
        "app_version": "0.1.0",
        "settings": {
            "fps": 30, "black_on_lock": false, "on_quit": "black",
            "boot_color": null, "brightness": 100, "power": "running"
        },
        "layouts": { "schema_version": 1, "active": "default", "layouts": [] },
        "zones": { "schema_version": 1, "sizes": {} },
        "effects": { "schema_version": 1, "saved": {}, "presets": {}, "favorites": [] },
        "fans": {
            "schema_version": 1,
            "floor_percent": 20,
            "active_profile": "Default",
            "profiles": [{
                "name": "Default",
                "headers": { "sys_fan1": { "mode": "fixed", "percent": 5 } }
            }],
            "boot_color": null
        }
    });
    let response = router
        .oneshot(request_with_body("POST", routes::CONFIG_IMPORT, &bad.to_string()))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_readback_lists_the_devices() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", routes::DIAG_READBACK))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert!(body.get("devices").and_then(|v| v.as_array()).is_some());
}

/// The headers a browser sends with a WebSocket handshake.
const UPGRADE_HEADERS: &str = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==";

/// Starts the router on a loopback port and returns its address.
async fn serve(router: axum::Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("the port is free");
    let addr = listener.local_addr().expect("the address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    addr
}

/// Sends a raw HTTP request over a socket and returns the answer's status line.
///
/// The request names `127.0.0.1:9430` as its host, because that is the host the daemon's
/// guard allows, whatever port the test server listens on.
async fn raw_get(addr: std::net::SocketAddr, target: &str, extra: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("the socket opens");
    let request = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:9430\r\n{extra}\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("the request is written");
    let mut buf = vec![0u8; 512];
    let n = stream.read(&mut buf).await.expect("the answer is read");
    String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string()
}

/// A WebSocket upgrade for the preview route. A browser cannot put a header on the handshake,
/// so the token rides in the query string.
fn preview_upgrade(query: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("{}{}", routes::PREVIEW, query))
        .header("host", "127.0.0.1:9430")
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .expect("the request builds")
}

#[tokio::test]
async fn the_preview_upgrade_with_only_a_query_token_passes_the_guard() {
    let (router, _app, _home) = build();
    // No Authorization header: the browser cannot send one on a WebSocket handshake. The
    // guard must let the handshake through, and the route checks the token in the query.
    let response = router
        .oneshot(preview_upgrade(&format!("?token={TOKEN}")))
        .await
        .expect("the router answers");
    assert_ne!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the guard must not refuse the preview handshake"
    );
    // The route accepted the token and reached the upgrade itself. A unit test has no
    // hyper upgrade state, so the upgrade cannot happen here (426), but the answer is the
    // route's, not the guard's refusal.
    let body = json(response).await;
    assert!(
        body.get("error").is_none(),
        "the answer is not the guard's refusal: {body}"
    );
}

#[tokio::test]
async fn the_preview_socket_opens_with_only_a_query_token() {
    let (router, _app, _home) = build();
    let addr = serve(router).await;
    // The whole point of F1: a browser sends the token in the query and nothing else, and
    // the socket opens.
    let status = raw_get(addr, &format!("{}?token={TOKEN}", routes::PREVIEW), UPGRADE_HEADERS).await;
    assert!(status.contains("101"), "the socket opened: {status}");
}

#[tokio::test]
async fn the_preview_socket_without_a_token_is_refused() {
    let (router, _app, _home) = build();
    let addr = serve(router).await;
    let status = raw_get(addr, routes::PREVIEW, UPGRADE_HEADERS).await;
    assert!(status.contains("401"), "the handshake was refused: {status}");
}

#[tokio::test]
async fn the_preview_socket_with_a_wrong_query_token_is_refused() {
    let (router, _app, _home) = build();
    let addr = serve(router).await;
    let status = raw_get(addr, &format!("{}?token=not-the-token", routes::PREVIEW), UPGRADE_HEADERS).await;
    assert!(status.contains("401"), "the handshake was refused: {status}");
}

#[tokio::test]
async fn a_refused_request_from_the_tauri_origin_carries_the_cors_headers() {
    let (router, _app, _home) = build();
    // No token: the guard refuses it. A browser cannot read a 401 that comes back without
    // the CORS headers, and the editor would say "the daemon did not answer".
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::DEVICES)
                .header("host", "127.0.0.1:9430")
                .header("origin", "http://tauri.localhost")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers().get("access-control-allow-origin").map(|v| v.to_str().unwrap()),
        Some("http://tauri.localhost"),
        "the browser must be able to read the refusal"
    );
}

#[tokio::test]
async fn a_refused_request_from_an_unknown_origin_carries_no_cors_headers() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::DEVICES)
                .header("host", "127.0.0.1:9430")
                .header("origin", "https://evil.example")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        !response.headers().contains_key("access-control-allow-origin"),
        "an unknown origin gets no CORS headers"
    );
}

// ---------------------------------------------------------------- numeric displays

use canvas_daemon::displays;
use canvas_proto::device::DeviceId;
use canvas_proto::display::{DisplayConfig, DisplaySource, DisplayUnit};
use canvas_proto::fans::{FanHeaderStatus, FanMode, FansStatus, TempReading, TempSource};
use common::{chione_info, DisplayLog, FakeDisplay};
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

/// The CHIONE's id, as the spine names it.
const ID: &str = "chione:1b80:b53c";

/// The display route for the CHIONE.
fn display_path() -> String {
    routes::DISPLAY.replace("{id}", ID)
}

/// The value route for the CHIONE.
fn value_path() -> String {
    routes::DISPLAY_VALUE.replace("{id}", ID)
}

/// Builds the app with a fake numeric display. The fake's log is returned for the assertions.
fn build_with_display(found: bool) -> (axum::Router, Arc<App>, TempHome, Arc<StdMutex<DisplayLog>>) {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let log = Arc::new(StdMutex::new(DisplayLog {
        found: if found { vec![chione_info()] } else { Vec::new() },
        ..DisplayLog::default()
    }));
    let app = Arc::new(App::with_displays(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
        Box::new(FakeDisplay(log.clone())),
    ));
    // The tests never look at the real machine: ZEUS CAST really runs on it.
    app.displays.set_vendor_check(Arc::new(|| false));
    (api::router(app.clone()), app, home, log)
}

/// A saved config for the CHIONE.
fn config(source: DisplaySource, unit: DisplayUnit, decimals: u8, blink_above: Option<f32>) -> DisplayConfig {
    DisplayConfig { device: DeviceId(ID.into()), source, unit, decimals, blink_above }
}

/// A `PUT` of a config to the CHIONE's route.
async fn put_config(router: &axum::Router, body: &DisplayConfig) -> axum::response::Response {
    let text = serde_json::to_string(body).expect("the config serializes");
    router
        .clone()
        .oneshot(request_with_body("PUT", &display_path(), &text))
        .await
        .expect("the router answers")
}

/// Fan status with temperatures and fan headers.
fn fans(temps: Vec<(TempSource, f32)>, headers: Vec<(&str, u32)>) -> FansStatus {
    FansStatus {
        chip: Some("NCT6687D-R @ 0xA20".into()),
        hardware_watchdog: None,
        headers: headers
            .into_iter()
            .map(|(id, rpm)| FanHeaderStatus {
                id: id.to_string(),
                label: id.to_string(),
                rpm: Some(rpm),
                pwm_percent: None,
                mode: FanMode::Bios,
                on_bios: true,
                override_reason: None,
            })
            .collect(),
        temps: temps
            .into_iter()
            .map(|(source, c)| TempReading { label: String::new(), source, celsius: Some(c) })
            .collect(),
        active_profile: "Default".into(),
        profiles: vec!["Default".into()],
        floor_percent: 30,
    }
}

/// Runs one pass of the display task at `now`. Returns the wait it asked for.
async fn pass(app: &App, now: Instant) -> Duration {
    displays::tick(app, now).await
}

#[tokio::test]
async fn an_unconfigured_display_is_never_connected_or_written() {
    let (_router, app, _home, log) = build_with_display(true);
    let mut now = Instant::now();
    for _ in 0..3 {
        now += pass(&app, now).await;
    }
    let log = log.lock().unwrap();
    assert_eq!(log.connects, 0, "a display with no saved config is never connected");
    assert!(log.writes.is_empty(), "and never written");
    assert_eq!(log.discovers, 0, "the task does not even enumerate with nothing saved");
}

#[tokio::test]
async fn a_configured_display_that_is_not_plugged_in_is_not_opened() {
    let (router, app, _home, log) = build_with_display(false);
    let response = put_config(&router, &config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let wait = pass(&app, Instant::now()).await;
    assert_eq!(wait, displays::ABSENT_POLL, "a missing display is looked for again later");
    let log = log.lock().unwrap();
    assert_eq!(log.discovers, 1, "it is enumerated");
    assert_eq!(log.connects, 0, "and not opened");
    assert!(log.writes.is_empty());
}

#[tokio::test]
async fn a_saved_config_is_written_at_the_interval_with_the_value() {
    let (router, app, _home, log) = build_with_display(true);
    let response = put_config(&router, &config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 1, None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    *app.fans.write().await = Some(fans(vec![(TempSource::Cpu, 42.5)], vec![]));

    let now = Instant::now();
    let wait = pass(&app, now).await;
    assert_eq!(wait, Duration::from_millis(250));
    {
        let log = log.lock().unwrap();
        assert_eq!(log.connects, 1);
        assert_eq!(log.writes.len(), 1);
        let (device, value) = &log.writes[0];
        assert_eq!(device.0, ID);
        assert_eq!(value.value, Some(42.5));
        assert_eq!(value.decimals, 1);
        assert_eq!(value.unit, DisplayUnit::Celsius);
        assert!(!value.blink);
    }
    pass(&app, now + Duration::from_millis(250)).await;
    assert_eq!(log.lock().unwrap().writes.len(), 2, "the next write is one interval later");
}

#[tokio::test]
async fn fahrenheit_is_rounded_and_blinks_at_blink_above() {
    let (router, app, _home, log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::CpuTemp, DisplayUnit::Fahrenheit, 0, Some(100.0))).await;
    let now = Instant::now();
    *app.fans.write().await = Some(fans(vec![(TempSource::Cpu, 37.8)], vec![]));
    pass(&app, now).await;
    *app.fans.write().await = Some(fans(vec![(TempSource::Cpu, 37.4)], vec![]));
    pass(&app, now + Duration::from_millis(250)).await;

    let log = log.lock().unwrap();
    let (_, hot) = &log.writes[0];
    assert_eq!(hot.value, Some(100.0), "37.8 C is 100.04 F, shown as 100");
    assert!(hot.blink, "100 is at blink_above 100");
    assert_eq!(hot.unit, DisplayUnit::Fahrenheit);
    let (_, cool) = &log.writes[1];
    assert_eq!(cool.value, Some(99.0), "37.4 C is 99.32 F, shown as 99");
    assert!(!cool.blink);
}

#[tokio::test]
async fn the_api_source_writes_nothing_until_a_value_arrives_and_holds_it_after_null() {
    let (router, app, _home, log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::Api, DisplayUnit::Celsius, 1, None)).await;
    let mut now = Instant::now();
    now += pass(&app, now).await;
    {
        let log = log.lock().unwrap();
        assert_eq!(log.connects, 1, "the display is connected");
        assert!(log.writes.is_empty(), "no value yet, so no packet at all, blank or otherwise");
    }

    let set = |v: serde_json::Value| router.clone().oneshot(request_with_body("PUT", &value_path(), &v.to_string()));
    assert_eq!(set(serde_json::json!({ "value": 12.5 })).await.unwrap().status(), StatusCode::OK);
    pass(&app, now).await;
    assert_eq!(log.lock().unwrap().writes.last().unwrap().1.value, Some(12.5));

    let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(status[0]["shown"]["value"], serde_json::json!(12.5));
    assert_eq!(status[0]["info"]["connected"], serde_json::json!(true));

    // null is "no value": nothing is written and the display holds 12.5
    assert_eq!(set(serde_json::json!({ "value": null })).await.unwrap().status(), StatusCode::OK);
    now += Duration::from_millis(250);
    pass(&app, now).await;
    pass(&app, now + Duration::from_millis(250)).await;
    let log = log.lock().unwrap();
    assert_eq!(log.writes.len(), 1, "null writes no blank packet");
    assert!(log.writes.iter().all(|(_, v)| v.value.is_some()));
}

#[tokio::test]
async fn a_value_for_an_unknown_display_is_a_404() {
    let (router, _app, _home, _log) = build_with_display(true);
    let path = routes::DISPLAY_VALUE.replace("{id}", "chione:0000:0000");
    let response = router
        .clone()
        .oneshot(request_with_body("PUT", &path, r#"{"value": 1.0}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(json(response).await["error"], "not_found");
}

#[tokio::test]
async fn a_value_the_display_refuses_writes_nothing_and_is_logged_once() {
    let (router, app, _home, log) = build_with_display(true);
    log.lock().unwrap().refuse_at = Some(1000.0);
    put_config(&router, &config(DisplaySource::Fixed { value: 1234.0 }, DisplayUnit::Celsius, 0, Some(10.0))).await;
    let mut now = Instant::now();
    for _ in 0..3 {
        now += pass(&app, now).await;
    }
    {
        let log = log.lock().unwrap();
        assert!(log.writes.is_empty(), "a refusal writes nothing, not even a blank");
        assert_eq!(log.connects, 1, "a refusal keeps the connection");
        assert_eq!(log.disconnects, 0);
    }
    {
        let runtime = app.displays.runtime.read().await;
        assert_eq!(runtime.refusal_warnings, 1, "the refusal is logged once");
        assert!(runtime.shown.is_empty(), "the refused value, and its blink flag, are not recorded");
    }

    // A value that fits clears the streak, so the next refusal is logged again.
    put_config(&router, &config(DisplaySource::Fixed { value: 500.0 }, DisplayUnit::Celsius, 0, None)).await;
    pass(&app, now).await;
    assert_eq!(log.lock().unwrap().writes.last().unwrap().1.value, Some(500.0));
    put_config(&router, &config(DisplaySource::Fixed { value: 1234.0 }, DisplayUnit::Celsius, 0, None)).await;
    pass(&app, now + Duration::from_millis(500)).await;
    assert_eq!(app.displays.runtime.read().await.refusal_warnings, 2);
    assert_eq!(log.lock().unwrap().writes.len(), 1, "still only the accepted 500");
}

#[tokio::test]
async fn a_missing_sensor_reading_writes_nothing() {
    let (router, app, _home, log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 1, None)).await;
    // No fan status yet (the service is down): the CPU reading is missing.
    pass(&app, Instant::now()).await;
    assert!(log.lock().unwrap().writes.is_empty(), "no reading, no packet");
    assert_eq!(log.lock().unwrap().connects, 1);
    // A reading arrives and is written; it then goes missing again and the display is left alone.
    *app.fans.write().await = Some(fans(vec![(TempSource::Cpu, 40.0)], vec![]));
    pass(&app, Instant::now()).await;
    *app.fans.write().await = Some(fans(vec![], vec![]));
    pass(&app, Instant::now()).await;
    assert_eq!(log.lock().unwrap().writes.len(), 1);
}

#[tokio::test]
async fn a_fan_rpm_display_shows_the_header_speed() {
    let (router, app, _home, log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::FanRpm { header: "pump_fan".into() }, DisplayUnit::Celsius, 0, None)).await;
    *app.fans.write().await = Some(fans(vec![], vec![("pump_fan", 2400)]));
    pass(&app, Instant::now()).await;
    let log = log.lock().unwrap();
    let (_, value) = &log.writes[0];
    assert_eq!(value.value, Some(2400.0));
    assert_eq!(value.mode, canvas_proto::display::DisplayMode::FanRpm);
}

#[tokio::test]
async fn a_failed_connect_backs_off_and_the_wait_doubles() {
    let (router, app, _home, log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None)).await;
    log.lock().unwrap().fail_connect = 2;
    let t0 = Instant::now();
    assert_eq!(pass(&app, t0).await, Duration::from_secs(1), "first error: wait 1 s");
    assert_eq!(pass(&app, t0 + Duration::from_millis(500)).await, Duration::from_millis(500), "waits, no call");
    assert_eq!(log.lock().unwrap().connects, 1, "no connect while backing off");
    let t1 = t0 + Duration::from_secs(1);
    assert_eq!(pass(&app, t1).await, Duration::from_secs(2), "second error: wait doubles to 2 s");
    assert_eq!(log.lock().unwrap().connects, 2);
    let t2 = t1 + Duration::from_secs(2);
    assert_eq!(pass(&app, t2).await, Duration::from_millis(250), "connected and writing");
    let log = log.lock().unwrap();
    assert_eq!(log.connects, 3);
    assert_eq!(log.writes.len(), 1);
    assert_eq!(log.disconnects, 2, "each failure closes the device");
}

#[tokio::test]
async fn a_failed_write_drops_the_connection_and_retries_after_backoff() {
    let (router, app, _home, log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None)).await;
    log.lock().unwrap().fail_write = 1;
    let t0 = Instant::now();
    assert_eq!(pass(&app, t0).await, Duration::from_secs(1));
    {
        let log = log.lock().unwrap();
        assert!(!log.connected, "the device is closed after the failed write");
        assert!(log.writes.is_empty());
    }
    assert!(app.displays.runtime.read().await.shown.is_empty(), "nothing is shown while closed");
    assert_eq!(pass(&app, t0 + Duration::from_secs(1)).await, Duration::from_millis(250));
    assert_eq!(log.lock().unwrap().writes.len(), 1, "reconnected and written");
}

#[tokio::test]
async fn delete_stops_the_writes_and_closes_the_device() {
    let (router, app, _home, log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None)).await;
    let now = Instant::now();
    pass(&app, now).await;
    assert_eq!(log.lock().unwrap().writes.len(), 1);

    let response = router.clone().oneshot(request("DELETE", &display_path())).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    {
        let log = log.lock().unwrap();
        assert_eq!(log.disconnects, 1, "the device is closed");
        assert!(!log.connected);
    }
    let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    // The config is gone, so the display is listed again with the default config: it is there,
    // but it is not driven.
    assert_eq!(status.as_array().map(Vec::len), Some(1), "the display is listed again");
    assert_eq!(status[0]["config"]["source"]["kind"], serde_json::json!("cpu_temp"));
    assert_eq!(status[0]["shown"], serde_json::json!(null));
    assert_eq!(status[0]["info"]["connected"], serde_json::json!(false));

    let wait = pass(&app, now + Duration::from_millis(250)).await;
    assert_eq!(wait, displays::IDLE_POLL, "nothing to drive");
    {
        let log = log.lock().unwrap();
        assert_eq!(log.writes.len(), 1, "no write after the delete");
        assert_eq!(log.connects, 1, "and no reconnect");
    }

    let again = router.clone().oneshot(request("DELETE", &display_path())).await.unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_bad_config_is_refused_with_422_and_nothing_is_saved() {
    let (router, _app, home, _log) = build_with_display(true);
    let other_device = DisplayConfig {
        device: DeviceId("chione:0000:0000".into()),
        ..config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 1, None)
    };
    let refused = [
        other_device,
        config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 2, None),
        config(DisplaySource::FanRpm { header: "pump_fan".into() }, DisplayUnit::Celsius, 1, None),
    ];
    for body in &refused {
        let response = put_config(&router, body).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body:?}");
        assert_eq!(json(response).await["error"], "validation");
    }
    assert!(!home.path.join("displays.json").exists(), "a refused config writes nothing");
}

#[tokio::test]
async fn a_saved_config_is_written_to_displays_json_and_reloaded() {
    let (router, _app, home, _log) = build_with_display(true);
    let saved = config(DisplaySource::BoardTemp { sensor: "board_temp0".into() }, DisplayUnit::Fahrenheit, 1, Some(90.0));
    assert_eq!(put_config(&router, &saved).await.status(), StatusCode::OK);
    let file = home.path.join("displays.json");
    assert!(file.exists(), "displays.json is in the user directory");
    let reloaded = canvas_daemon::storage::Store::load_from(&home.path);
    assert_eq!(reloaded.displays.displays, vec![saved.clone()]);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"schema_version\": 1"), "{text}");
}

#[tokio::test]
async fn a_value_for_a_display_not_on_the_api_source_is_a_422() {
    let (router, _app, _home, _log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 1, None)).await;
    let response = router
        .clone()
        .oneshot(request_with_body("PUT", &value_path(), r#"{"value": 40.0}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json(response).await["error"], "validation");
}

// ---------------------------------------------------------------- an unconfigured display is listed

#[tokio::test]
async fn an_unconfigured_display_is_listed_but_never_opened() {
    let (router, app, _home, log) = build_with_display(true);
    let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(status.as_array().map(Vec::len), Some(1), "the display is listed");
    assert_eq!(status[0]["info"]["id"], serde_json::json!(ID));
    assert_eq!(status[0]["info"]["name"], serde_json::json!("GAMDIAS CHIONE Digital Display"));
    assert_eq!(status[0]["info"]["digits"], serde_json::json!(4));
    assert_eq!(status[0]["info"]["connected"], serde_json::json!(false), "it is never opened");
    // The default config: CPU temperature, Celsius, one decimal, no blink.
    assert_eq!(status[0]["config"]["device"], serde_json::json!(ID));
    assert_eq!(status[0]["config"]["source"]["kind"], serde_json::json!("cpu_temp"));
    assert_eq!(status[0]["config"]["unit"], serde_json::json!("celsius"));
    assert_eq!(status[0]["config"]["decimals"], serde_json::json!(1));
    assert_eq!(status[0]["config"]["blink_above"], serde_json::json!(null));
    assert_eq!(status[0]["shown"], serde_json::json!(null), "nothing is shown yet");

    // The task does not drive it: with no saved config nothing is opened or written.
    let mut now = Instant::now();
    for _ in 0..3 {
        now += pass(&app, now).await;
    }
    let log = log.lock().unwrap();
    assert_eq!(log.connects, 0, "a display with no saved config is never connected");
    assert!(log.writes.is_empty(), "and never written");
}

#[tokio::test]
async fn the_enumeration_is_cached_so_a_polling_page_does_not_enumerate_again() {
    let (router, _app, _home, log) = build_with_display(true);
    for _ in 0..3 {
        let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
        assert_eq!(status.as_array().map(Vec::len), Some(1), "the display is listed every time");
    }
    assert_eq!(log.lock().unwrap().discovers, 1, "the enumeration is cached");
}

#[tokio::test]
async fn configuring_a_listed_display_drives_it_and_deleting_it_lists_it_again() {
    let (router, app, _home, log) = build_with_display(true);
    let listed = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(listed[0]["config"]["source"]["kind"], serde_json::json!("cpu_temp"), "the default config");

    // Saving a config makes the task drive it.
    let saved = config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None);
    assert_eq!(put_config(&router, &saved).await.status(), StatusCode::OK);
    let now = Instant::now();
    pass(&app, now).await;
    {
        let log = log.lock().unwrap();
        assert_eq!(log.connects, 1, "the display is opened once it has a config");
        assert_eq!(log.writes.len(), 1);
    }
    let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(status.as_array().map(Vec::len), Some(1), "listed once, not twice");
    assert_eq!(status[0]["config"]["source"]["kind"], serde_json::json!("fixed"), "the saved config, not the default");
    assert_eq!(status[0]["info"]["connected"], serde_json::json!(true));

    // Deleting it: listed again with the default config, and no longer driven.
    assert_eq!(router.clone().oneshot(request("DELETE", &display_path())).await.unwrap().status(), StatusCode::OK);
    let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(status.as_array().map(Vec::len), Some(1), "listed again");
    assert_eq!(status[0]["config"]["source"]["kind"], serde_json::json!("cpu_temp"), "back to the default config");
    assert_eq!(status[0]["shown"], serde_json::json!(null));
    assert_eq!(status[0]["info"]["connected"], serde_json::json!(false), "and not connected");
    pass(&app, now + Duration::from_millis(250)).await;
    let log = log.lock().unwrap();
    assert_eq!(log.writes.len(), 1, "no write after the delete");
    assert_eq!(log.connects, 1, "and no reconnect");
}

// ---------------------------------------------------------------- F3, F5, F6

/// A display whose write waits for the test, to prove the routes answer while the device is busy.
struct SlowDisplay {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    connected: bool,
}

#[async_trait::async_trait]
impl canvas_proto::traits::NumericDisplayAdapter for SlowDisplay {
    fn name(&self) -> &'static str {
        "slow-display"
    }
    async fn connect(&mut self) -> Result<(), canvas_proto::traits::AdapterError> {
        self.connected = true;
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<(), canvas_proto::traits::AdapterError> {
        self.connected = false;
        Ok(())
    }
    fn is_connected(&self) -> bool {
        self.connected
    }
    async fn discover(&mut self) -> Result<Vec<canvas_proto::display::DisplayInfo>, canvas_proto::traits::AdapterError> {
        Ok(vec![chione_info()])
    }
    async fn write_value(
        &mut self,
        _device: &DeviceId,
        _value: &canvas_proto::display::DisplayValue,
    ) -> Result<(), canvas_proto::traits::AdapterError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(())
    }
    fn write_interval_ms(&self) -> u32 {
        250
    }
}

#[tokio::test]
async fn the_routes_answer_while_a_write_is_in_flight() {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.path);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let app = Arc::new(App::with_displays(
        store,
        Arc::new(FakeEngine::default()),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
        Box::new(SlowDisplay { started: started.clone(), release: release.clone(), connected: false }),
    ));
    app.displays.set_vendor_check(Arc::new(|| false));
    let router = api::router(app.clone());
    put_config(&router, &config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None)).await;

    let task = {
        let app = app.clone();
        tokio::spawn(async move { displays::tick(&app, Instant::now()).await })
    };
    started.notified().await; // the tick is inside write_value, holding the adapter

    let wait = Duration::from_secs(2);
    let get = tokio::time::timeout(wait, router.clone().oneshot(request("GET", routes::DISPLAYS)))
        .await
        .expect("GET /displays answers while the device is busy")
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let put = tokio::time::timeout(
        wait,
        router.clone().oneshot(request_with_body("PUT", &value_path(), r#"{"value": 1.0}"#)),
    )
    .await
    .expect("PUT /value answers while the device is busy")
    .unwrap();
    // 422: the saved source is Fixed, not Api. The point is that the route answered at all.
    assert_eq!(put.status(), StatusCode::UNPROCESSABLE_ENTITY);

    release.notify_one();
    task.await.unwrap();
}

#[tokio::test]
async fn the_display_is_not_opened_while_the_vendor_app_runs() {
    let (router, app, _home, log) = build_with_display(true);
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let checks = Arc::new(std::sync::atomic::AtomicU32::new(0));
    {
        let (running, checks) = (running.clone(), checks.clone());
        app.displays.set_vendor_check(Arc::new(move || {
            checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            running.load(std::sync::atomic::Ordering::SeqCst)
        }));
    }
    put_config(&router, &config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None)).await;

    let t0 = Instant::now();
    assert_eq!(pass(&app, t0).await, displays::VENDOR_POLL);
    assert_eq!(log.lock().unwrap().connects, 0, "ZEUS CAST is running: nothing is opened");
    assert!(log.lock().unwrap().writes.is_empty());
    let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(status[0]["info"]["connected"], serde_json::json!(false), "reported as not connected");
    assert!(app.displays.runtime.read().await.vendor_blocked);

    // Still running at the next poll: still closed, still one check per poll.
    assert_eq!(pass(&app, t0 + displays::VENDOR_POLL).await, displays::VENDOR_POLL);
    assert_eq!(log.lock().unwrap().connects, 0);
    assert_eq!(checks.load(std::sync::atomic::Ordering::SeqCst), 2, "re-checked on each poll");

    // ZEUS CAST exits: the next poll opens the display and writes.
    running.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(pass(&app, t0 + displays::VENDOR_POLL * 2).await, Duration::from_millis(250));
    assert_eq!(log.lock().unwrap().connects, 1);
    assert_eq!(log.lock().unwrap().writes.len(), 1);
    assert!(!app.displays.runtime.read().await.vendor_blocked);
}

/// #14 review: a vendor check that panics is "unknown", never "not running". The display stays
/// closed (no connect, no write) until a check succeeds.
#[tokio::test]
async fn a_panicking_vendor_check_keeps_the_display_closed() {
    let (router, app, _home, log) = build_with_display(true);
    let broken = Arc::new(std::sync::atomic::AtomicBool::new(true));
    {
        let broken = broken.clone();
        app.displays.set_vendor_check(Arc::new(move || {
            if broken.load(std::sync::atomic::Ordering::SeqCst) {
                panic!("process snapshot failed");
            }
            false
        }));
    }
    put_config(&router, &config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None)).await;

    let t0 = Instant::now();
    assert_eq!(pass(&app, t0).await, displays::VENDOR_POLL);
    assert_eq!(pass(&app, t0 + displays::VENDOR_POLL).await, displays::VENDOR_POLL);
    assert_eq!(log.lock().unwrap().connects, 0, "unknown: nothing is opened");
    assert!(log.lock().unwrap().writes.is_empty(), "unknown: nothing is written");
    assert!(app.displays.runtime.read().await.vendor_unknown);
    assert!(!app.displays.runtime.read().await.vendor_blocked, "unknown is not claimed as ZEUS CAST");

    // The check recovers: the display opens and writes.
    broken.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(pass(&app, t0 + displays::VENDOR_POLL * 2).await, Duration::from_millis(250));
    assert_eq!(log.lock().unwrap().connects, 1);
    assert_eq!(log.lock().unwrap().writes.len(), 1);
    assert!(!app.displays.runtime.read().await.vendor_unknown);

    // It breaks again while the display is open: the display is closed at the next vendor poll.
    broken.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(pass(&app, t0 + displays::VENDOR_POLL * 3).await, displays::VENDOR_POLL);
    assert!(app.displays.runtime.read().await.vendor_unknown);
    let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(status[0]["info"]["connected"], serde_json::json!(false));
}

/// #14: the editor warned about ZEUS CAST on every visit because the API never said whether it ran.
/// `info.blocked_by` follows the poll loop's own process check, both ways, within one poll.
#[tokio::test]
async fn the_api_reports_the_vendor_app_within_one_poll_of_it_starting_and_exiting() {
    let (router, app, _home, _log) = build_with_display(true);
    let running = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let running = running.clone();
        app.displays
            .set_vendor_check(Arc::new(move || running.load(std::sync::atomic::Ordering::SeqCst)));
    }
    put_config(&router, &config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None)).await;
    let blocked_by = || async {
        let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
        status[0]["info"]["blocked_by"].clone()
    };

    // ZEUS CAST is not running: connected, nothing blocks it.
    let t0 = Instant::now();
    pass(&app, t0).await;
    assert_eq!(blocked_by().await, serde_json::Value::Null, "not running: no warning");
    let connected = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(connected[0]["info"]["connected"], serde_json::json!(true));

    // It starts while the display is open: the next vendor poll reports it by name, closes the
    // display, and the status reads as not connected.
    running.store(true, std::sync::atomic::Ordering::SeqCst);
    pass(&app, t0 + displays::VENDOR_POLL).await;
    assert_eq!(blocked_by().await, serde_json::json!("ZEUS CAST"));
    let held = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(held[0]["info"]["connected"], serde_json::json!(false));

    // It exits: the next poll clears it.
    running.store(false, std::sync::atomic::Ordering::SeqCst);
    pass(&app, t0 + displays::VENDOR_POLL * 2).await;
    assert_eq!(blocked_by().await, serde_json::Value::Null, "exited: the warning goes");
}

#[tokio::test]
async fn the_real_vendor_check_matches_a_process_name_exactly() {
    // Not the real machine's answer (ZEUS CAST may be running here); only that it runs and returns.
    let _ = displays::vendor_app_running();
    assert_eq!(displays::VENDOR_EXE, "ZEUS CAST.exe");
}

#[tokio::test]
async fn repeating_an_unchanged_put_does_not_reset_the_backoff() {
    let (router, app, _home, log) = build_with_display(true);
    let body = config(DisplaySource::Fixed { value: 5.0 }, DisplayUnit::Celsius, 0, None);
    put_config(&router, &body).await;
    log.lock().unwrap().fail_connect = 5;
    let t0 = Instant::now();
    assert_eq!(pass(&app, t0).await, Duration::from_secs(1));
    assert_eq!(pass(&app, t0 + Duration::from_secs(1)).await, Duration::from_secs(2));
    assert_eq!(log.lock().unwrap().connects, 2);

    // The same config again: the backoff keeps counting (no connect, no reset to 1 s).
    assert_eq!(put_config(&router, &body).await.status(), StatusCode::OK);
    assert_eq!(pass(&app, t0 + Duration::from_millis(1500)).await, Duration::from_millis(1500), "still waiting out the 2 s");
    assert_eq!(log.lock().unwrap().connects, 2, "an unchanged PUT retries nothing");
    assert_eq!(pass(&app, t0 + Duration::from_secs(3)).await, Duration::from_secs(4), "the wait keeps doubling");

    // A changed config does drop the pacing and retry at once.
    let changed = config(DisplaySource::Fixed { value: 6.0 }, DisplayUnit::Celsius, 0, None);
    put_config(&router, &changed).await;
    assert_eq!(pass(&app, t0 + Duration::from_millis(3100)).await, Duration::from_secs(1));
    assert_eq!(log.lock().unwrap().connects, 4);
}

// ---------------------------------------------------------------- export and import of displays

#[tokio::test]
async fn displays_are_exported_and_a_bundle_without_the_field_still_imports() {
    let (router, app, home, _log) = build_with_display(true);
    let saved = config(DisplaySource::BoardTemp { sensor: "board_temp0".into() }, DisplayUnit::Fahrenheit, 1, Some(90.0));
    put_config(&router, &saved).await;

    let bundle = json(router.clone().oneshot(request("GET", routes::CONFIG_EXPORT)).await.unwrap()).await;
    assert_eq!(bundle["displays"].as_array().map(Vec::len), Some(1), "the export carries the display configs");
    assert_eq!(bundle["displays"][0]["device"], serde_json::json!(ID));

    // Delete the display, then import the bundle: displays.json comes back from it.
    router.clone().oneshot(request("DELETE", &display_path())).await.unwrap();
    assert!(app.store.read().await.displays.displays.is_empty());
    let response = router
        .clone()
        .oneshot(request_with_body("POST", routes::CONFIG_IMPORT, &bundle.to_string()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(app.store.read().await.displays.displays, vec![saved.clone()]);
    let reloaded = canvas_daemon::storage::Store::load_from(&home.path);
    assert_eq!(reloaded.displays.displays, vec![saved]);

    // A bundle from before displays has no `displays` key and still imports. As with the other
    // files, the import replaces what was saved, so the display configs are now empty.
    let mut old = bundle.clone();
    old.as_object_mut().unwrap().remove("displays");
    assert!(old.get("displays").is_none());
    let response = router
        .clone()
        .oneshot(request_with_body("POST", routes::CONFIG_IMPORT, &old.to_string()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "a bundle without displays imports");
    assert!(app.store.read().await.displays.displays.is_empty());
    assert!(canvas_daemon::storage::Store::load_from(&home.path).displays.displays.is_empty());
}

#[tokio::test]
async fn an_import_with_a_bad_display_config_is_refused() {
    let (router, app, _home, _log) = build_with_display(true);
    let mut bundle = json(router.clone().oneshot(request("GET", routes::CONFIG_EXPORT)).await.unwrap()).await;
    let bad = config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 2, None);
    bundle["displays"] = serde_json::json!([bad]);
    let response = router
        .clone()
        .oneshot(request_with_body("POST", routes::CONFIG_IMPORT, &bundle.to_string()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(app.store.read().await.displays.displays.is_empty());
}

#[tokio::test]
async fn identify_is_an_override_that_stop_ends_even_with_output_paused() {
    use canvas_proto::device::DeviceKind;
    use canvas_proto::traits::FrameSink;
    let (router, app, _home) = build();
    *app.devices.write().await = vec![
        common::device("Board", DeviceKind::Motherboard, 12),
        common::device("Ram", DeviceKind::Motherboard, 8),
    ];
    // Paused output: the engine renders nothing, and identify must still show.
    let response = router
        .clone()
        .oneshot(request_with_body("PATCH", routes::OUTPUT, r#"{"power":"paused"}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let sink = canvas_daemon::state::DeviceSink::with_gate(
        canvas_proto::DeviceId("fake:Board".into()),
        tx,
        app.identify.clone(),
    );

    let path = routes::DEVICE_IDENTIFY.replace("{id}", "fake:Board");
    let response = router
        .clone()
        .oneshot(request_with_body("POST", &path, r#"{"zone":0,"led":4,"count":3}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(app.identify.is_active());
    let frame = rx.try_recv().expect("the first override frame is sent at once");
    let lit: Vec<usize> =
        frame.iter().enumerate().filter(|(_, c)| **c != canvas_proto::Rgb8::BLACK).map(|(i, _)| i).collect();
    assert_eq!(lit, vec![4, 5, 6]);

    // The engine's own frames are silent during the override.
    sink.submit(vec![canvas_proto::Rgb8::WHITE; 12]);
    assert!(rx.try_recv().is_err());

    // A zone or LED that does not exist is a validation error.
    let response = router
        .clone()
        .oneshot(request_with_body("POST", &path, r#"{"zone":3,"led":0}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = router
        .clone()
        .oneshot(request_with_body("POST", &path, r#"{"zone":0,"led":12}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // `stop` ends it.
    let response = router
        .oneshot(request_with_body("POST", &path, r#"{"zone":0,"led":0,"stop":true}"#))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!app.identify.is_active());
    sink.submit(vec![canvas_proto::Rgb8::WHITE; 12]);
    assert!(rx.try_recv().is_ok(), "engine frames flow again");
}

// ---- user presets (#9) ----

/// A number property value as the API writes it.
fn num(v: f64) -> serde_json::Value {
    serde_json::json!({ "type": "number", "value": v })
}

/// Sends one request and returns the status and the JSON body.
async fn send(
    router: &axum::Router,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(request_with_body(method, path, &body.to_string()))
        .await
        .expect("the router answers");
    let status = response.status();
    (status, json(response).await)
}

fn preset_names(body: &serde_json::Value) -> Vec<String> {
    body.get("user_presets")
        .and_then(|v| v.as_array())
        .expect("the user presets")
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect()
}

#[tokio::test]
async fn a_preset_is_applied_on_the_defaults_not_on_the_saved_values() {
    // D9.1. Before the fix the saved values (here speed 77, set by a patch) were laid over the
    // preset, so applying a one-key preset kept the speed of the last session.
    let (router, _app, engine, _home) = build_with_engine();
    let (status, _) = send(&router, "POST", "/api/v1/effects/aurora/activate", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(
        &router,
        "PATCH",
        "/api/v1/effects/active/props",
        serde_json::json!({ "speed": num(77.0), "bands": num(5.0) }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // A user preset with a single key.
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Only scale", "props": { "scale": num(2.0) } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/activate",
        serde_json::json!({ "preset": "Only scale" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let props = body.get("props").expect("the props come back");
    // The schema defaults (speed 20, bands 3) plus the preset's key, nothing from the patch.
    assert_eq!(props["speed"]["value"], 20.0);
    assert_eq!(props["bands"]["value"], 3.0);
    assert_eq!(props["scale"]["value"], 2.0);
    // The engine got the same, and a full map: every key of the schema.
    let running = engine.props.lock().unwrap().clone();
    assert_eq!(running.len(), 6, "the effect is started with every property");
    assert_eq!(running.get("speed"), Some(&canvas_proto::props::PropValue::Number(20.0)));

    // The request's own props still come last.
    let (_, body) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/activate",
        serde_json::json!({ "preset": "Only scale", "props": { "speed": num(9.0) } }),
    )
    .await;
    assert_eq!(body["props"]["speed"]["value"], 9.0);
    assert_eq!(body["props"]["scale"]["value"], 2.0);
}

#[tokio::test]
async fn a_user_preset_is_found_by_name_when_the_effect_is_activated() {
    let (router, _app, _home) = build();
    send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Storm", "props": { "speed": num(61.0) } }),
    )
    .await;
    // Case does not matter to activation, as for the built-in presets.
    let (status, body) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/activate",
        serde_json::json!({ "preset": "STORM" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["props"]["speed"]["value"], 61.0);

    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/activate",
        serde_json::json!({ "preset": "Nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a missing preset is a 404");
}

#[tokio::test]
async fn a_preset_name_that_clashes_is_a_409_ignoring_case() {
    let (router, _app, _home) = build();
    let create = |name: &str| serde_json::json!({ "name": name, "props": { "speed": num(1.0) } });
    // A built-in preset of Aurora, in other case.
    let (status, body) = send(&router, "POST", "/api/v1/effects/aurora/presets", create("EMBER")).await;
    assert_eq!(status, StatusCode::CONFLICT, "the built-in preset Ember is taken");
    assert_eq!(body["error"], "conflict");

    let (status, _) = send(&router, "POST", "/api/v1/effects/aurora/presets", create("Storm")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(&router, "POST", "/api/v1/effects/aurora/presets", create("sTORM")).await;
    assert_eq!(status, StatusCode::CONFLICT, "the user preset Storm is taken");

    // A name only has to be free for its own effect.
    let (status, _) = send(&router, "POST", "/api/v1/effects/rain/presets", create("Storm")).await;
    assert_eq!(status, StatusCode::OK);

    // An empty name and an unknown effect.
    let (status, _) = send(&router, "POST", "/api/v1/effects/aurora/presets", create("   ")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = send(&router, "POST", "/api/v1/effects/nothing/presets", create("X")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_user_preset_is_replaced_renamed_and_the_stale_edit_is_refused() {
    let (router, _app, _home) = build();
    send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Storm", "props": { "speed": num(61.0) } }),
    )
    .await;
    send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Calm", "props": { "speed": num(3.0) } }),
    )
    .await;

    // Overwrite with the right `expected`.
    let (status, body) = send(
        &router,
        "PUT",
        "/api/v1/effects/aurora/presets/Storm",
        serde_json::json!({
            "props": { "speed": num(70.0) },
            "expected": { "speed": num(61.0) },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user_presets"][0]["props"]["speed"]["value"], 70.0);

    // The same request again is stale: the stored props are no longer what the window saw.
    let (status, body) = send(
        &router,
        "PUT",
        "/api/v1/effects/aurora/presets/Storm",
        serde_json::json!({
            "props": { "speed": num(80.0) },
            "expected": { "speed": num(61.0) },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "a stale expected is a 409");
    assert_eq!(body["error"], "conflict");
    let (_, detail) = send(&router, "GET", "/api/v1/effects/aurora", serde_json::json!({})).await;
    assert_eq!(
        detail["user_presets"][0]["props"]["speed"]["value"], 70.0,
        "the stale edit changed nothing"
    );

    // Rename keeps the place in the list; a case-only change of its own name is allowed.
    let same = serde_json::json!({ "props": { "speed": num(70.0) }, "expected": { "speed": num(70.0) } });
    let mut rename = same.clone();
    rename["rename_to"] = "Gale".into();
    let (status, body) = send(&router, "PUT", "/api/v1/effects/aurora/presets/Storm", rename).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preset_names(&body), ["Gale", "Calm"]);

    let mut case_only = same.clone();
    case_only["rename_to"] = "GALE".into();
    let (status, body) = send(&router, "PUT", "/api/v1/effects/aurora/presets/Gale", case_only).await;
    assert_eq!(status, StatusCode::OK, "a preset may change the case of its own name");
    assert_eq!(preset_names(&body), ["GALE", "Calm"]);

    // Renaming onto another preset, or a built-in one, is a 409.
    let mut onto = same.clone();
    onto["rename_to"] = "calm".into();
    let (status, _) = send(&router, "PUT", "/api/v1/effects/aurora/presets/GALE", onto).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let mut onto_built_in = same.clone();
    onto_built_in["rename_to"] = "ember".into();
    let (status, _) = send(&router, "PUT", "/api/v1/effects/aurora/presets/GALE", onto_built_in).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A missing preset is a 404.
    let (status, _) = send(&router, "PUT", "/api/v1/effects/aurora/presets/Nope", same).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_a_user_preset_removes_it_and_a_missing_one_is_a_404() {
    let (router, _app, _home) = build();
    send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Storm", "props": { "speed": num(61.0) } }),
    )
    .await;
    let (status, body) =
        send(&router, "DELETE", "/api/v1/effects/aurora/presets/Storm", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(preset_names(&body).is_empty());

    let (status, _) =
        send(&router, "DELETE", "/api/v1/effects/aurora/presets/Storm", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the preset is gone");
    let (status, _) =
        send(&router, "DELETE", "/api/v1/effects/nothing/presets/Storm", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no such effect");
    // A built-in preset is not the owner's to delete.
    let (status, _) =
        send(&router, "DELETE", "/api/v1/effects/aurora/presets/Ember", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn user_presets_are_saved_to_disk_and_survive_a_restart() {
    let (router, app, home) = build();
    send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Storm", "props": { "speed": num(61.0) } }),
    )
    .await;
    drop(app);
    // A second store over the same folder is the daemon after a restart.
    let reopened = canvas_daemon::storage::Store::load_from(&home.path);
    let names: Vec<&str> = reopened
        .effects
        .presets
        .get("aurora")
        .expect("the presets are on disk")
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, ["Storm"]);
}

/// The rule in `docs/preset-equality.json`. The editor's rule is the only production one; this
/// copy pins the fixture, with JavaScript's rounding. Missing keys take the default, a number rounds to
/// its step, a hue to a whole degree modulo 360, colours compare without case, keys the schema
/// does not name are ignored. The editor reads the same file in `presetMatch.test.ts`.
#[test]
fn the_preset_equality_fixture_holds() {
    use canvas_proto::props::{PropKind, PropValue, PropertyDef, Props};

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/preset-equality.json");
    let text = std::fs::read_to_string(&path).expect("the fixture is in docs/");
    let fixture: serde_json::Value = serde_json::from_str(&text).expect("the fixture is JSON");
    let defs: Vec<PropertyDef> =
        serde_json::from_value(fixture["properties"].clone()).expect("the properties are proto PropertyDefs");

    fn normal(defs: &[PropertyDef], props: &Props) -> Vec<(String, String)> {
        defs.iter()
            .map(|d| {
                let value = props.get(&d.key).cloned().unwrap_or_else(|| d.kind.default_value());
                let text = match (&d.kind, value) {
                    (PropKind::Number { step, .. }, PropValue::Number(v)) => {
                        let step = if *step > 0.0 { *step } else { 1.0 };
                        // JavaScript's Math.round: a half goes toward +infinity (f64::round goes
                        // away from zero, which differs for a negative half).
                        let rounded = (v / step + 0.5).floor() * step;
                        format!("{}", (rounded * 1e4 + 0.5).floor() / 1e4)
                    }
                    (PropKind::Hue { .. }, PropValue::Hue(v)) => {
                        format!("{}", (v + 0.5).floor().rem_euclid(360.0))
                    }
                    (_, PropValue::Color(c)) => {
                        serde_json::to_value(c).unwrap().as_str().unwrap().to_lowercase()
                    }
                    (_, other) => serde_json::to_string(&other).unwrap(),
                };
                (d.key.clone(), text)
            })
            .collect()
    }

    let cases = fixture["cases"].as_array().expect("the cases");
    assert!(cases.len() >= 20, "the fixture has its cases");
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let preset: Props = serde_json::from_value(case["preset"].clone()).expect(name);
        let live: Props = serde_json::from_value(case["live"].clone()).expect(name);
        let equal = normal(&defs, &preset) == normal(&defs, &live);
        assert_eq!(equal, case["equal"].as_bool().unwrap(), "{name}");
    }
}

// ---- user presets: the fix pass ----

/// An app whose user effect `html:fancy` has one number property `speed` (0 to `max` in the file:
/// the file's text is `max=NN`), so a test can narrow the range by writing the file again.
fn build_with_speed_effect() -> (axum::Router, Arc<App>, TempHome, std::path::PathBuf) {
    use canvas_proto::props::{PropKind, PropertyDef};
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    let fake: canvas_daemon::user_effects::Builder = Arc::new(|id: &str, source: &str| {
        let max = source
            .strip_prefix("max=")
            .and_then(|n| n.trim().parse::<f64>().ok())
            .unwrap_or(100.0);
        let mut effect = common::FakeEffect::new(id);
        effect.schema.properties = vec![PropertyDef {
            key: "speed".to_string(),
            label: "Speed".to_string(),
            kind: PropKind::Number { min: 0.0, max, step: 1.0, default: 20.0 },
        }];
        Ok(Box::new(effect) as Box<dyn canvas_proto::traits::Effect>)
    });
    app.user_effects = Arc::new(canvas_daemon::user_effects::UserEffects::new(fake));
    let folder = home.path.join("effects");
    std::fs::create_dir_all(&folder).expect("the effects folder is created");
    std::fs::write(folder.join("fancy.html"), "max=100").expect("the effect file is written");
    app.user_effects.rescan(&folder);
    app.effects_dir = folder.clone();
    let app = Arc::new(app);
    (api::router(app.clone()), app, home, folder)
}

#[tokio::test]
async fn a_failed_save_leaves_no_ghost_preset() {
    // The save fails because effects.json is a folder. Before the fix the preset was already in
    // memory, so the next GET listed a preset that was not on disk.
    let (router, _app, home) = build();
    std::fs::create_dir_all(home.path.join("effects.json")).expect("the blocker is made");
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Ghost", "props": { "speed": num(61.0) } }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "the save failed");
    let (_, detail) = send(&router, "GET", "/api/v1/effects/aurora", serde_json::json!({})).await;
    assert!(
        detail["user_presets"].as_array().unwrap().is_empty(),
        "no ghost preset in memory: {detail}"
    );
}

#[tokio::test]
async fn a_failed_save_rolls_back_a_replace_and_a_delete() {
    let (router, _app, home) = build();
    send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Storm", "props": { "speed": num(61.0) } }),
    )
    .await;
    // The disk fails from here on.
    let file = home.path.join("effects.json");
    std::fs::remove_file(&file).expect("the file is removed");
    std::fs::create_dir_all(&file).expect("the blocker is made");

    let (status, _) = send(
        &router,
        "PUT",
        "/api/v1/effects/aurora/presets/Storm",
        serde_json::json!({
            "rename_to": "Gale",
            "props": { "speed": num(70.0) },
            "expected": { "speed": num(61.0) },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let (status, _) =
        send(&router, "DELETE", "/api/v1/effects/aurora/presets/Storm", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let (_, detail) = send(&router, "GET", "/api/v1/effects/aurora", serde_json::json!({})).await;
    assert_eq!(detail["user_presets"][0]["name"], "Storm", "the rename did not stick");
    assert_eq!(detail["user_presets"][0]["props"]["speed"]["value"], 61.0, "the overwrite did not stick");
}

#[tokio::test]
async fn preset_props_are_fitted_to_the_schema_and_capped() {
    let (router, _app, _home) = build();
    // An unknown key is dropped and a number is clamped to the schema's range (Aurora speed 0-100).
    let (status, body) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({
            "name": "Wild",
            "props": { "speed": num(9999.0), "ghost": num(1.0), "bands": { "type": "bool", "value": true } },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let props = &body["user_presets"][0]["props"];
    assert_eq!(props["speed"]["value"], 100.0, "clamped to the schema's max");
    assert!(props.get("ghost").is_none(), "an unknown key is dropped");
    assert!(props.get("bands").is_none(), "a value of the wrong type is dropped");

    // Over the caps: too many keys, too many bytes, too many presets.
    let many: serde_json::Map<String, serde_json::Value> =
        (0..65).map(|i| (format!("k{i}"), num(1.0))).collect();
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Many", "props": many }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "65 keys is over the cap");
    let long = "x".repeat(9000);
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Big", "props": { "mode": { "type": "choice", "value": long } } }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "9 KB of values is over the cap");
    for i in 0..99 {
        let (status, _) = send(
            &router,
            "POST",
            "/api/v1/effects/aurora/presets",
            serde_json::json!({ "name": format!("P{i}"), "props": {} }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "preset {i}");
    }
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "One too many", "props": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "the 101st preset is refused");
}

#[tokio::test]
async fn a_preset_saved_before_the_effect_narrowed_applies_clamped() {
    let (router, app, _home, folder) = build_with_speed_effect();
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/html:fancy/presets",
        serde_json::json!({ "name": "Fast", "props": { "speed": num(90.0) } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // The file is imported again with a narrower range, and a stored key it no longer has.
    std::fs::write(folder.join("fancy.html"), "max=50").expect("the file is written");
    app.user_effects.rescan(&folder);
    let (status, body) = send(
        &router,
        "POST",
        "/api/v1/effects/html:fancy/activate",
        serde_json::json!({ "preset": "Fast" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["props"]["speed"]["value"], 50.0, "applied clamped to the new range");
}

#[tokio::test]
async fn a_preset_name_with_invisible_or_bidirectional_characters_is_refused() {
    let (router, _app, _home) = build();
    for bad in ["Evil\u{202E}txt", "Zero\u{200B}width", "Bom\u{FEFF}", "Tab\tname", "Line\nbreak"] {
        let (status, _) = send(
            &router,
            "POST",
            "/api/v1/effects/aurora/presets",
            serde_json::json!({ "name": bad, "props": {} }),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad:?}");
    }
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "x".repeat(41), "props": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "41 characters is too long");
    // A plain accented name is fine.
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "Soir\u{e9}e", "props": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_clash_rule_is_ascii_only_not_unicode() {
    let (router, _app, _home) = build();
    send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "\u{c9}t\u{e9}", "props": {} }),
    )
    .await;
    // E-acute in lower case is another name for the ASCII rule: both are kept.
    let (status, _) = send(
        &router,
        "POST",
        "/api/v1/effects/aurora/presets",
        serde_json::json!({ "name": "\u{e9}t\u{e9}", "props": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "only ASCII letters fold, as the web rule does");
}

/// Save, apply by name, rename and delete a preset of an effect, whatever kind it is.
async fn preset_round_trip(router: &axum::Router, id: &str, key: &str) {
    let base = format!("/api/v1/effects/{id}");
    let (status, _) = send(
        router,
        "POST",
        &format!("{base}/presets"),
        serde_json::json!({ "name": "Mine", "props": { key: num(33.0) } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{id}: saved");
    let (status, body) =
        send(router, "POST", &format!("{base}/activate"), serde_json::json!({ "preset": "mine" })).await;
    assert_eq!(status, StatusCode::OK, "{id}: applied by name");
    assert_eq!(body["props"][key]["value"], 33.0, "{id}: the preset's value is running");
    let (status, body) = send(
        router,
        "PUT",
        &format!("{base}/presets/Mine"),
        serde_json::json!({
            "rename_to": "Yours",
            "props": { key: num(44.0) },
            "expected": { key: num(33.0) },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{id}: renamed");
    assert_eq!(preset_names(&body), ["Yours"]);
    let (status, body) = send(router, "DELETE", &format!("{base}/presets/Yours"), serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "{id}: deleted");
    assert!(preset_names(&body).is_empty());
}

#[tokio::test]
async fn presets_work_for_a_built_in_a_stock_html_and_a_user_html_effect() {
    // A Rust built-in.
    let (router, _app, _home) = build();
    preset_round_trip(&router, "aurora", "speed").await;
    // A stock HTML effect: the first number property it has.
    let (_, detail) = send(&router, "GET", "/api/v1/effects/html:ink-drop", serde_json::json!({})).await;
    let key = detail["schema"]["properties"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["kind"]["type"] == "number"))
        .and_then(|p| p["key"].as_str())
        .expect("the stock effect has a number property")
        .to_string();
    let min = detail["schema"]["properties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == key.as_str())
        .unwrap()["kind"]["min"]
        .as_f64()
        .unwrap();
    assert!(min <= 33.0, "the test value sits inside the stock range");
    preset_round_trip(&router, "html:ink-drop", &key).await;
    // A user HTML effect.
    let (router, _app, _home, _folder) = build_with_speed_effect();
    preset_round_trip(&router, "html:fancy", "speed").await;
}

#[tokio::test]
async fn deleting_a_user_effect_removes_its_presets_unless_they_are_kept() {
    // Default: the presets go with the effect, so importing the file again starts clean.
    let (router, app, home, folder) = build_with_speed_effect();
    send(
        &router,
        "POST",
        "/api/v1/effects/html:fancy/presets",
        serde_json::json!({ "name": "Fast", "props": { "speed": num(90.0) } }),
    )
    .await;
    let (status, body) =
        send(&router, "DELETE", "/api/v1/effects/html:fancy", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["presets_removed"], 1);
    assert!(!app.store.read().await.effects.presets.contains_key("html:fancy"), "no orphan in memory");
    let on_disk = canvas_daemon::storage::Store::load_from(&home.path);
    assert!(!on_disk.effects.presets.contains_key("html:fancy"), "no orphan on disk");
    std::fs::write(folder.join("fancy.html"), "max=100").expect("the file is imported again");
    app.user_effects.rescan(&folder);
    let (_, detail) = send(&router, "GET", "/api/v1/effects/html:fancy", serde_json::json!({})).await;
    assert!(detail["user_presets"].as_array().unwrap().is_empty(), "the presets did not come back");

    // keep_presets=true: they stay, and come back with the file.
    send(
        &router,
        "POST",
        "/api/v1/effects/html:fancy/presets",
        serde_json::json!({ "name": "Fast", "props": { "speed": num(90.0) } }),
    )
    .await;
    let (status, body) = send(
        &router,
        "DELETE",
        "/api/v1/effects/html:fancy?keep_presets=true",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["presets_removed"], 0);
    std::fs::write(folder.join("fancy.html"), "max=100").expect("the file is imported again");
    app.user_effects.rescan(&folder);
    let (_, detail) = send(&router, "GET", "/api/v1/effects/html:fancy", serde_json::json!({})).await;
    assert_eq!(preset_names(&detail), ["Fast"], "the kept preset is back");
}
