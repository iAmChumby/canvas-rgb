//! #241: a slow config write must not hold the store lock or a runtime worker.
//!
//! The daemon's two-worker runtime serves the API and its timers. A handler that used to take
//! `app.store.write()`, change the config and call the synchronous `save_config()` while holding
//! the guard parked a worker on the disk and made every reader wait for the write, including
//! `GET /api/v1/settings`. The editor then showed "Daemon connecting" while the port still
//! listened.
//!
//! This test injects a slow writer with the writer gate in `storage::write_atomic`, holds a PUT
//! there, and asks for settings while it is parked. The user-visible verdict is the answer the
//! GET gets: it must come back 200 while the disk is busy, not time out. The wait is on a test
//! thread with a deadline, so a hung worker cannot hide the hang from the test.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::fakes::FakePipeLink;
use canvas_daemon::state::App;
use canvas_daemon::storage::write_gate;
use canvas_proto::config::Settings;
use common::{FakeAdapter, FakeEngine};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// How long the GET may take while the write is parked. A settings read is a lock and a clone;
/// three seconds is far beyond a slow runner and only a held store lock can reach it.
const GET_DEADLINE: Duration = Duration::from_secs(3);

/// A temporary test home, removed when the test ends.
struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-blocking-saves-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp home is created");
        Self { path }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Releases the global writer gate even if an assertion panics, so this test cannot leave other
/// tests in the same binary parked.
struct GateRelease;

impl Drop for GateRelease {
    fn drop(&mut self) {
        write_gate().release();
    }
}

fn build(home: &TempHome) -> (Arc<App>, axum::Router) {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        canvas_daemon::storage::Store::at(&home.path),
        Arc::new(FakeEngine::default()),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    app.fakes = true;
    app.pipe = Arc::new(FakePipeLink::default());
    app.inputs = Arc::new(canvas_daemon::capture::DaemonInputs::disabled());
    let app = Arc::new(app);
    let router = api::router(app.clone());
    (app, router)
}

fn request(method: &str, uri: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(body)
        .expect("the request builds")
}

/// `GET /settings` answers while a concurrent config save is stalled on the disk.
#[test]
fn settings_answers_while_the_config_write_is_stalled() {
    let home = TempHome::new();
    let (app, router) = build(&home);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("the runtime is built");

    let gate = write_gate();
    gate.hold();
    let _release = GateRelease;

    let settings = Settings { fps: 45, ..Default::default() };
    let body = serde_json::to_vec(&settings).expect("the settings serialise");

    let put_router = router.clone();
    let put = runtime.spawn(async move {
        put_router
            .oneshot(request("PUT", "/api/v1/settings", Body::from(body)))
            .await
            .expect("the PUT route answers")
            .status()
    });

    assert!(
        gate.wait_until_entered(Duration::from_secs(10)),
        "the PUT never reached the writer gate"
    );

    // The GET runs on a test thread with its own deadline. A parked worker cannot stop the test
    // from seeing that no answer came.
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = runtime.handle().clone();
    let get_router = router.clone();
    let getter = std::thread::spawn(move || {
        let status = handle.block_on(async {
            get_router
                .oneshot(request("GET", "/api/v1/settings", Body::empty()))
                .await
                .expect("the GET route answers")
                .status()
        });
        let _ = tx.send(status);
    });

    let verdict = rx.recv_timeout(GET_DEADLINE);
    gate.release();

    match verdict {
        Ok(StatusCode::OK) => {}
        Ok(other) => panic!("settings answered {other} while the write was parked"),
        Err(e) => panic!("settings did not answer while the config write was stalled: {e}"),
    }

    let put_status = runtime.block_on(put).expect("the PUT task finished");
    assert_eq!(put_status, StatusCode::OK, "the stalled save still succeeded");
    getter.join().expect("the getter thread finished");

    // The change the PUT made is the one that landed, even though readers were not blocked.
    let stored = runtime.block_on(async { app.store.read().await.config.settings.clone() });
    assert_eq!(stored.fps, 45);
}
