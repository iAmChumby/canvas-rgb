//! The effect editor's daemon routes: an effect's source, a draft's preview, and the draft on
//! the lights.
//!
//! Every test runs against the in-test fake engine and adapter and a temporary data folder, so
//! no device, no saved file and nothing in the owner's own folder is touched.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::api::{routes, EffectSource};
use common::{FakeAdapter, FakeEngine};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A temporary directory that is removed when it is dropped.
struct TempHome {
    path: PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-editor-{}-{:?}",
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

/// The router, the state, the home and the effects folder.
fn build() -> (axum::Router, Arc<App>, TempHome, PathBuf) {
    let home = TempHome::new();
    let folder = home.path.join("effects");
    std::fs::create_dir_all(&folder).expect("the effects folder is created");
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
    // The routes work on this folder, never on the owner's.
    app.effects_dir = folder.clone();
    let app = Arc::new(app);
    (api::router(app.clone()), app, home, folder)
}

/// A request with the token and a body.
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

/// A request with the token and no body.
fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

/// The body of a response, as JSON.
async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("the body is bounded");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// A moving effect whose block is painted with a property, so a test can see the props reach
/// the picture.
const MOVING: &str = r##"<head>
  <title>Moving</title>
  <meta property="tint" label="Tint" type="color" default="#ffcc00" />
</head>
<body><canvas id="c" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("c");
  var ctx = c.getContext("2d");
  var x = 0;
  function draw() {
    ctx.fillStyle = "#101020";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = tint;
    ctx.fillRect(x, 40, 40, 40);
    x = (x + 4) % 280;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

/// An effect that draws one flat colour and never changes it.
const FLAT: &str = r##"<head><title>Flat</title></head>
<body><canvas id="c" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("c");
  var ctx = c.getContext("2d");
  ctx.fillStyle = "#3366cc";
  function draw() { window.requestAnimationFrame(draw); }
  window.requestAnimationFrame(draw);
</script>"##;

/// A file with no script: the parser refuses it.
const NO_SCRIPT: &str = "<head><title>Empty</title></head><body></body>";

// ---------------------------------------------------------------- source

#[tokio::test]
async fn a_user_effect_serves_its_source_and_the_file_name_save_would_use() {
    let (router, app, _home, folder) = build();
    std::fs::write(folder.join("My Effect.html"), MOVING).expect("the effect is written");
    app.user_effects.rescan(&folder);

    let response = router
        .oneshot(request("GET", "/api/v1/effects/html:My%20Effect/source"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let source: EffectSource = serde_json::from_value(json(response).await).expect("an EffectSource");
    assert_eq!(source.source, MOVING);
    assert_eq!(source.file_name, "My Effect.html");
}

#[tokio::test]
async fn a_stock_effect_serves_its_source_and_a_file_name_for_save() {
    let (router, _app, _home, _folder) = build();
    let response = router
        .oneshot(request("GET", "/api/v1/effects/html:ink-drop/source"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let source: EffectSource = serde_json::from_value(json(response).await).expect("an EffectSource");
    assert!(!source.source.is_empty(), "the stock source is there");
    assert_eq!(source.file_name, "ink-drop.html");
}

#[tokio::test]
async fn a_rust_built_in_has_no_source() {
    let (router, _app, _home, _folder) = build();
    let response = router
        .oneshot(request("GET", &routes::EFFECT_SOURCE.replace("{id}", "solid")))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json(response).await;
    let message = body.get("message").and_then(|m| m.as_str()).unwrap_or_default();
    assert!(message.contains("built-in"), "the message says why: {message}");
    assert!(message.contains("no HTML source"), "the message says why: {message}");
}

#[tokio::test]
async fn an_unknown_effect_has_no_source() {
    let (router, _app, _home, _folder) = build();
    let response = router
        .oneshot(request("GET", "/api/v1/effects/html:not-there/source"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_source_route_needs_the_token() {
    let (router, _app, _home, _folder) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/effects/html:ink-drop/source")
                .header("host", "127.0.0.1:9430")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
