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

#[tokio::test]
async fn the_preview_needs_a_token() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::PREVIEW)
                .header("host", "127.0.0.1:9430")
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
