//! #241: the API must not wait on the render engine's lock.
//!
//! The engine's handle is a `std::sync::Mutex` the render thread takes as well, so a
//! route that asks the engine on a runtime worker waits for the render thread there.
//! The daemon's runtime has **two** workers (`crates/canvas-daemon/src/main.rs`), so two
//! such waits take the whole API down while the port keeps listening: the editor says
//! "the daemon did not answer" and the capture's poll of `/api/v1/settings` times out.
//!
//! These tests drive the API with an engine that answers far too late, so a route that
//! waits for it is a route that never answers inside the limit. Every request runs on a
//! thread of its own, so the wait below is a plain channel wait: a route that blocks
//! cannot stop the test from noticing, and the test process ends either way.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::device::DeviceId;
use canvas_proto::props::Props;
use canvas_proto::traits::{
    Effect, Engine, FrameSink, GroupState, InputSource, LightPreviewSink, PreviewSink, RenderStats,
};
use canvas_proto::Rgb8;
use common::FakeAdapter;
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// How long a route may take to answer before the test says it never answered.
const ANSWER_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);

/// How long the engine takes to answer. It is longer than [`ANSWER_LIMIT`], so a route
/// that waits for it never answers inside the limit.
const ENGINE_WAIT: std::time::Duration = std::time::Duration::from_secs(12);

/// An engine that answers too late: its numbers and its group states come back only
/// after [`ENGINE_WAIT`].
///
/// It stands in for a render thread that does not hand the engine's lock over: a long
/// frame, a stuck effect, a thread that is wedged. The daemon must keep serving the API
/// while that is true.
struct StuckEngine {
    /// How long the answer takes.
    wait: std::time::Duration,
    /// How many times the engine was asked for something. A test waits on this count, so
    /// it knows the route is waiting inside the engine and not somewhere else.
    asked: std::sync::atomic::AtomicUsize,
}

impl StuckEngine {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            wait: ENGINE_WAIT,
            asked: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Waits for an answer that comes far too late.
    fn wait_too_long(&self) {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(self.wait);
    }

    /// How many times the engine was asked for something.
    fn asked(&self) -> usize {
        self.asked.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Engine for StuckEngine {
    fn set_fps(&self, _fps: u32) {}
    fn set_power(&self, _power: canvas_proto::api::Power) {}
    fn set_brightness(&self, _brightness: u8) {}
    fn set_warmth(&self, _kelvin: u16) {}
    fn set_white_balance(&self, _balances: HashMap<DeviceId, canvas_proto::color::WhiteBalance>) {}
    fn add_sink(&self, _device: DeviceId, _sink: Arc<dyn FrameSink>) {}
    fn remove_sink(&self, _device: &DeviceId) {}
    fn set_preview(&self, _sink: Option<Arc<dyn PreviewSink>>) {}
    fn set_input_source(&self, _source: Option<Arc<dyn InputSource>>) {}
    fn write_solid(&self, _color: Rgb8) {}

    /// The numbers a render thread that is slow to answer gives.
    fn stats(&self) -> RenderStats {
        self.wait_too_long();
        RenderStats::default()
    }

    fn shutdown(&self) {}

    fn set_scene(
        &self,
        _lights: Vec<canvas_proto::scene::Light>,
        _devices: Vec<canvas_proto::device::DeviceInfo>,
        _groups: Vec<canvas_proto::EngineGroup>,
        _fade: bool,
    ) {
    }

    fn set_group_effect(&self, _group: &str, _effect: Option<Box<dyn Effect>>, _props: &Props) {}
    fn set_group_props(&self, _group: &str, _props: &Props) {}
    fn set_group_brightness(&self, _group: &str, _brightness: u8) {}
    fn set_light_preview(&self, _sink: Option<Arc<dyn LightPreviewSink>>) {}
    fn set_group_preview(&self, _group: Option<String>, _sink: Option<Arc<dyn PreviewSink>>) {}

    /// The group states a render thread that is slow to answer gives.
    fn group_states(&self) -> Vec<GroupState> {
        self.wait_too_long();
        Vec::new()
    }
}

/// A temporary directory that is removed when it is dropped.
struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-engine-241-{}-{:?}",
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

/// The router and the state, with an engine that answers too late.
fn build() -> (axum::Router, Arc<App>, TempHome, Arc<StuckEngine>) {
    let home = TempHome::new();
    let store = canvas_daemon::storage::Store::at(&home.path);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let engine = StuckEngine::new();
    let app = Arc::new(App::new(
        store,
        Arc::clone(&engine) as Arc<dyn Engine>,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    (api::router(app.clone()), app, home, engine)
}

/// A request with the token.
fn request(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

/// Asks the route on a thread of its own and waits for its answer. `None` when it never
/// came.
///
/// The route runs on its own thread with its own runtime, so the wait is a plain channel
/// wait: a route that blocks the render engine cannot stop the test from noticing.
fn ask(router: axum::Router, path: &str) -> Option<StatusCode> {
    let (tx, rx) = std::sync::mpsc::channel();
    let path = path.to_string();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2)
            .enable_all()
            .build()
            .expect("the runtime is built");
        let answer = runtime.block_on(async move {
            router.oneshot(request(&path)).await.map(|response| response.status())
        });
        let _ = tx.send(answer.ok());
    });
    rx.recv_timeout(ANSWER_LIMIT).ok().flatten()
}

/// Waits until the engine has been asked `count` times, so a test knows the route is
/// waiting inside the engine and not somewhere else.
fn wait_for_asks(engine: &Arc<StuckEngine>, count: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while engine.asked() < count {
        if std::time::Instant::now() >= deadline {
            panic!("the engine was not asked {count} times");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Gives the writer the moment it needs to reach the store's lock.
fn let_the_writer_queue() {
    std::thread::sleep(std::time::Duration::from_millis(200));
}

/// The status route must answer even when the engine does not.
///
/// Before the fix the route called the engine on a runtime worker, so a render thread
/// that never hands the lock over took that worker for good. The editor polls the status
/// every second, so the second poll took the second worker and the whole API stopped
/// answering while the port kept listening: the capture's poll of `/api/v1/settings`
/// timed out.
#[test]
fn the_status_route_answers_while_the_engine_does_not() {
    let (router, _app, _home, _engine) = build();
    let status = ask(router, "/api/v1/status");
    assert_eq!(
        status,
        Some(StatusCode::OK),
        "the status route never answered: it waited for the render engine on a runtime worker"
    );
}

/// The scenes route must answer too, and it must not hold the store's lock while it
/// waits: a reader that holds it across that wait blocks every writer, and through them
/// every other reader (`/api/v1/settings` among them).
#[test]
fn the_scenes_route_answers_while_the_engine_does_not() {
    let (router, _app, _home, _engine) = build();
    let status = ask(router, "/api/v1/scenes");
    assert_eq!(
        status,
        Some(StatusCode::OK),
        "the scenes route never answered: it waited for the render engine"
    );
}

/// The settings route must answer while a scenes request is stuck.
///
/// The scenes route used to hold the store's lock while it waited for the engine. The
/// store's lock is a fair one: a writer that queues behind that reader stops every
/// other reader too, and `/api/v1/settings` is one of them. This is the route the
/// capture polls, and the one the report says timed out.
#[test]
fn the_settings_route_answers_while_a_scenes_request_is_stuck() {
    let (router, _app, _home, engine) = build();
    // A scenes request that waits inside the engine.
    let stuck = {
        let router = router.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2)
                .enable_all()
                .build()
                .expect("the runtime is built");
            let _ = runtime.block_on(async move {
                router.oneshot(request("/api/v1/scenes")).await.map(|response| response.status())
            });
        })
    };
    wait_for_asks(&engine, 1);
    // A writer queues behind the reader that holds the store's lock. It is a route that
    // takes the store's lock and never asks the engine, so it queues for the lock and
    // for nothing else.
    let writer = {
        let router = router.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2)
                .enable_all()
                .build()
                .expect("the runtime is built");
            let _ = runtime.block_on(async move {
                router
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/v1/first-run/dismiss-zone-panel")
                            .header("host", "127.0.0.1:9430")
                            .header("authorization", format!("Bearer {TOKEN}"))
                            .body(Body::empty())
                            .expect("the request builds"),
                    )
                    .await
                    .map(|response| response.status())
            });
        })
    };
    let_the_writer_queue();
    // The writer is queued behind the reader that holds the store's lock.
    //
    // The settings route is the one the capture polls. It must still answer.
    let status = ask(router, "/api/v1/settings");
    assert_eq!(
        status,
        Some(StatusCode::OK),
        "the settings route never answered: a stuck scenes request held the store's lock"
    );
    let _ = stuck.join();
    let _ = writer.join();
}
