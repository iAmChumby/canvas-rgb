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
/// The state lives in a temporary directory, so the tests never touch Luke's real
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
    assert_eq!(effects.len(), 12, "there are exactly 12 effects");
    let ids: Vec<&str> =
        effects.iter().filter_map(|e| e.get("id").and_then(|v| v.as_str())).collect();
    for id in canvas_proto::EFFECT_IDS {
        assert!(ids.contains(&id), "{id} is in the list");
    }
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
async fn user_html_effects_are_listed_after_the_twelve_built_ins() {
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
    assert_eq!(ids.len(), 13);
    for (i, id) in canvas_proto::EFFECT_IDS.iter().enumerate() {
        assert_eq!(ids[i], *id, "built-in {i} comes first");
    }
    assert_eq!(ids[12], "html:fancy");
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
async fn the_thumbnail_of_poison_has_the_binary_format() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(request("GET", "/api/v1/effects/poison/thumbnail"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = body_bytes(response).await;
    assert_thumbnail_format(&bytes);
    // Poison is not a flat colour after a second of warm-up.
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

/// A fake service. It answers every request the same way.
struct FakePipe {
    reply: FakeReply,
}

#[derive(Clone, Copy)]
enum FakeReply {
    Ok,
    Error(&'static str),
    Down,
}

#[async_trait::async_trait]
impl canvas_daemon::pipe_client::PipeLink for FakePipe {
    async fn exchange(
        &self,
        _request: &canvas_proto::pipe::PipeRequest,
    ) -> Result<canvas_proto::pipe::PipeResponse, canvas_daemon::pipe_client::PipeError> {
        match self.reply {
            FakeReply::Ok => Ok(canvas_proto::pipe::PipeResponse::Ok),
            FakeReply::Error(code) => Ok(canvas_proto::pipe::PipeResponse::Error {
                code: code.to_string(),
                message: format!("the service said {code}"),
            }),
            FakeReply::Down => Err(canvas_daemon::pipe_client::PipeError::Io(
                std::io::Error::new(std::io::ErrorKind::NotFound, "the pipe is not up"),
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
async fn the_api_source_shows_the_put_value_and_blank_until_then() {
    let (router, app, _home, log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::Api, DisplayUnit::Celsius, 1, None)).await;
    let mut now = Instant::now();
    now += pass(&app, now).await;
    assert_eq!(log.lock().unwrap().writes.last().unwrap().1.value, None, "blank before any value");

    let set = |v: serde_json::Value| router.clone().oneshot(request_with_body("PUT", &value_path(), &v.to_string()));
    assert_eq!(set(serde_json::json!({ "value": 12.5 })).await.unwrap().status(), StatusCode::OK);
    pass(&app, now).await;
    assert_eq!(log.lock().unwrap().writes.last().unwrap().1.value, Some(12.5));

    let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert_eq!(status[0]["shown"]["value"], serde_json::json!(12.5));
    assert_eq!(status[0]["info"]["connected"], serde_json::json!(true));

    assert_eq!(set(serde_json::json!({ "value": null })).await.unwrap().status(), StatusCode::OK);
    now += Duration::from_millis(250);
    pass(&app, now).await;
    assert_eq!(log.lock().unwrap().writes.last().unwrap().1.value, None, "null blanks it");
    let status = json(router.clone().oneshot(request("GET", routes::DISPLAYS)).await.unwrap()).await;
    assert!(status[0]["shown"].is_null(), "a blank reports shown: null");
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
async fn a_value_the_display_refuses_shows_blank_and_is_logged_once() {
    let (router, app, _home, log) = build_with_display(true);
    log.lock().unwrap().refuse_at = Some(1000.0);
    put_config(&router, &config(DisplaySource::Fixed { value: 1234.0 }, DisplayUnit::Celsius, 0, None)).await;
    let mut now = Instant::now();
    for _ in 0..3 {
        now += pass(&app, now).await;
        now += Duration::from_millis(250);
    }
    {
        let log = log.lock().unwrap();
        assert!(log.writes.iter().all(|(_, v)| v.value.is_none()), "every pass writes blank");
        assert_eq!(log.writes.len(), 3);
        assert_eq!(log.connects, 1, "a refusal keeps the connection");
        assert_eq!(log.disconnects, 0);
    }
    assert_eq!(app.displays.runtime.read().await.refusal_warnings, 1, "the refusal is logged once");

    // A value that fits clears the streak, so the next refusal is logged again.
    put_config(&router, &config(DisplaySource::Fixed { value: 500.0 }, DisplayUnit::Celsius, 0, None)).await;
    pass(&app, now).await;
    assert_eq!(log.lock().unwrap().writes.last().unwrap().1.value, Some(500.0));
    put_config(&router, &config(DisplaySource::Fixed { value: 1234.0 }, DisplayUnit::Celsius, 0, None)).await;
    pass(&app, now + Duration::from_millis(500)).await;
    assert_eq!(app.displays.runtime.read().await.refusal_warnings, 2);
}

#[tokio::test]
async fn a_missing_sensor_reading_shows_blank() {
    let (router, app, _home, log) = build_with_display(true);
    put_config(&router, &config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 1, None)).await;
    // No fan status yet (the service is down): the CPU reading is missing.
    pass(&app, Instant::now()).await;
    assert_eq!(log.lock().unwrap().writes[0].1.value, None);
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
    assert_eq!(status, serde_json::json!([]), "the config is gone");

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
