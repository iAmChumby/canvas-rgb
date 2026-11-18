//! #241: `GET /api/v1/settings` must keep answering while the fakes' timers run.
//!
//! A fakes-mode daemon is a two-worker tokio runtime (`crates/canvas-daemon/src/main.rs`):
//! the API and every in-process fake timer share the same two workers. The capture polled
//! `/settings` between shots, so a task that blocks a worker (or a lock a route needs that a
//! timer holds across an await) takes the API down while the port keeps listening: the editor
//! shows "Daemon connecting" and the poll times out.
//!
//! This test builds the daemon's state against the fakes (the fake pipe, the disabled capture,
//! the in-memory display's app state) and starts the timers fakes mode starts: the draft and
//! audio lease sweepers, the engine watchers (on their own threads, as in production) and the
//! user-effect rescan. It then hammers `/settings` on the daemon's two workers. Every request
//! is reported over a std channel, and the test waits with a generous hang guard on its own
//! thread, so a stuck worker cannot stop the test from noticing (#241's own two-worker trap).
//!
//! The verdict is the answer the client gets: every request must come back 200 with a Settings
//! body. A hang is not a slow number, it is no answer at all.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::fakes::FakePipeLink;
use canvas_daemon::state::App;
use canvas_proto::config::Settings;
use common::{FakeAdapter, FakeEngine};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

/// How many requests to fire at the API. Bounded, so a pass finishes quickly and a hang shows
/// up as one missing answer rather than an unbounded wait.
const REQUESTS: usize = 400;

/// How long the test waits for one answer before calling it a hang. A route on a two-worker
/// runtime answers in milliseconds; ten seconds is far beyond a slow runner and only a stuck
/// worker can reach it.
const HANG_GUARD: Duration = Duration::from_secs(10);

/// The token the requests carry.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A temporary home, removed when the test ends.
struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-fake-settings-{}-{:?}",
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

/// The daemon's fakes-mode state, with the capture disabled and the fake service pipe in place.
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
    app.fans = tokio::sync::RwLock::new(Some(canvas_daemon::fakes::fake_fans_status()));
    let app = Arc::new(app);
    let router = api::router(app.clone());
    (app, router)
}

/// An authorised settings request.
fn request() -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/api/v1/settings")
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

/// The fakes-mode daemon must answer every settings request while its timers run.
///
/// This is the user-visible effect: the capture's poll gets a Settings answer, not a timeout
/// and not the editor's "Daemon connecting" screen. The engine watchers, the two sweepers and
/// the effect rescan run on the same runtime as these requests, as they do in the daemon.
#[test]
fn the_fakes_daemon_keeps_answering_settings_while_its_timers_run() {
    let home = TempHome::new();
    let (app, router) = build(&home);

    // The daemon's runtime shape: two workers shared by the API and the fake timers.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("the runtime is built");
    let handle = runtime.handle().clone();
    app.shutting_down.store(0, Ordering::SeqCst);

    // The timers fakes mode starts. The engine watchers use their own threads in production.
    // The sweepers call `tokio::spawn` internally, so they need the runtime in scope.
    let _enter = handle.enter();
    let watch = canvas_daemon::state::watch_engine(app.clone());
    canvas_daemon::draft::spawn_sweeper(app.clone());
    canvas_daemon::api::audio::spawn_sweeper(app.clone());
    let rescan = {
        let app = app.clone();
        handle.spawn(async move {
            let mut ticker =
                tokio::time::interval(canvas_daemon::user_effects::RESCAN_INTERVAL);
            loop {
                ticker.tick().await;
                let user = app.user_effects.clone();
                let dir = app.effects_dir.clone();
                let _ = tokio::task::spawn_blocking(move || user.rescan(&dir)).await;
            }
        })
    };

    let (tx, rx) = std::sync::mpsc::channel();
    let mut tasks = Vec::with_capacity(REQUESTS);
    for _ in 0..REQUESTS {
        let router = router.clone();
        let tx = tx.clone();
        tasks.push(handle.spawn(async move {
            let response = router.oneshot(request()).await.expect("the route answers");
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("the body is read")
                .to_vec();
            let _ = tx.send((status, body));
        }));
    }

    // The wait is on this thread, not on a worker: a task that never yields cannot keep the
    // test from seeing that its answer never came.
    let verdict = (|| -> Result<(), String> {
        for i in 0..REQUESTS {
            let (status, body) = rx.recv_timeout(HANG_GUARD).map_err(|e| {
                format!("settings request {i} never answered within {HANG_GUARD:?}: {e}")
            })?;
            if status != StatusCode::OK {
                return Err(format!(
                    "settings request {i} answered {status}: {}",
                    String::from_utf8_lossy(&body)
                ));
            }
            serde_json::from_slice::<Settings>(&body).map_err(|e| {
                format!("settings request {i} answered a body that is not Settings: {e}")
            })?;
        }
        Ok(())
    })();

    drop(_enter);
    app.shutting_down.store(1, Ordering::SeqCst);
    rescan.abort();
    for task in tasks {
        task.abort();
    }
    drop(watch);
    runtime.shutdown_timeout(Duration::from_millis(200));
    verdict.unwrap_or_else(|error| panic!("{error}"));
}
