//! Safe start on the fakes (#577 section 9): the first `Hello`, the mode latch, the in-memory
//! Breathing scene and the answer-before-relaunch ordering of the PUT route. No pipe, process,
//! chip or light is real: the app is built from the in-test engine and adapter.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::pipe_client::{self, PipeError, PipeLink};
use canvas_daemon::state::App;
use canvas_proto::api::{routes, LastExit, Power, SafeStartMode, SafeStartPut};
use canvas_proto::traits::Engine as _;
use canvas_proto::pipe::{PipeRequest, PipeResponse, PROTOCOL_VERSION};
use canvas_proto::props::Props;
use common::{a_scene, FakeAdapter, FakeEngine};
use std::sync::atomic::Ordering;
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
                safe_start: None,
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
    assert_eq!(status.reason, None, "no record from the service: nothing is guessed");
    assert_eq!(status.unclean_streak, 0);

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

    canvas_daemon::scenes::apply_active(&app, false).await;
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
                safe_start: None,
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
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
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

// ---- #577 fix pass 1 ----

async fn saved_solid(app: &Arc<App>) {
    let (lights, scenes) = a_scene("rainbow_wave", &Props::new());
    let mut store = app.store.write().await;
    store.lights = lights;
    store.scenes = scenes;
}

fn playing(engine: &FakeEngine) -> Vec<Option<String>> {
    engine.scene.lock().unwrap().iter().map(|g| g.1.clone()).collect()
}

#[tokio::test]
async fn b3_safe_mode_never_applies_the_saved_scene() {
    let (app, engine, _home) = build();
    saved_solid(&app).await;
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
    canvas_daemon::scenes::apply_for_devices(&app).await;
    assert_eq!(playing(&engine), vec![Some("breathing".to_string())], "after a device list");
    canvas_daemon::scenes::apply_active(&app, true).await;
    assert_eq!(playing(&engine), vec![Some("breathing".to_string())], "after an activation");
}

#[tokio::test]
async fn b4_breathing_starts_when_the_first_lights_arrive() {
    let (app, engine, _home) = build();
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
    canvas_daemon::scenes::apply_for_devices(&app).await;
    assert!(engine.solid_writes.lock().unwrap().is_empty(), "no lights yet is not a failure");
    saved_solid(&app).await;
    canvas_daemon::scenes::apply_for_devices(&app).await;
    assert_eq!(playing(&engine), vec![Some("breathing".to_string())]);
    assert!(engine.solid_writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn m3_the_services_record_and_the_live_fan_status_feed_the_status() {
    let (mut app, _engine, _home) = build();
    Arc::get_mut(&mut app).unwrap().pipe = Arc::new(RecordPipe);
    pipe_client::read_start_decision(&app).await;
    let safe = app.status().await.unwrap().safe_start.unwrap();
    assert_eq!(safe.reason, Some(canvas_proto::api::SafeStartReason::CrashLoop));
    assert_eq!(safe.unclean_streak, 3);
    assert_eq!(safe.since.as_deref(), Some("2026-11-27T10:00:00Z"));
    assert_eq!(safe.fans_on_bios, None, "no fan status yet is unknown");

    let mut fans = canvas_daemon::fakes::fake_fans_status();
    fans.headers.iter_mut().for_each(|h| h.on_bios = false);
    *app.fans.write().await = Some(fans.clone());
    assert_eq!(app.status().await.unwrap().safe_start.unwrap().fans_on_bios, Some(false));
    fans.headers.iter_mut().for_each(|h| h.on_bios = true);
    *app.fans.write().await = Some(fans);
    assert_eq!(app.status().await.unwrap().safe_start.unwrap().fans_on_bios, Some(true));
}

struct RecordPipe;

#[async_trait::async_trait]
impl PipeLink for RecordPipe {
    async fn exchange(&self, _: &PipeRequest) -> Result<PipeResponse, PipeError> {
        Ok(PipeResponse::Hello {
            protocol: PROTOCOL_VERSION,
            service_version: "test".into(),
            start_mode: SafeStartMode::Safe,
            last_exit: LastExit::Unclean,
            safe_start: Some(canvas_proto::api::SafeStart {
                active: true,
                reason: Some(canvas_proto::api::SafeStartReason::CrashLoop),
                unclean_streak: 3,
                since: Some("2026-11-27T10:00:00Z".into()),
                fans_on_bios: None,
                lights: None,
            }),
        })
    }
}

#[tokio::test]
async fn l4_the_pipe_call_starts_only_after_the_reply_is_written() {
    let (mut app, _engine, _home) = build();
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
    let seen = Arc::new(Mutex::new(None));
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    Arc::get_mut(&mut app).unwrap().pipe = Arc::new(HeldPipe {
        seen: seen.clone(),
        entered: entered_tx,
        release: Arc::new(tokio::sync::Mutex::new(None)),
    });
    let body = serde_json::to_string(&SafeStartPut { mode: SafeStartMode::Normal }).unwrap();
    let request = Request::builder()
        .method("PUT")
        .uri(canvas_proto::api::routes::SAFE_START)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let response = api::router(app.clone()).oneshot(request).await.unwrap();
    // The reply exists but nothing has written it yet.
    // A task that did not wait for the body would run within these turns.
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
    assert!(seen.lock().unwrap().is_none(), "the service was told before the reply was written");
    axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
    tokio::time::timeout(WAIT, entered_rx.recv()).await.expect("the pipe message was never sent").unwrap();
    assert!(matches!(*seen.lock().unwrap(), Some(PipeRequest::LeaveSafeStart)));
}

#[tokio::test]
async fn m4_a_breathing_failure_shows_a_steady_colour_that_status_names() {
    let (app, engine, _home) = build();
    saved_solid(&app).await;
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
    *app.safe_builtin.lock().unwrap() = "no-such-effect".into();
    canvas_daemon::scenes::apply_for_devices(&app).await;
    assert_eq!(playing(&engine), vec![Some("solid".to_string())]);
    assert_eq!(
        engine.scene.lock().unwrap()[0].2.get("color"),
        Some(&canvas_proto::props::PropValue::Color(canvas_proto::Rgb8::new(40, 90, 160)))
    );
    let status = app.status().await.unwrap();
    // The scene keeps its short name (the rail and strip never truncate it); the colour is
    // named in the safe-start record.
    assert_eq!(status.active_scene.unwrap().name, "Safe start");
    assert_eq!(status.safe_start.unwrap().lights.as_deref(), Some("One steady colour, blue"));
    assert_eq!(status.active_effects, vec!["solid".to_string()]);
}

// ---- #577 fix pass 2 ----

/// A service whose first answer fails, or hangs past the start limit, and then says Safe.
struct LatePipe {
    hang: bool,
    calls: Arc<Mutex<u32>>,
}

#[async_trait::async_trait]
impl PipeLink for LatePipe {
    async fn exchange(&self, _: &PipeRequest) -> Result<PipeResponse, PipeError> {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        if call == 1 {
            if self.hang {
                tokio::time::sleep(pipe_client::FIRST_HELLO_LIMIT + Duration::from_millis(500)).await;
            }
            return Err(PipeError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "no fake service yet")));
        }
        RecordPipe.exchange(&PipeRequest::Heartbeat).await
    }
}

async fn send(
    app: &Arc<App>,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let response = tokio::time::timeout(WAIT, api::router(app.clone()).oneshot(request))
        .await
        .expect("the request hung")
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

#[tokio::test]
async fn r4_a_safe_answer_after_a_missed_first_hello_turns_the_run_safe() {
    for hang in [false, true] {
        let (mut app, engine, _home) = build();
        Arc::get_mut(&mut app).unwrap().pipe =
            Arc::new(LatePipe { hang, calls: Arc::new(Mutex::new(0)) });
        saved_solid(&app).await;
        assert_eq!(pipe_client::read_start_decision(&app).await, SafeStartMode::Normal, "hang={hang}");
        canvas_daemon::scenes::restore_active(&app).await;
        assert_eq!(playing(&engine), vec![Some("rainbow_wave".to_string())], "hang={hang}");
        let saved = app.store.read().await.scenes.clone();

        assert_eq!(pipe_client::read_start_decision(&app).await, SafeStartMode::Safe, "hang={hang}");
        assert_eq!(playing(&engine), vec![Some("breathing".to_string())], "hang={hang}");
        let status = app.status().await.unwrap();
        assert!(status.safe_start.expect("the banner's record").active);
        assert_eq!(status.active_scene.unwrap().name, "Safe start");
        assert_eq!(app.last_exit(), LastExit::Unclean);
        assert_eq!(app.store.read().await.scenes, saved, "the saved scene was changed");
    }
}

#[tokio::test]
async fn r4_a_late_safe_answer_ends_the_draft_on_the_lights() {
    let (mut app, engine, _home) = build();
    Arc::get_mut(&mut app).unwrap().pipe = Arc::new(LatePipe { hang: false, calls: Arc::new(Mutex::new(0)) });
    saved_solid(&app).await;
    pipe_client::read_start_decision(&app).await;
    canvas_daemon::scenes::restore_active(&app).await;
    let (status, body) = send(&app, "PUT", routes::EFFECT_DRAFT, Some(pattern_draft())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(canvas_daemon::draft::status(&app).active);

    pipe_client::read_start_decision(&app).await;
    assert!(!canvas_daemon::draft::status(&app).active, "the draft stayed live in safe start");
    assert_eq!(playing(&engine), vec![Some("breathing".to_string())]);
}

#[tokio::test]
async fn r5_safe_start_lights_the_lights_whatever_was_saved() {
    for (saved, shown) in [(0u8, 20u8), (5, 20), (80, 80)] {
        let (mut app, engine, _home) = build();
        Arc::get_mut(&mut app).unwrap().pipe = Arc::new(RecordPipe);
        {
            let mut store = app.store.write().await;
            store.config.settings.power = Power::Paused;
            store.config.settings.brightness = saved;
            store.config.settings.warmth = 3000;
        }
        // The engine starts from the saved settings, before the mode is known.
        engine.set_power(Power::Paused);
        engine.set_brightness(saved);
        engine.set_warmth(3000);
        let before = app.store.read().await.config.settings.clone();

        pipe_client::read_start_decision(&app).await;
        let lit = |when: &str| {
            assert_eq!(*engine.power.lock().unwrap(), Power::Running, "{when}, saved {saved}");
            assert_eq!(engine.brightness.load(Ordering::SeqCst), u32::from(shown), "{when}, saved {saved}");
            assert_eq!(engine.warmth.load(Ordering::SeqCst), 3000, "{when}");
        };
        lit("at start");
        let status = app.status().await.unwrap();
        assert_eq!((status.power, status.brightness), (Power::Running, shown), "status");
        let (_, output) = send(&app, "GET", routes::OUTPUT, None).await;
        assert_eq!(output["power"], "running");
        assert_eq!(output["brightness"], shown);

        // What puts the saved output back must not darken the lights.
        canvas_daemon::identify::restore(&app).await;
        assert!(engine.solid_writes.lock().unwrap().is_empty(), "black was painted");
        let mut watcher = canvas_daemon::power::PowerWatcher::new();
        for event in [canvas_daemon::power::PowerEvent::Suspend, canvas_daemon::power::PowerEvent::Resume] {
            canvas_daemon::power::handle(&app, &mut watcher, event).await;
        }
        lit("after sleep");
        // A settings save that changes something else keeps the lights on.
        let mut settings = before.clone();
        settings.fps = 24;
        let (code, body) = send(&app, "PUT", routes::SETTINGS, Some(serde_json::to_value(&settings).unwrap())).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        lit("after a settings save");
        let mut stored = app.store.read().await.config.settings.clone();
        stored.fps = before.fps;
        assert_eq!(stored, before, "safe start wrote to the store");

        // The owner's own change wins, and is saved as in any run.
        let (code, body) = send(&app, "PATCH", routes::OUTPUT, Some(serde_json::json!({ "brightness": 7 }))).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(body["brightness"], 7);
        assert_eq!(engine.brightness.load(Ordering::SeqCst), 7);
        assert_eq!(app.status().await.unwrap().brightness, 7);
        let (code, body) = send(&app, "PATCH", routes::OUTPUT, Some(serde_json::json!({ "power": "paused" }))).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(body["power"], "paused");
        assert_eq!(*engine.power.lock().unwrap(), Power::Paused);
    }
}

fn pattern_draft() -> serde_json::Value {
    serde_json::json!({
        "pattern": {
            "id": "pattern:Probe",
            "built_on": { "led_count": 12, "arrangement": "ring" },
            "frames": [{ "colors": vec!["#c80000"; 12], "hold": 0 }],
            "motion": serde_json::to_value(canvas_proto::pattern::Motion::none()).unwrap(),
        },
        "props": {},
        "epoch": "0123456789abcdef",
        "seq": 1,
    })
}

const DRAFT_HTML: &str = r##"<head><title>Flat</title></head>
<body><canvas id="c" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("c");
  var ctx = c.getContext("2d");
  ctx.fillStyle = "#3366cc";
  function draw() { ctx.fillRect(0, 0, 320, 200); window.requestAnimationFrame(draw); }
  window.requestAnimationFrame(draw);
</script>"##;

#[tokio::test]
async fn r7_safe_start_refuses_every_draft_with_a_clear_error() {
    let (app, engine, _home) = build();
    saved_solid(&app).await;
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
    canvas_daemon::scenes::apply_for_devices(&app).await;
    let stack = serde_json::json!({
        "stack": { "id": "stack:Probe", "layers": [{ "id": "l1", "name": "Base", "effect": "solid" }] },
        "props": {},
        "epoch": "0123456789abcdef",
        "seq": 1,
    });
    let html = serde_json::json!({ "html": DRAFT_HTML, "props": {} });
    for (kind, body) in [("html", html), ("stack", stack), ("pattern", pattern_draft())] {
        let (status, answer) = send(&app, "PUT", routes::EFFECT_DRAFT, Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{kind}: {answer}");
        assert_eq!(answer["message"], "Not available in safe start", "{kind}");
        assert!(!canvas_daemon::draft::status(&app).active, "{kind} left a live draft");
    }
    assert_eq!(playing(&engine), vec![Some("breathing".to_string())]);
}

#[tokio::test]
async fn r7_safe_start_refuses_the_greeting() {
    let (app, engine, _home) = build();
    saved_solid(&app).await;
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
    canvas_daemon::scenes::apply_for_devices(&app).await;
    let uri = routes::EFFECT_ACTIVATE.replace("{id}", "onboard-hello");
    let (status, answer) = send(&app, "POST", &uri, Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert_eq!(answer["message"], "Not available in safe start");
    assert_eq!(playing(&engine), vec![Some("breathing".to_string())]);
}

#[tokio::test]
async fn r7_safe_start_applies_nothing_of_the_owners() {
    let (app, engine, _home) = build();
    saved_solid(&app).await;
    app.latch_start(SafeStartMode::Safe, LastExit::Unclean);
    canvas_daemon::scenes::apply_for_devices(&app).await;
    let safe = vec![("safe-start".to_string(), Some("breathing".to_string()))];
    let on_lights = || {
        engine.scene.lock().unwrap().iter().map(|g| (g.0.clone(), g.1.clone())).collect::<Vec<_>>()
    };
    assert_eq!(on_lights(), safe);

    let build_one = canvas_daemon::takeover_builder(&app, "solid");
    let groups = canvas_daemon::scenes::apply_takeover(&app, "solid", &Props::new(), None, build_one)
        .await
        .unwrap();
    assert!(groups.is_empty());
    assert_eq!(on_lights(), safe, "takeover");

    let pattern = serde_json::from_value(pattern_draft()["pattern"].clone()).unwrap();
    assert_eq!(canvas_daemon::scenes::apply_pattern_takeover(&app, &pattern).await.unwrap(), 0);
    assert_eq!(on_lights(), safe, "pattern takeover");

    // From here an apply of any kind shows: the record is emptied, and stays empty.
    engine.scene.lock().unwrap().push(("marker".into(), None, Props::new()));
    let marked = on_lights();
    // A saved group that shares the safe group's id: nothing but the guard keeps it off.
    {
        let mut store = app.store.write().await;
        let group = &mut store.scenes.scenes[0].groups[0];
        group.id = "safe-start".into();
        group.plays = canvas_proto::scene::Plays::Effect { effect: "pattern:Probe".into(), props: Props::new() };
    }

    let old = app.store.read().await.scenes.scenes[0].clone();
    let mut new = old.clone();
    new.groups[0].plays = canvas_proto::scene::Plays::Effect { effect: "solid".into(), props: Props::new() };
    {
        let lock = app.scene.lock().await;
        canvas_daemon::scenes::apply_group_change(&app, &lock, &old, &new).await;
        let mut wider = new.clone();
        let mut extra = new.groups[0].clone();
        extra.id = "second".into();
        extra.members.clear();
        wider.groups.push(extra);
        canvas_daemon::scenes::apply_group_change(&app, &lock, &new, &wider).await;
    }
    assert_eq!(on_lights(), marked, "group change");

    canvas_daemon::scenes::pattern_changed(&app, "pattern:Probe").await;
    assert_eq!(on_lights(), marked, "pattern changed");

    // With a device here, a first run that went ahead would save lights and its own flag.
    app.devices.write().await.push(common::device("x", canvas_proto::device::DeviceKind::Motherboard, 8));
    app.store.write().await.config.first_run_done = false;
    let before = app.store.read().await.config.clone();
    canvas_daemon::first_run::maybe_run(&app).await;
    assert_eq!(on_lights(), marked, "first run");
    assert_eq!(app.store.read().await.config, before, "the first run wrote the config");
}
