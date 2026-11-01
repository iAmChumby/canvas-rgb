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

// ---------------------------------------------------------------- preview

/// A preview request body for `html` at `props` (already JSON).
fn preview_body(html: &str, props: serde_json::Value) -> String {
    serde_json::json!({ "html": html, "props": props }).to_string()
}

/// The loop a preview answer carries, decoded.
fn thumbnail(value: &serde_json::Value) -> Vec<u8> {
    use base64::Engine as _;
    let text = value.get("thumbnail").and_then(|t| t.as_str()).expect("the answer has a loop");
    base64::engine::general_purpose::STANDARD.decode(text).expect("the loop is base64")
}

/// A POST to the preview route with the token.
async fn post_preview(router: axum::Router, body: String) -> axum::response::Response {
    router
        .oneshot(request_with_body("POST", routes::EFFECT_PREVIEW, &body))
        .await
        .expect("the router answers")
}

#[tokio::test]
async fn a_moving_draft_previews_with_its_schema_and_loop() {
    let (router, _app, _home, _folder) = build();
    let response = post_preview(
        router,
        preview_body(MOVING, serde_json::json!({ "tint": { "type": "color", "value": "#ff0000" } })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let answer = json(response).await;

    assert_eq!(answer["schema"]["name"], "Moving");
    assert_eq!(
        answer["source_hash"].as_str(),
        Some(canvas_daemon::user_effects::source_hash(MOVING).as_str()),
        "the hash is of the exact draft"
    );
    assert_eq!(answer["errors"], serde_json::json!([]), "a good draft has no errors");
    assert_eq!(answer["warnings"], serde_json::json!([]), "and no warnings");

    let loop_bytes = thumbnail(&answer);
    assert_eq!(&loop_bytes[..6], &[64u8, 0, 40, 0, 24, 0][..], "64x40, 24 frames");
    assert_eq!(loop_bytes.len(), 6 + 24 * 64 * 40 * 3);
    assert!(canvas_daemon::thumbnail::body_moves(&loop_bytes), "the loop moves");
    assert!(!canvas_daemon::thumbnail::body_is_flat(&loop_bytes), "and is not one colour");

    // The editor's props reach the render: another tint is another picture.
    let (router2, _app2, _home2, _folder2) = build();
    let other = post_preview(
        router2,
        preview_body(MOVING, serde_json::json!({ "tint": { "type": "color", "value": "#00ff00" } })),
    )
    .await;
    assert_eq!(other.status(), StatusCode::OK);
    assert_ne!(
        thumbnail(&json(other).await),
        loop_bytes,
        "the draft was rendered with the props the editor sent"
    );
}

#[tokio::test]
async fn a_still_draft_previews_with_the_check_warnings() {
    let (router, _app, _home, _folder) = build();
    let response = post_preview(router, preview_body(FLAT, serde_json::json!({}))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let answer = json(response).await;
    let warnings: Vec<String> = answer["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(|w| w.as_str().map(str::to_string))
        .collect();
    assert!(
        warnings.iter().any(|w| w == canvas_daemon::thumbnail::NO_MOVEMENT),
        "the still picture is reported: {warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| w == canvas_daemon::thumbnail::FLAT_COLOUR),
        "the flat picture is reported: {warnings:?}"
    );
}

#[tokio::test]
async fn a_draft_that_does_not_parse_is_refused() {
    let (router, _app, _home, _folder) = build();
    let response = post_preview(router, preview_body(NO_SCRIPT, serde_json::json!({}))).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let message = json(response).await["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("parse"), "the parser's message is kept: {message}");
}

#[tokio::test]
async fn a_script_that_throws_is_reported_with_its_loop() {
    let source = r##"<head><title>Throws</title></head>
<body><canvas id="c" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("c");
  var ctx = c.getContext("2d");
  ctx.fillStyle = "#ff8800";
  ctx.fillRect(0, 0, 320, 200);
  function draw() {
    notAFunction();
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;
    let (router, _app, _home, _folder) = build();
    let response = post_preview(router, preview_body(source, serde_json::json!({}))).await;
    assert_eq!(response.status(), StatusCode::OK, "the loop is still served");
    let answer = json(response).await;
    let errors: Vec<String> = answer["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .filter_map(|e| e.as_str().map(str::to_string))
        .collect();
    assert!(!errors.is_empty(), "the thrown error is reported: {answer}");
    assert!(
        errors.iter().any(|e| e.contains("notAFunction")),
        "the error names what threw: {errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.contains("eval_script:") && e.contains(':')),
        "the runtime's line is kept: {errors:?}"
    );
    let loop_bytes = thumbnail(&answer);
    assert_eq!(loop_bytes.len(), 6 + 24 * 64 * 40 * 3, "the last good frame is still a loop");
}

#[tokio::test]
async fn a_runaway_draft_is_stopped_and_the_api_keeps_answering() {
    let runaway = r##"<head><title>Runaway</title></head>
<body><canvas id="c"></canvas></body>
<script>
  var c = document.getElementById("c");
  var ctx = c.getContext("2d");
  while (true) {}
</script>"##;
    let (router, _app, _home, _folder) = build();
    // The runtime's own load budget stops the script; this bound only keeps a broken build
    // from hanging the test run.
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        post_preview(router.clone(), preview_body(runaway, serde_json::json!({}))),
    )
    .await
    .expect("the preview answers; the runaway script does not hold it");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let message = json(response).await["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("too long"), "the answer says what happened: {message}");

    // The API kept answering, and a good preview still renders.
    let status = router
        .clone()
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
    assert_eq!(status.status(), StatusCode::OK);
    let good = post_preview(router, preview_body(MOVING, serde_json::json!({}))).await;
    assert_eq!(good.status(), StatusCode::OK, "the next draft renders");
}

#[tokio::test]
async fn a_preview_body_over_the_limit_is_refused() {
    let (router, _app, _home, _folder) = build();
    // One byte over the import's body cap.
    let html = "a".repeat(canvas_daemon::api::effects::IMPORT_BODY_LIMIT);
    let body = preview_body(&html, serde_json::json!({}));
    let response = post_preview(router, body).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let error = json(response).await["error"].as_str().unwrap_or_default().to_string();
    assert_eq!(error, canvas_proto::api::error_codes::VALIDATION);
}

#[tokio::test]
async fn the_preview_route_needs_the_token() {
    let (router, _app, _home, _folder) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(routes::EFFECT_PREVIEW)
                .header("host", "127.0.0.1:9430")
                .header("content-type", "application/json")
                .body(Body::from(preview_body(MOVING, serde_json::json!({}))))
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
