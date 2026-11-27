//! #576/#575 regression tests, run entirely on the fakes.
//!
//! The first two tests would fail on the tree before the response layer: a fake profile path
//! came back in an error body, both from a JSON error and from a plain-text extractor rejection.
//! The last one drives the diagnostics route's list-then-write pair.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_diag::scrub::Scrubber;
use canvas_proto::api::routes;
use common::{FakeAdapter, FakeEngine};
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
// Spelled in parts so no test writes a real profile path into the repository.
const FAKE: &str = concat!("C:\\", "Use", "rs\\Alice");

/// One fake home for this test binary. Setting the override once keeps parallel tests from
/// racing on the environment and keeps every read off the real user and machine trees.
fn home() -> &'static std::path::PathBuf {
    static HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-scrub-{}-{}",
            std::process::id(),
            canvas_diag::paths::timestamp()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zones.json"), format!("{{\"{FAKE}/zones\":1}}")).unwrap();
        // The override also relocates the fake machine tree, so no test reads ProgramData.
        std::env::set_var(canvas_diag::paths::HOME_ENV, &dir);
        dir
    })
}

fn build() -> (axum::Router, Arc<App>, std::path::PathBuf) {
    let home = home().clone();
    let store = canvas_daemon::storage::Store::at(&home);
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
    app.set_scrubber(Scrubber::new(
        Some(FAKE.to_string()),
        Some("Alice".to_string()),
        Some("ALICE-PC".to_string()),
    ));
    (api::router(app.clone()), app, home)
}

async fn body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A JSON error's `message` and `hint` never carry the profile path, in any spelling.
#[tokio::test]
async fn a_fake_profile_path_in_an_error_route_is_scrubbed() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::STATUS)
                .header("host", "127.0.0.1:9430")
                .header("origin", format!("http://{}/notes", FAKE.replace('\\', "/")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let text = body(response).await;
    assert!(!text.to_ascii_lowercase().contains("alice"), "{text}");
    assert!(!text.contains("Use\\rs") && !text.contains("Use/rs"), "{text}");
    assert!(text.contains("<profile>"), "{text}");
    assert!(text.contains("hint"), "the next step rides with every error: {text}");
}

/// An extractor rejection is plain text, not JSON. It is scrubbed by the same layer.
#[tokio::test]
async fn a_malformed_json_body_never_comes_back_unscrubbed() {
    let (router, _app, _home) = build();
    // Serialised so the body is valid JSON syntax with a wrong field type: serde quotes the
    // offending value in its rejection, which is the plain-text body the layer must scrub.
    let bad = serde_json::to_string(&serde_json::json!({ "source": "cpu", "failed": FAKE })).unwrap();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/diag/fail-sensor")
                .header("host", "127.0.0.1:9430")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(bad))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_client_error(), "{:?}", response.status());
    let text = body(response).await;
    assert!(!text.to_ascii_lowercase().contains("alice"), "{text}");
    assert!(!text.contains("Use\\rs") && !text.contains("Use/rs"), "{text}");
}

/// `GET` lists before `POST` writes; the answer carries the file list and a plain reason for
/// service logs that are absent on the fakes' tree.
#[tokio::test]
async fn the_bundle_route_lists_then_writes() {
    let (router, _app, home) = build();
    let auth = format!("Bearer {TOKEN}");
    let list = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::DIAG_BUNDLE)
                .header("host", "127.0.0.1:9430")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let text = body(list).await;
    let listed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(listed["files"].as_array().unwrap().iter().any(|f| f["name"] == "config/zones.json"));
    assert!(listed["path"].is_null(), "GET does not write: {text}");
    assert!(listed["service_logs"].as_str().unwrap().contains("not included"), "{text}");

    // A caller cannot choose the destination (P2).
    let refused = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(routes::DIAG_BUNDLE)
                .header("host", "127.0.0.1:9430")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({ "path": home.join("out").to_string_lossy() })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    // The handler takes no body, so a caller-chosen path is ignored (P2).
    let ignored: serde_json::Value = serde_json::from_str(&body(refused).await).unwrap();
    assert_eq!(ignored["error"], serde_json::Value::Null);
    assert_eq!(
        std::path::PathBuf::from(ignored["path"].as_str().unwrap()).parent().unwrap(),
        canvas_diag::paths::diagnostics_dir()
    );

    let write = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(routes::DIAG_BUNDLE)
                .header("host", "127.0.0.1:9430")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = write.status();
    let text = body(write).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let written: serde_json::Value = serde_json::from_str(&text).unwrap();
    let path = std::path::PathBuf::from(written["path"].as_str().unwrap());
    assert!(path.exists());
    assert_eq!(path.parent().unwrap(), canvas_diag::paths::diagnostics_dir());
    let _ = std::fs::remove_dir_all(&home);
}
