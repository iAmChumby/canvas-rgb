//! Safe start on the fakes (#577 section 9): the first `Hello`, the mode latch, the in-memory
//! Breathing scene and the answer-before-relaunch ordering of the PUT route. No pipe, process,
//! chip or light is real: the app is built from the in-test engine and adapter.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::pipe_client::{self, PipeError, PipeLink};
use canvas_daemon::state::App;
use canvas_proto::api::{LastExit, SafeStartMode, SafeStartPut};
use canvas_proto::pipe::{PipeRequest, PipeResponse, PROTOCOL_VERSION};
use canvas_proto::props::Props;
use common::{a_scene, FakeAdapter, FakeEngine};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const WAIT: Duration = Duration::from_secs(10);

struct TempHome(std::path::PathBuf);

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("canvas-rgb-safe-start-{}-{}", std::process::id(), {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        }));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build() -> (Arc<App>, Arc<FakeEngine>, TempHome) {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.0);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        TOKEN.into(),
        "0.1.0-test".into(),
        tx,
    ));
    (app, engine, home)
}

/// A service that answers the start `Hello` with a chosen decision.
struct StubPipe {
    mode: SafeStartMode,
    last: LastExit,
    hello_seen: Arc<Mutex<u32>>,
}

#[async_trait::async_trait]
impl PipeLink for StubPipe {
    async fn exchange(&self, request: &PipeRequest) -> Result<PipeResponse, PipeError> {
        if matches!(request, PipeRequest::Hello { .. }) {
            *self.hello_seen.lock().unwrap() += 1;
            return Ok(PipeResponse::Hello {
                protocol: PROTOCOL_VERSION,
                service_version: "test".into(),
                start_mode: self.mode,
                last_exit: self.last,
            });
        }
        Ok(PipeResponse::Ok)
    }
}

struct FailingPipe;

#[async_trait::async_trait]
impl PipeLink for FailingPipe {
    async fn exchange(&self, _: &PipeRequest) -> Result<PipeResponse, PipeError> {
        Err(PipeError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "no fake service")))
    }
}

#[test]
fn an_old_hello_without_the_new_fields_reads_normal_and_unknown() {
    // `#[serde(default)]` on the spine fields is what lets an older service still be parsed.
    let old = r#"{"result":"hello","protocol":1,"service_version":"0.19.0"}"#;
    let response: PipeResponse = serde_json::from_str(old).unwrap();
    match response {
        PipeResponse::Hello { start_mode, last_exit, .. } => {
            assert_eq!(start_mode, SafeStartMode::Normal);
            assert_eq!(last_exit, LastExit::Unknown);
        }
        other => panic!("not a Hello: {other:?}"),
    }
}

#[tokio::test]
async fn no_pipe_and_an_old_service_both_start_normally() {
    for pipe in [None, Some((SafeStartMode::Normal, LastExit::Unknown))] {
        let (mut app, _engine, _home) = build();
        let slot = Arc::get_mut(&mut app).unwrap();
        match pipe {
            None => slot.pipe = Arc::new(FailingPipe),
            Some((mode, last)) => {
                slot.pipe = Arc::new(StubPipe { mode, last, hello_seen: Arc::new(Mutex::new(0)) });
            }
        }
        assert_eq!(pipe_client::read_start_decision(&app).await, SafeStartMode::Normal);
        assert_eq!(app.start_mode(), SafeStartMode::Normal);
        assert_eq!(app.last_exit(), LastExit::Unknown);
        assert!(app.safe_start_status().is_none());
    }
}

#[tokio::test]
async fn the_mode_is_latched_from_the_first_hello() {
    let (mut app, _engine, _home) = build();
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
    app.latch_start(SafeStartMode::Normal, LastExit::Clean);
    assert_eq!(app.start_mode(), SafeStartMode::Safe, "a later reply changed the mode mid-run");
    assert_eq!(app.last_exit(), LastExit::Unclean);
    let status = app.safe_start_status().unwrap();
    assert!(status.active);
    assert_eq!(status.reason, Some(canvas_proto::api::SafeStartReason::UncleanOnce));
    assert_eq!(status.unclean_streak, 1);

    let hello_seen = Arc::new(Mutex::new(0));
    Arc::get_mut(&mut app).unwrap().pipe = Arc::new(StubPipe {
        mode: SafeStartMode::Normal,
        last: LastExit::Clean,
        hello_seen: hello_seen.clone(),
    });
    assert_eq!(pipe_client::read_start_decision(&app).await, SafeStartMode::Safe);
    assert_eq!(*hello_seen.lock().unwrap(), 1, "the latch did not re-read the pipe");
}

#[tokio::test]
async fn a_safe_mode_plays_breathing_on_every_saved_light_without_writing_the_store() {
    let (app, engine, _home) = build();
    let (lights, scenes) = a_scene("saved-effect", &Props::new());
    {
        let mut store = app.store.write().await;
        store.lights = lights.clone();
        store.scenes = scenes.clone();
    }
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
    *app.safe_effect.lock().unwrap() = Some("breathing".into());

    canvas_daemon::scenes::play_builtin_on_all(&app, "breathing").await.unwrap();
    assert_eq!(engine.scene.lock().unwrap().len(), 1);
    assert_eq!(engine.scene.lock().unwrap()[0].1.as_deref(), Some("breathing"));
    assert_eq!(engine.scene_lights.lock().unwrap().len(), lights.lights.len());
    assert_eq!(engine.scene_members.lock().unwrap()[0], vec!["fake:x#z0".to_string()]);

    let stored = app.store.read().await;
    assert_eq!(stored.scenes, scenes, "the saved scene was changed");
    assert_eq!(stored.lights, lights, "the saved lights were changed");
    drop(stored);

    // What status reports is the synthetic scene, not the one that would play after a restart.
    let status = app.status().await.unwrap();
    assert!(status.safe_start.unwrap().active);
    assert_eq!(status.active_scene.unwrap().name, "Safe start");
    assert_eq!(status.active_effects, vec!["breathing".to_string()]);
}

#[tokio::test]
async fn a_restart_replays_a_chain_restore_record_left_by_the_killed_daemon() {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.0);
    let engine = Arc::new(FakeEngine::default());
    let adapter = FakeAdapter::default();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(store, engine, Box::new(adapter), TOKEN.into(), "test".into(), tx));

    // A mode-change restart kills the daemon without `quit_sequence`, so an open setup's record
    // survives. The next start replays it when its device list arrives (#577 section 6).
    let record = canvas_daemon::chain_records::RestoreRecord {
        device_id: canvas_proto::device::DeviceId("fake:x".into()),
        zone_index: 0,
        leds: 60,
    };
    canvas_daemon::chain_records::write_restore(&home.0, &record).unwrap();
    let path = home.0.join(canvas_proto::scene::CHAIN_RESTORE_FILE);
    assert!(path.exists());

    let logs = common::global_logs();
    canvas_daemon::api::chains::replay_leftovers(&app).await;
    let text = logs.text();
    assert!(
        text.contains("a chain setup was interrupted: the header's earlier length is back"),
        "the record was not replayed: {text}"
    );
    assert!(!path.exists(), "the replayed record was not cleared");
}

/// A fake pipe that records the request and then waits for the test to release it. The PUT must
/// answer while this exchange is still held.
struct HeldPipe {
    seen: Arc<Mutex<Option<PipeRequest>>>,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
}

#[async_trait::async_trait]
impl PipeLink for HeldPipe {
    async fn exchange(&self, request: &PipeRequest) -> Result<PipeResponse, PipeError> {
        if matches!(request, PipeRequest::Hello { .. }) {
            return Ok(PipeResponse::Hello {
                protocol: PROTOCOL_VERSION,
                service_version: "test".into(),
                start_mode: SafeStartMode::Normal,
                last_exit: LastExit::Unknown,
            });
        }
        *self.seen.lock().unwrap() = Some(request.clone());
        let _ = self.entered.send(());
        let mut slot = self.release.lock().await;
        if let Some(rx) = slot.take() {
            let _ = rx.await;
        }
        Ok(PipeResponse::Ok)
    }
}

#[tokio::test]
async fn the_put_answers_before_the_service_sees_the_relaunch() {
    let (mut app, _engine, _home) = build();
    let seen = Arc::new(Mutex::new(None));
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    Arc::get_mut(&mut app).unwrap().pipe = Arc::new(HeldPipe {
        seen: seen.clone(),
        entered: entered_tx,
        release: Arc::new(tokio::sync::Mutex::new(Some(release_rx))),
    });
    let router = api::router(app.clone());

    let body = serde_json::to_string(&SafeStartPut { mode: SafeStartMode::Normal }).unwrap();
    let request = Request::builder()
        .method("PUT")
        .uri(canvas_proto::api::routes::SAFE_START)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let response = tokio::time::timeout(WAIT, router.oneshot(request)).await.expect("the PUT hung").unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
    let answer: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(answer["mode"], "normal");
    assert_eq!(answer["restarting"], true);

    // The pipe task has started but is still held; the HTTP reply was already complete.
    tokio::time::timeout(WAIT, entered_rx.recv()).await.expect("the pipe message was never sent").unwrap();
    assert!(seen.lock().unwrap().is_some(), "the pipe message was never recorded");
    assert!(matches!(*seen.lock().unwrap(), Some(PipeRequest::LeaveSafeStart)));
    let _ = release_tx.send(());

    // The daemon does not run quit_sequence for this: no DaemonQuitting goes over the pipe.
    assert!(!matches!(*seen.lock().unwrap(), Some(PipeRequest::DaemonQuitting)));
}
