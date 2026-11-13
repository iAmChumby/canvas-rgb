//! Class `cause:unchecked-state` (#19): a step whose failure the code used to drop must now say
//! so, in the log or in the reply.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::api::routes;
use common::{FakeAdapter, FakeEngine, LogBuf};
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct TempHome(std::path::PathBuf);

impl TempHome {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("canvas-rgb-unchecked-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp directory is created");
        Self(path)
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

type Built = (axum::Router, Arc<App>, TempHome, Option<tokio::sync::mpsc::UnboundedReceiver<()>>);

/// The router and state on fakes. With `keep_quit_rx` false the quit channel's receiver is
/// dropped, as when the main task has gone.
fn build(tag: &str, keep_quit_rx: bool) -> Built {
    let home = TempHome::new(tag);
    let store = canvas_daemon::storage::Store::at(&home.0);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        Arc::new(FakeEngine::default()),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    (api::router(app.clone()), app, home, keep_quit_rx.then_some(rx))
}

/// The router and state on fakes, with a store that holds one scene of one group over one
/// light: the shape every scene route reads.
fn build_with_a_scene(tag: &str) -> Built {
    let home = TempHome::new(tag);
    let mut store = canvas_daemon::storage::Store::at(&home.0);
    let (lights, scenes) = common::a_scene("solid", &canvas_proto::props::Props::new());
    store.lights = lights;
    store.scenes = scenes;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        Arc::new(FakeEngine::default()),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    (api::router(app.clone()), app, home, Some(rx))
}

fn request(method: &str, path: &str, body: Option<&str>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .expect("the request builds")
}

/// A quit request that cannot reach the main task (its receiver is gone) used to be dropped with
/// `let _ =` while the reply said `quitting: true`. Now the reply is an error and the log says why.
#[tokio::test]
async fn a_quit_that_cannot_reach_the_main_task_is_reported_not_claimed() {
    let logs = LogBuf::default();
    let _log = logs.install();
    let (router, app, _home, rx) = build("quit-lost", false);
    assert!(rx.is_none());

    let response = router.oneshot(request("POST", routes::QUIT, None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body = String::from_utf8_lossy(&bytes);
    assert!(!body.contains("\"quitting\":true"), "{body}");
    assert_eq!(app.shutting_down.load(std::sync::atomic::Ordering::SeqCst), 0, "the daemon is still running");
    let text = logs.text();
    assert!(text.contains("WARN") && text.contains("the quit request could not reach the main task"), "{text}");
}

/// The same route with a live receiver still answers `quitting: true` and delivers the signal.
#[tokio::test]
async fn a_quit_that_reaches_the_main_task_still_says_quitting() {
    let (router, _app, _home, rx) = build("quit-ok", true);
    let mut rx = rx.unwrap();
    let response = router.oneshot(request("POST", routes::QUIT, None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(rx.try_recv().is_ok(), "the main task was asked to stop");
}

/// A scene whose group plays an effect that does not exist keeps the failure in the state the
/// scene list reports: the group is dark and named, instead of failing silently.
#[tokio::test]
async fn a_scene_group_whose_effect_cannot_be_built_is_named_in_the_failures() {
    let (router, app, _home, _rx) = build_with_a_scene("effect-fails");
    {
        let mut store = app.store.write().await;
        let active = store.scenes.active.clone();
        if let Some(scene) = store.scenes.scenes.iter_mut().find(|s| s.id == active) {
            if let Some(group) = scene.groups.first_mut() {
                group.plays = canvas_proto::scene::Plays::Effect {
                    effect: "no-such-effect".into(),
                    props: Default::default(),
                };
            }
        }
        store.save_scenes().expect("the scenes are saved");
    }
    // The scene the store now holds is applied to the engine.
    canvas_daemon::scenes::apply_active_ungated(&app, false).await;
    let failures = app.scene.failures();
    assert!(
        failures.values().any(|why| why.contains("no-such-effect")),
        "the failed group is named in the scene's failures: {failures:?}"
    );
    router
        .oneshot(request("GET", routes::STATUS, None))
        .await
        .expect("the router answers");
}


