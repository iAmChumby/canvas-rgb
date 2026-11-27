//! Issue #580: hardware notes on the fakes. No real device, service or config folder is touched.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::faults::{clear_fan_if_healthy, clear_openrgb_if_healthy, key};
use canvas_daemon::state::App;
use canvas_proto::api::{routes, FaultKind, LinkState, Status};
use canvas_proto::fans::{ChipProblem, FansStatus};
use common::{FakeAdapter, FakeEngine};
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-faults-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the test home is created");
        Self { path }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

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

fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

async fn body<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.expect("the body is read");
    serde_json::from_slice(&bytes).expect("the body is the answer type")
}

fn bad_fans(problem: ChipProblem) -> FansStatus {
    let mut status = canvas_daemon::fakes::fake_fans_status();
    status.chip_problem = Some(problem);
    status
}

async fn status_of(router: &axum::Router) -> Status {
    body::<Status>(router.clone().oneshot(request("GET", routes::STATUS)).await.expect("the router answers")).await
}

#[tokio::test]
async fn faults_appear_on_a_bad_state_and_not_on_connecting_or_unchecked() {
    let (router, app, _home) = build();
    *app.fans.write().await = Some(bad_fans(ChipProblem::PawnIoUnavailable));
    *app.openrgb.write().await = LinkState::Down;
    let status = status_of(&router).await;
    assert_eq!(status.faults.len(), 2, "both definite bad states are notes");
    assert!(status.faults.iter().any(|f| f.kind == FaultKind::FanChip));
    assert!(status.faults.iter().any(|f| f.kind == FaultKind::OpenRgbDown));

    *app.openrgb.write().await = LinkState::Connecting;
    let status = status_of(&router).await;
    assert_eq!(status.faults.len(), 1, "connecting is not a fault");

    let mut unchecked = bad_fans(ChipProblem::ChipNotFound);
    unchecked.chip_checked = false;
    *app.fans.write().await = Some(unchecked);
    assert!(status_of(&router).await.faults.is_empty(), "an unchecked service is silent");
}

#[tokio::test]
async fn the_dismiss_route_hides_the_page_note_and_keeps_the_strip_count() {
    let (router, app, home) = build();
    *app.openrgb.write().await = LinkState::Connected;
    *app.fans.write().await = Some(bad_fans(ChipProblem::PawnIoUnavailable));
    let fault_key = status_of(&router).await.faults[0].key.clone();
    assert_eq!(fault_key, key(FaultKind::FanChip, None, "pawn_io_unavailable"));

    let response = router
        .clone()
        .oneshot(request("POST", &routes::FAULT_DISMISS.replace("{key}", &fault_key)))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let answer: serde_json::Value = body(response).await;
    assert_eq!(answer["ok"], true);

    let status = status_of(&router).await;
    assert_eq!(status.faults.len(), 1, "a dismissed fault stays in the list");
    assert!(status.faults[0].dismissed);
    let saved = std::fs::read_to_string(home.path.join("config.toml")).expect("the config was written");
    assert!(saved.contains(&fault_key), "the key is stored in config.toml");

    // A bad key is refused, so arbitrary text never enters the store.
    let response = router
        .oneshot(request("POST", "/api/v1/faults/not-a-key/dismiss"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_clear_needs_an_observed_healthy_state_and_writes_only_when_the_key_is_present() {
    let (_router, app, home) = build();
    let config_path = home.path.join("config.toml");
    let fan_key = key(FaultKind::FanChip, None, "pawn_io_unavailable");
    let openrgb_key = key(FaultKind::OpenRgbDown, None, "down");
    // An unknown field survives a load and is dropped by a save, so it says whether a write
    // happened without leaning on the clock.
    let seed = format!(
        "schema_version = 1\napi_port = 9430\nopenrgb_port = 16742\nfirst_run_done = true\nzone_panel_dismissed = true\nonboarding_complete = true\ndismissed_faults = [\"{fan_key}\", \"{openrgb_key}\"]\nwrite_marker = \"keep-me\"\n[settings]\nfps = 30\nblack_on_lock = false\non_quit = \"black\"\nbrightness = 100\npower = \"running\"\n"
    );
    std::fs::write(&config_path, seed).expect("the test config is written");
    let mut store = app.store.write().await;
    store.config = toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    drop(store);

    let mut unchecked = canvas_daemon::fakes::fake_fans_status();
    unchecked.chip_checked = false;
    clear_fan_if_healthy(&app, &unchecked).await;
    clear_openrgb_if_healthy(&app, LinkState::Connecting).await;
    clear_openrgb_if_healthy(&app, LinkState::Down).await;
    let text = std::fs::read_to_string(&config_path).unwrap();
    assert!(text.contains("write_marker"), "no healthy observation wrote nothing");
    assert!(text.contains(&fan_key) && text.contains(&openrgb_key));

    clear_fan_if_healthy(&app, &bad_fans(ChipProblem::ChipNotFound)).await;
    let text = std::fs::read_to_string(&config_path).unwrap();
    assert!(text.contains("write_marker"), "a still-bad chip wrote nothing");

    // The cause change's old key waits for a healthy chip, not merely for absence.
    let healthy = canvas_daemon::fakes::fake_fans_status();
    clear_fan_if_healthy(&app, &healthy).await;
    clear_openrgb_if_healthy(&app, LinkState::Connected).await;
    let text = std::fs::read_to_string(&config_path).unwrap();
    assert!(!text.contains("write_marker"), "a clear saved the config");
    assert!(!text.contains(&fan_key) && !text.contains(&openrgb_key));

    // A second clear with nothing present is a no-op and writes no file again.
    clear_fan_if_healthy(&app, &healthy).await;
    let text = std::fs::read_to_string(&config_path).unwrap();
    assert!(!text.contains(&fan_key), "the key stays gone");
    assert!(app.store.read().await.config.dismissed_faults.is_empty());
}

#[tokio::test]
async fn an_old_config_without_dismissed_faults_loads() {
    let home = TempHome::new();
    std::fs::write(
        home.path.join("config.toml"),
        "schema_version = 1\napi_port = 9430\nopenrgb_port = 16742\nfirst_run_done = true\nzone_panel_dismissed = true\nonboarding_complete = true\n\n[settings]\nfps = 30\nblack_on_lock = false\non_quit = \"black\"\nbrightness = 100\npower = \"running\"\n",
    )
    .expect("the old config is written");
    let store = canvas_daemon::storage::Store::at(&home.path);
    assert!(store.config.dismissed_faults.is_empty());
    // A status read with that store still has no notes and never writes the file back.
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(store, Arc::new(FakeEngine::default()), Box::new(FakeAdapter::default()), TOKEN.to_string(), "test".into(), tx));
    *app.openrgb.write().await = LinkState::Connected;
    let router = api::router(app);
    assert!(status_of(&router).await.faults.is_empty());
    let text = std::fs::read_to_string(home.path.join("config.toml")).unwrap();
    assert!(!text.contains("dismissed_faults"));
}
