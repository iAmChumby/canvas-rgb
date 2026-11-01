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
    let (router, app, _engine, home, folder, _clock) = build_with_clock();
    (router, app, home, folder)
}

/// A clock the test moves by hand: a draft lease is tested without waiting.
#[derive(Clone)]
struct TestClock {
    now: Arc<std::sync::Mutex<std::time::Duration>>,
}

impl TestClock {
    fn new() -> Self {
        Self { now: Arc::new(std::sync::Mutex::new(std::time::Duration::ZERO)) }
    }

    /// The clock the draft reads.
    fn clock(&self) -> canvas_daemon::draft::Clock {
        let now = self.now.clone();
        Arc::new(move || *now.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    /// Moves the clock on.
    fn advance(&self, by: std::time::Duration) {
        let mut now = self.now.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *now += by;
    }
}

/// Like [`build`], with the fake engine and the draft's clock handed back too.
fn build_with_clock(
) -> (axum::Router, Arc<App>, Arc<FakeEngine>, TempHome, PathBuf, TestClock) {
    let home = TempHome::new();
    let folder = home.path.join("effects");
    std::fs::create_dir_all(&folder).expect("the effects folder is created");
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    // The routes work on this folder, never on the owner's.
    app.effects_dir = folder.clone();
    // A lease is driven by the test's clock, never by the wall clock.
    let clock = TestClock::new();
    app.draft = canvas_daemon::draft::Draft::with_clock(clock.clock());
    let app = Arc::new(app);
    (api::router(app.clone()), app, engine, home, folder, clock)
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
async fn a_file_that_does_not_parse_still_serves_its_source() {
    let (router, app, _home, folder) = build();
    std::fs::write(folder.join("broken.html"), NO_SCRIPT).expect("the broken effect is written");
    app.user_effects.rescan(&folder);
    // The file is not in the library, but it is exactly what needs editing.
    assert!(app.user_effects.schemas().iter().all(|s| s.id != "html:broken"));

    let response = router
        .oneshot(request("GET", "/api/v1/effects/html:broken/source"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let source: EffectSource = serde_json::from_value(json(response).await).expect("an EffectSource");
    assert_eq!(source.source, NO_SCRIPT);
    assert_eq!(source.file_name, "broken.html");
}

#[tokio::test]
async fn a_source_id_cannot_reach_out_of_the_effects_folder() {
    let (router, _app, _home, _folder) = build();
    let response = router
        .oneshot(request("GET", "/api/v1/effects/html:..%2Fsecret/source"))
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

// ---------------------------------------------------------------- draft on the lights

/// A `color` prop, as the editor sends it.
fn color(hex: &str) -> canvas_proto::props::Props {
    let mut props = canvas_proto::props::Props::new();
    props.insert(
        "color".to_string(),
        canvas_proto::PropValue::Color(hex.parse().expect("a #rrggbb colour")),
    );
    props
}

/// Makes `id` the active layout's effect, and runs it: this is what a draft takes over from.
async fn prior_effect(app: &Arc<App>, id: &str, props: canvas_proto::props::Props) {
    {
        let mut store = app.store.write().await;
        store.layouts.layouts[0].effect =
            Some(canvas_proto::layout::ActiveEffect { id: id.to_string(), props: props.clone() });
    }
    canvas_daemon::apply_effect(app, id, &props).await.expect("the prior effect runs");
}

/// What the fake engine is running, and with which props.
fn running(engine: &FakeEngine) -> (Option<String>, canvas_proto::props::Props) {
    (
        engine.effect_id.lock().unwrap().clone(),
        engine.props.lock().unwrap().clone(),
    )
}

/// A PUT to the draft route.
async fn put_draft(
    router: axum::Router,
    html: &str,
    props: serde_json::Value,
) -> axum::response::Response {
    let body = serde_json::json!({ "html": html, "props": props }).to_string();
    router
        .oneshot(request_with_body("PUT", routes::EFFECT_DRAFT, &body))
        .await
        .expect("the router answers")
}

/// The draft state the daemon reports.
async fn draft_state(router: axum::Router) -> canvas_proto::api::DraftStatus {
    let response = router
        .oneshot(request("GET", routes::EFFECT_DRAFT))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_value(json(response).await).expect("a DraftStatus")
}

#[tokio::test]
async fn a_draft_runs_on_the_lights_and_stop_puts_the_prior_effect_back() {
    let (router, app, engine, _home, _folder, _clock) = build_with_clock();
    let prior = color("#112233");
    prior_effect(&app, "solid", prior.clone()).await;
    assert_eq!(running(&engine).0.as_deref(), Some("solid"));

    let props = serde_json::json!({ "tint": { "type": "color", "value": "#ff0000" } });
    let response = put_draft(router.clone(), MOVING, props.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let status: canvas_proto::api::DraftStatus =
        serde_json::from_value(json(response).await).expect("a DraftStatus");
    let hash = canvas_daemon::user_effects::source_hash(MOVING);
    assert!(status.active, "the draft is on");
    assert_eq!(status.id.as_deref(), Some(format!("draft:{hash}").as_str()));
    assert_eq!(status.source_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(status.props.len(), 1, "the props are reported: {:?}", status.props);
    assert_eq!(status.heartbeat_ms, 5_000);
    assert_eq!(status.lease_ms, 15_000);
    assert!(status.lease_remaining_ms > 0, "the lease is running");

    let (id, engine_props) = running(&engine);
    assert_eq!(id.as_deref(), Some(format!("draft:{hash}").as_str()));
    assert_eq!(engine_props.len(), 1, "the draft was handed to the engine with its props");

    // The same draft is reported for a reload.
    let state = draft_state(router.clone()).await;
    assert!(state.active);
    assert_eq!(state.id, status.id);

    let response = router
        .clone()
        .oneshot(request("DELETE", routes::EFFECT_DRAFT))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let stopped: canvas_proto::api::DraftStatus =
        serde_json::from_value(json(response).await).expect("a DraftStatus");
    assert!(!stopped.active, "the draft is over");

    let (id, engine_props) = running(&engine);
    assert_eq!(id.as_deref(), Some("solid"), "the prior effect is back");
    assert_eq!(engine_props, prior, "with the props it had");
    assert!(!draft_state(router).await.active);
}

#[tokio::test]
async fn a_beat_renews_the_lease_and_a_beat_after_the_lease_still_runs() {
    let (router, app, engine, _home, _folder, clock) = build_with_clock();
    prior_effect(&app, "solid", color("#112233")).await;

    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);
    clock.advance(std::time::Duration::from_secs(10));
    // A second draft replaces the code but keeps the visit: the prior is still solid.
    let changed = MOVING.replace("#ffcc00", "#00ff00");
    assert_eq!(put_draft(router.clone(), &changed, serde_json::json!({})).await.status(), StatusCode::OK);
    clock.advance(std::time::Duration::from_secs(10));

    let state = draft_state(router.clone()).await;
    assert!(state.active, "the second beat renewed the lease");
    assert_eq!(
        state.source_hash.as_deref(),
        Some(canvas_daemon::user_effects::source_hash(&changed).as_str())
    );
    assert_eq!(running(&engine).0.as_deref(), Some(format!("draft:{}", canvas_daemon::user_effects::source_hash(&changed)).as_str()));

    // Sixteen seconds after the last beat the lease is over: the prior effect is back.
    clock.advance(std::time::Duration::from_secs(16));
    assert!(!draft_state(router).await.active);
    assert_eq!(running(&engine).0.as_deref(), Some("solid"));
}

#[tokio::test]
async fn a_draft_whose_lease_runs_out_restores_the_prior_effect() {
    let (router, app, engine, _home, _folder, clock) = build_with_clock();
    let prior = color("#224466");
    prior_effect(&app, "solid", prior.clone()).await;
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);
    assert_eq!(
        running(&engine).0,
        Some(format!("draft:{}", canvas_daemon::user_effects::source_hash(MOVING)))
    );

    clock.advance(canvas_proto::consts::DRAFT_LEASE);
    assert!(!draft_state(router).await.active, "the lease ran out");
    let (id, props) = running(&engine);
    assert_eq!(id.as_deref(), Some("solid"), "the prior effect is back");
    assert_eq!(props, prior, "with the props it had");
}

#[tokio::test]
async fn a_saved_activation_ends_the_draft_without_restoring_it() {
    let (router, app, engine, _home, _folder, clock) = build_with_clock();
    prior_effect(&app, "solid", color("#112233")).await;
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            &routes::EFFECT_ACTIVATE.replace("{id}", "gradient"),
            r#"{"props": {}, "preset": null}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(running(&engine).0.as_deref(), Some("gradient"));

    // The draft is gone, and time passing does not bring the old effect back.
    assert!(!draft_state(router.clone()).await.active);
    clock.advance(canvas_proto::consts::DRAFT_LEASE * 2);
    assert!(!draft_state(router).await.active);
    assert_eq!(
        running(&engine).0.as_deref(),
        Some("gradient"),
        "the owner chose gradient; nothing restored solid over it"
    );
}

#[tokio::test]
async fn a_layout_switch_ends_the_draft_without_restoring_it() {
    let (router, app, engine, _home, _folder, clock) = build_with_clock();
    prior_effect(&app, "solid", color("#112233")).await;
    // A second layout, with an effect of its own.
    {
        let mut store = app.store.write().await;
        let mut other = store.layouts.layouts[0].clone();
        other.id = "other".to_string();
        other.name = "Other".to_string();
        other.effect = Some(canvas_proto::layout::ActiveEffect {
            id: "gradient".to_string(),
            props: canvas_proto::props::Props::new(),
        });
        store.layouts.layouts.push(other);
    }
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PUT",
            routes::LAYOUT_ACTIVE,
            r#"{"id": "other"}"#,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(running(&engine).0.as_deref(), Some("gradient"));

    assert!(!draft_state(router).await.active, "the switch ended the draft");
    clock.advance(canvas_proto::consts::DRAFT_LEASE * 2);
    assert_eq!(
        running(&engine).0.as_deref(),
        Some("gradient"),
        "nothing restored the draft's prior effect over the chosen layout"
    );
}

#[tokio::test]
async fn the_active_props_route_is_refused_while_a_draft_runs() {
    let (router, app, engine, _home, _folder, _clock) = build_with_clock();
    prior_effect(&app, "solid", color("#112233")).await;
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(request_with_body(
            "PATCH",
            routes::EFFECT_ACTIVE_PROPS,
            r##"{"color": {"type": "color", "value": "#00ff00"}}"##,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let message = json(response).await["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("draft"), "the message names the draft route: {message}");
    assert_eq!(
        running(&engine).0,
        Some(format!("draft:{}", canvas_daemon::user_effects::source_hash(MOVING))),
        "the draft is still what runs"
    );

    // After Stop, the route works again.
    assert_eq!(
        router
            .clone()
            .oneshot(request("DELETE", routes::EFFECT_DRAFT))
            .await
            .expect("the router answers")
            .status(),
        StatusCode::OK
    );
    let response = router
        .oneshot(request_with_body(
            "PATCH",
            routes::EFFECT_ACTIVE_PROPS,
            r##"{"color": {"type": "color", "value": "#00ff00"}}"##,
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_bad_draft_leaves_the_running_draft_alone() {
    let (router, app, engine, _home, _folder, _clock) = build_with_clock();
    prior_effect(&app, "solid", color("#112233")).await;
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);
    let hash = canvas_daemon::user_effects::source_hash(MOVING);

    let response = put_draft(router.clone(), NO_SCRIPT, serde_json::json!({})).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let message = json(response).await["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("parse"), "the parser's message is kept: {message}");

    let state = draft_state(router).await;
    assert!(state.active, "the good draft still runs");
    assert_eq!(state.source_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(running(&engine).0, Some(format!("draft:{hash}")));
}

#[tokio::test]
async fn a_beat_with_new_props_updates_the_lights_without_a_reload() {
    let (router, app, engine, _home, _folder, _clock) = build_with_clock();
    prior_effect(&app, "solid", color("#112233")).await;
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);
    assert!(running(&engine).1.is_empty(), "no props on the first beat");

    let response = put_draft(
        router.clone(),
        MOVING,
        serde_json::json!({ "tint": { "type": "color", "value": "#ff0000" } }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let state: canvas_proto::api::DraftStatus =
        serde_json::from_value(json(response).await).expect("a DraftStatus");
    assert_eq!(state.id.as_deref(), Some(format!("draft:{}", canvas_daemon::user_effects::source_hash(MOVING)).as_str()));
    let (id, props) = running(&engine);
    assert_eq!(id, state.id, "the same draft still runs");
    assert_eq!(props.len(), 1, "the new props reached the engine: {props:?}");
}

#[tokio::test]
async fn a_draft_with_no_saved_effect_goes_black_on_stop() {
    let (router, _app, engine, _home, _folder, _clock) = build_with_clock();
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);
    assert_eq!(
        router
            .clone()
            .oneshot(request("DELETE", routes::EFFECT_DRAFT))
            .await
            .expect("the router answers")
            .status(),
        StatusCode::OK
    );
    let (id, props) = running(&engine);
    assert_eq!(id.as_deref(), Some("solid"), "the layout had no effect: black");
    assert_eq!(
        props,
        canvas_daemon::draft::black(),
        "black is the same fallback the onboarding restore uses"
    );
}

#[tokio::test]
async fn a_frame_that_never_returns_is_stopped_and_the_api_still_answers() {
    let runaway = r##"<head><title>Runaway frame</title></head>
<body><canvas id="c" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("c");
  var ctx = c.getContext("2d");
  ctx.fillStyle = "#00ff00";
  ctx.fillRect(0, 0, 320, 200);
  function draw() { while (true) {} }
  window.requestAnimationFrame(draw);
</script>"##;
    let (router, _app, _home, _folder) = build();
    let pending = tokio::spawn(post_preview(router.clone(), preview_body(runaway, serde_json::json!({}))));
    // The API answers while the preview's frame is stopped at its deadline on a blocking
    // thread: the render loop's thread is not the API's.
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

    // A frame is given one whole frame at the slowest rate (100 ms), so the render is bounded.
    let response = tokio::time::timeout(std::time::Duration::from_secs(60), pending)
        .await
        .expect("the preview answers; the runaway frame does not hold it")
        .expect("the task ends");
    assert_eq!(response.status(), StatusCode::OK, "the last good loop is still served");
    let answer = json(response).await;
    let errors: Vec<String> = answer["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .filter_map(|e| e.as_str().map(str::to_string))
        .collect();
    assert!(
        errors.iter().any(|e| e.contains("ran longer than")),
        "the frame deadline is reported: {errors:?}"
    );
    assert_eq!(thumbnail(&answer).len(), 6 + 24 * 64 * 40 * 3);
}

#[tokio::test]
async fn a_draft_writes_nothing_to_the_saved_files_or_the_effects_folder() {
    let (router, app, engine, home, folder, _clock) = build_with_clock();
    prior_effect(&app, "solid", color("#112233")).await;
    {
        let store = app.store.read().await;
        store.save_effects().expect("effects.json is written");
        store.save_layouts().expect("layouts.json is written");
    }
    let snapshot = || {
        let effects = std::fs::read(home.path.join("effects.json")).expect("effects.json");
        let layouts = std::fs::read(home.path.join("layouts.json")).expect("layouts.json");
        let files: Vec<String> = std::fs::read_dir(&folder)
            .expect("the effects folder")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        (effects, layouts, files)
    };
    let before = snapshot();

    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);
    assert!(draft_state(router.clone()).await.active);
    // A beat with new code rebuilds the draft; it still saves nothing.
    assert_eq!(
        put_draft(router.clone(), &MOVING.replace("#ffcc00", "#00ff00"), serde_json::json!({}))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request("DELETE", routes::EFFECT_DRAFT))
            .await
            .expect("the router answers")
            .status(),
        StatusCode::OK
    );

    assert_eq!(snapshot(), before, "a draft never writes the effects or layouts file");
    assert!(running(&engine).0.as_deref() == Some("solid"));
    let active = app.active_layout().await.and_then(|layout| layout.effect);
    assert_eq!(
        active.map(|effect| effect.id),
        Some("solid".to_string()),
        "the draft never became the layout's effect"
    );
}

#[tokio::test]
async fn a_draft_body_over_the_limit_is_refused() {
    let (router, _app, _engine, _home, _folder, _clock) = build_with_clock();
    let html = "a".repeat(canvas_daemon::api::effects::IMPORT_BODY_LIMIT);
    let response = put_draft(router, &html, serde_json::json!({})).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn the_draft_route_needs_the_token() {
    let (router, _app, _engine, _home, _folder, _clock) = build_with_clock();
    for method in ["GET", "PUT", "DELETE"] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(routes::EFFECT_DRAFT)
                    .header("host", "127.0.0.1:9430")
                    .body(Body::empty())
                    .expect("the request builds"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method}");
    }
}

#[tokio::test]
async fn a_draft_with_a_syntax_error_is_refused() {
    let broken = "<head><title>Syntax</title></head><body><canvas id=\"c\"></canvas></body><script>\nvar c = document.getElementById('c');\nvar ctx = c.getContext('2d');\nvar = ;\n</script>";
    let (router, _app, _home, _folder) = build();
    let response = post_preview(router, preview_body(broken, serde_json::json!({}))).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let message = json(response).await["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("eval_script:4"), "the runtime's line is kept: {message}");
}
// ---------------------------------------------------------------- round-1 fixes

/// Writes a user effect into the folder and rescans, so the library has it.
fn add_user_effect(app: &Arc<App>, folder: &std::path::Path, file: &str, html: &str) {
    std::fs::write(folder.join(file), html).expect("the effect is written");
    app.user_effects.rescan(folder);
}

#[tokio::test]
async fn deleting_a_drafts_prior_ends_the_draft_at_once() {
    let (router, app, engine, _home, folder, _clock) = build_with_clock();
    add_user_effect(&app, &folder, "foo.html", MOVING);
    prior_effect(&app, "html:foo", canvas_proto::props::Props::new()).await;
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);

    // The prior's file goes through the route while the draft holds the lights. The draft is
    // in memory only, and deleting what the lights show is what the owner asked for, so the
    // draft ends here (black) instead of the route refusing with a 409.
    let deleted = router
        .clone()
        .oneshot(request("DELETE", "/api/v1/effects/html:foo"))
        .await
        .expect("the router answers");
    assert_eq!(deleted.status(), StatusCode::OK);

    let state = draft_state(router.clone()).await;
    assert!(!state.active, "the delete ended the draft");
    assert_eq!(running(&engine).0.as_deref(), Some("solid"), "the lights are black");
    let stopped = router
        .oneshot(request("DELETE", routes::EFFECT_DRAFT))
        .await
        .expect("the router answers");
    assert_eq!(stopped.status(), StatusCode::OK, "a Stop after the delete is a no-op");
}

#[tokio::test]
async fn a_prior_that_vanishes_ends_the_draft_on_expiry() {
    let (router, app, engine, _home, folder, clock) = build_with_clock();
    add_user_effect(&app, &folder, "foo.html", MOVING);
    prior_effect(&app, "html:foo", canvas_proto::props::Props::new()).await;
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);

    // The file goes behind the daemon's back (Explorer, a sync client): the rescan drops it
    // from the library while the draft still holds the lights.
    std::fs::remove_file(folder.join("foo.html")).expect("the effect file is removed");
    app.user_effects.rescan(&folder);

    clock.advance(canvas_proto::consts::DRAFT_LEASE);
    let state = draft_state(router).await;
    assert!(!state.active, "the lease ended the draft instead of stranding it");
    let note = state.note.as_deref().unwrap_or_default();
    assert!(note.contains("html:foo"), "the expiry answer says what happened: {note}");
    assert_eq!(
        running(&engine).0.as_deref(),
        Some("solid"),
        "the prior could not be built, so the lights are black"
    );
}

#[tokio::test]
async fn a_prior_that_vanishes_stop_answers_200_with_a_note() {
    let (router, app, engine, _home, folder, _clock) = build_with_clock();
    add_user_effect(&app, &folder, "foo.html", MOVING);
    prior_effect(&app, "html:foo", canvas_proto::props::Props::new()).await;
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);
    std::fs::remove_file(folder.join("foo.html")).expect("the effect file is removed");
    app.user_effects.rescan(&folder);

    let response = router
        .oneshot(request("DELETE", routes::EFFECT_DRAFT))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK, "Stop answers 200, not 500");
    let body = json(response).await;
    assert_eq!(body["active"], serde_json::json!(false));
    let note = body["note"].as_str().unwrap_or_default();
    assert!(note.contains("html:foo"), "the note names the prior: {note}");
    assert_eq!(running(&engine).0.as_deref(), Some("solid"), "black is on the lights");
}

#[tokio::test]
async fn a_prior_that_vanishes_stop_after_an_expiry_still_answers_200() {
    let (router, app, engine, _home, folder, clock) = build_with_clock();
    add_user_effect(&app, &folder, "foo.html", MOVING);
    prior_effect(&app, "html:foo", canvas_proto::props::Props::new()).await;
    assert_eq!(put_draft(router.clone(), MOVING, serde_json::json!({})).await.status(), StatusCode::OK);
    std::fs::remove_file(folder.join("foo.html")).expect("the effect file is removed");
    app.user_effects.rescan(&folder);

    clock.advance(canvas_proto::consts::DRAFT_LEASE);
    let _ = canvas_daemon::draft::expire_if_due(&app).await;
    let response = router
        .oneshot(request("DELETE", routes::EFFECT_DRAFT))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK, "Stop after an expiry answers 200");
    assert_eq!(running(&engine).0.as_deref(), Some("solid"));
}

