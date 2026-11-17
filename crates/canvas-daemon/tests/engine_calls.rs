//! #241: the API must not wait on the render engine's lock.
//!
//! The engine's handle is a `std::sync::Mutex` the render thread takes as well, so a
//! route that asks the engine on a runtime worker waits for the render thread there.
//! The daemon's runtime has **two** workers (`crates/canvas-daemon/src/main.rs`), so two
//! such waits take the whole API down while the port keeps listening: the editor says
//! "the daemon did not answer" and the capture's poll of `/api/v1/settings` times out.
//!
//! These tests drive the API with an engine the test drives: it answers with whatever
//! the test put in it, after a delay the test sets. A delay longer than the route's
//! bound stands in for a render thread that does not hand the lock over. Every request
//! runs on a thread of its own, so the wait below is a plain channel wait: a route that
//! blocks cannot stop the test from noticing, and the test process ends either way.
//!
//! Four things are proved here, and each of them has a test that fails without it:
//!
//! 1. The API keeps answering while the engine does not, and it says that the engine is
//!    not answering. Zeros and an empty group list look like a measurement and like a
//!    group that is fine; they are not (#241).
//! 2. A daemon whose engine answered once answers with what it knew: the numbers are
//!    true as of the time the engine gave them, and a group that failed is still failed.
//! 3. The daemon's own copy of the engine's numbers is not the only source: once it is
//!    older than a bound, a route asks the engine itself, and the numbers move again.
//!    The watcher that keeps the copy fresh survives a quit that was cancelled.
//! 4. The store's lock is given back before the engine is asked, so a writer queued
//!    behind a stuck scenes request does not stop the other readers.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::api::{ScenesResponse, Status};
use canvas_proto::device::DeviceId;
use canvas_proto::props::Props;
use canvas_proto::scene::{Group, GroupMode, Plays, Scene, ScenesFile, SCENES_SCHEMA};
use canvas_proto::traits::{
    Effect, Engine, FrameSink, GroupState, InputSource, LightPreviewSink, PreviewSink, RenderStats,
};
use canvas_proto::Rgb8;
use common::FakeAdapter;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// How long a route may take to answer before the test says it never answered.
const ANSWER_LIMIT: Duration = Duration::from_secs(3);

/// How long the engine takes to answer when the test says it is stuck. It is longer than
/// [`ANSWER_LIMIT`], so a route that waits for it never answers inside the limit.
const ENGINE_WAIT: Duration = Duration::from_secs(12);

/// The group the test's one scene holds.
const GROUP: &str = "left";

/// An engine the test drives.
///
/// It answers with whatever the test last put in it, after the delay the test set, and
/// it counts how often it was asked: a test waits on that count, so it knows the route is
/// waiting inside the engine and not somewhere else.
struct FakeEngine {
    /// How long every answer takes.
    delay: Mutex<Duration>,
    /// How long an answer takes when a **route** asks for it. The watcher asks on its
    /// own thread and is never slow; a route that asks has to wait, and a route that
    /// answers in less than this never asked.
    slow_for_routes: Mutex<Duration>,
    /// The numbers it answers with.
    numbers: Mutex<RenderStats>,
    /// The group states it answers with.
    states: Mutex<Vec<GroupState>>,
    /// How many times the engine was asked for its numbers.
    asked_stats: AtomicUsize,
    /// How many times it was asked for its group states.
    asked_groups: AtomicUsize,
}

impl FakeEngine {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            delay: Mutex::new(Duration::from_millis(0)),
            slow_for_routes: Mutex::new(Duration::from_millis(0)),
            numbers: Mutex::new(RenderStats::default()),
            states: Mutex::new(Vec::new()),
            asked_stats: AtomicUsize::new(0),
            asked_groups: AtomicUsize::new(0),
        })
    }

    /// How long every answer takes from now on.
    fn set_delay(&self, delay: Duration) {
        *self.delay.lock().unwrap_or_else(|e| e.into_inner()) = delay;
    }

    /// How long an answer takes when a route asks for it.
    fn set_slow_for_routes(&self, slow: Duration) {
        *self.slow_for_routes.lock().unwrap_or_else(|e| e.into_inner()) = slow;
    }

    /// The numbers the engine answers with from now on.
    fn set_numbers(&self, numbers: RenderStats) {
        *self.numbers.lock().unwrap_or_else(|e| e.into_inner()) = numbers;
    }

    /// The group states the engine answers with from now on.
    fn set_states(&self, states: Vec<GroupState>) {
        *self.states.lock().unwrap_or_else(|e| e.into_inner()) = states;
    }

    /// How many times the engine was asked for its numbers.
    fn asked_stats(&self) -> usize {
        self.asked_stats.load(Ordering::SeqCst)
    }

    /// How many times the engine was asked for its group states.
    fn asked_groups(&self) -> usize {
        self.asked_groups.load(Ordering::SeqCst)
    }

    /// Waits the delay the test set. Everybody waits the delay; a **route** waits the
    /// delay the test gave it for routes on top of it.
    fn wait(&self) {
        let delay = *self.delay.lock().unwrap_or_else(|e| e.into_inner());
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        let from_watcher = std::thread::current()
            .name()
            .is_some_and(|name| name.starts_with("canvas-rgb-engine-"));
        if from_watcher {
            return;
        }
        let slow = *self.slow_for_routes.lock().unwrap_or_else(|e| e.into_inner());
        if !slow.is_zero() {
            std::thread::sleep(slow);
        }
    }
}

impl Engine for FakeEngine {
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
        self.asked_stats.fetch_add(1, Ordering::SeqCst);
        self.wait();
        self.numbers.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
        self.asked_groups.fetch_add(1, Ordering::SeqCst);
        self.wait();
        self.states.lock().unwrap_or_else(|e| e.into_inner()).clone()
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

/// The router and the state, with an engine the test drives.
struct Rig {
    router: axum::Router,
    app: Arc<App>,
    engine: Arc<FakeEngine>,
    /// Dropped last, so the home folder outlives the app that reads it.
    _home: TempHome,
}

/// The rig: one scene with one group, so the scenes route has a group to report on.
fn build() -> Rig {
    let home = TempHome::new();
    let mut store = canvas_daemon::storage::Store::at(&home.path);
    store.scenes = ScenesFile {
        schema_version: SCENES_SCHEMA,
        active: "main".into(),
        scenes: vec![Scene {
            id: "main".into(),
            name: "Main".into(),
            rev: 1,
            groups: vec![Group {
                id: GROUP.into(),
                name: "Left".into(),
                members: Vec::new(),
                plays: Plays::Off,
                mode: GroupMode::EachItsOwn,
                brightness: 100,
            }],
        }],
    };
    let engine = FakeEngine::new();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        Arc::clone(&engine) as Arc<dyn Engine>,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    Rig { router: api::router(app.clone()), app, engine, _home: home }
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

/// A request with the token and a method of its own.
fn request_with(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

/// What a route answered.
struct Answer {
    status: StatusCode,
    body: Vec<u8>,
    /// How long the route took.
    took: Duration,
}

impl Answer {
    /// The body as JSON.
    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!("the body is not JSON ({error}): {}", String::from_utf8_lossy(&self.body))
        })
    }

    /// The body as text.
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

/// Asks the route on a thread of its own and waits for its answer. `None` when it never
/// came.
///
/// The route runs on its own thread with its own runtime, so the wait is a plain channel
/// wait: a route that blocks the render engine cannot stop the test from noticing.
fn ask(router: axum::Router, path: &str) -> Option<Answer> {
    ask_with(router, request(path))
}

/// [`ask`], with a request of its own.
fn ask_with(router: axum::Router, request: Request<Body>) -> Option<Answer> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("the runtime is built");
        let started = Instant::now();
        let answer = runtime.block_on(async move {
            let response = router.oneshot(request).await.ok()?;
            let status = response.status();
            let body =
                axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.ok()?.to_vec();
            Some(Answer { status, body, took: started.elapsed() })
        });
        let _ = tx.send(answer);
    });
    rx.recv_timeout(ANSWER_LIMIT).ok().flatten()
}

/// Waits until the engine has been asked for its numbers `count` times, so a test knows
/// the daemon's copy of them is filled.
fn wait_for_stats_asks(engine: &Arc<FakeEngine>, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while engine.asked_stats() < count {
        if Instant::now() >= deadline {
            panic!("the engine was not asked for its numbers {count} times");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until the engine has been asked for its group states `count` times, so a test
/// knows the daemon's copy of them is filled.
fn wait_for_group_asks(engine: &Arc<FakeEngine>, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while engine.asked_groups() < count {
        if Instant::now() >= deadline {
            panic!("the engine was not asked for its group states {count} times");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until the watcher has stopped asking the engine.
///
/// The watcher asks every [`canvas_daemon::state::ENGINE_POLL_INTERVAL`], so the count
/// has to hold still for longer than that before the watcher is gone.
fn wait_for_the_watcher_to_stop(engine: &Arc<FakeEngine>) {
    let mut last = engine.asked_stats() + engine.asked_groups();
    for _ in 0..20 {
        std::thread::sleep(canvas_daemon::state::ENGINE_POLL_INTERVAL + Duration::from_millis(150));
        let now = engine.asked_stats() + engine.asked_groups();
        if now == last {
            return;
        }
        last = now;
    }
    panic!("the watcher is still asking the engine");
}

/// The status route must answer even when the engine does not, and it must say that the
/// engine is not answering.
///
/// Before the fix the route called the engine on a runtime worker, so a render thread
/// that never hands the lock over took that worker for good. The editor polls the status
/// every second, so the second poll took the second worker and the whole API stopped
/// answering while the port kept listening: the capture's poll of `/api/v1/settings`
/// timed out.
///
/// The answer is a 503 and not a status with zeros in it: `0` fps, `0` frames and
/// `parked: false` look like a measurement, and the editor shows them as one.
#[test]
fn the_status_route_says_the_engine_is_not_answering() {
    let rig = build();
    rig.engine.set_delay(ENGINE_WAIT);
    let answer = ask(rig.router, "/api/v1/status")
        .expect("the status route never answered: it waited for the render engine");
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE, "the body: {}", answer.text());
    assert!(
        answer.text().contains("the render engine is not answering"),
        "the body says why: {}",
        answer.text()
    );
}

/// The scenes route must answer too, and it must not say that no group failed when it
/// does not know: an empty group list is what a healthy daemon answers.
#[test]
fn the_scenes_route_says_the_engine_is_not_answering() {
    let rig = build();
    rig.engine.set_delay(ENGINE_WAIT);
    let answer = ask(rig.router, "/api/v1/scenes")
        .expect("the scenes route never answered: it waited for the render engine");
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE, "the body: {}", answer.text());
    assert!(
        answer.text().contains("the render engine is not answering"),
        "the body says why: {}",
        answer.text()
    );
}

/// A daemon whose engine answered once and then stopped answers with the numbers it
/// knew. They are true as of the time the engine gave them, and zeros are a measurement
/// that never happened.
#[test]
fn the_status_route_answers_with_the_numbers_it_knew() {
    let rig = build();
    rig.engine.set_numbers(RenderStats {
        fps_measured: 42.0,
        frames_rendered: 1234,
        ..RenderStats::default()
    });
    // The daemon's watcher fills the copy of the numbers.
    let watch = canvas_daemon::state::watch_engine(Arc::clone(&rig.app));
    wait_for_stats_asks(&rig.engine, 1);
    drop(watch);
    wait_for_the_watcher_to_stop(&rig.engine);
    // The engine stops answering, and the daemon's copy goes stale.
    rig.engine.set_delay(ENGINE_WAIT);
    set_max_age(&rig.app, Duration::from_millis(50));
    std::thread::sleep(Duration::from_millis(150));
    let answer = ask(rig.router, "/api/v1/status").expect("the status route never answered");
    assert_eq!(answer.status, StatusCode::OK, "the body: {}", answer.text());
    let status: Status = answer.json();
    assert_eq!(status.fps_measured, 42.0, "the numbers are the ones the engine gave");
    assert_eq!(status.frames_rendered, 1234, "the frames are the ones the engine counted");
}

/// A daemon whose engine answered once and then stopped answers with the group states it
/// knew, failures and all. A group that failed is dropped when the answer is an empty
/// list, and the editor then shows the group as fine.
#[test]
fn the_scenes_route_answers_with_the_group_states_it_knew() {
    let rig = build();
    rig.engine.set_states(vec![GroupState {
        id: GROUP.into(),
        failed: Some("the effect could not be built".into()),
        fits: Vec::new(),
    }]);
    // The daemon's watcher fills the copy of the group states.
    let watch = canvas_daemon::state::watch_engine(Arc::clone(&rig.app));
    wait_for_group_asks(&rig.engine, 1);
    drop(watch);
    wait_for_the_watcher_to_stop(&rig.engine);
    // The engine stops answering, and the daemon's copy goes stale.
    rig.engine.set_delay(ENGINE_WAIT);
    set_max_age(&rig.app, Duration::from_millis(50));
    std::thread::sleep(Duration::from_millis(150));
    let answer = ask(rig.router, "/api/v1/scenes").expect("the scenes route never answered");
    assert_eq!(answer.status, StatusCode::OK, "the body: {}", answer.text());
    let scenes: ScenesResponse = answer.json();
    let group =
        scenes.group_states.iter().find(|g| g.id == GROUP).expect("the group is in the answer");
    assert_eq!(
        group.failed.as_deref(),
        Some("the effect could not be built"),
        "the failed group is not dropped: {group:?}"
    );
}

/// How old the daemon's copy of the engine's numbers may be.
fn set_max_age(app: &Arc<App>, age: Duration) {
    *app.engine_cache_max_age.lock().unwrap_or_else(|e| e.into_inner()) = age;
}

/// The daemon's copy of the numbers must not be the only source.
///
/// The watcher that keeps it fresh can stop (a render thread that never hands the lock
/// over, a quit that was cancelled). A copy that is older than the bound is asked of the
/// engine again, so the numbers move with the engine.
#[test]
fn the_status_route_asks_the_engine_again_once_its_copy_is_old() {
    let rig = build();
    rig.engine.set_numbers(RenderStats { fps_measured: 42.0, ..RenderStats::default() });
    let watch = canvas_daemon::state::watch_engine(Arc::clone(&rig.app));
    wait_for_stats_asks(&rig.engine, 1);
    // The watcher stops: the daemon's copy ages from here.
    drop(watch);
    wait_for_the_watcher_to_stop(&rig.engine);
    // The engine moves on.
    rig.engine.set_numbers(RenderStats { fps_measured: 99.0, ..RenderStats::default() });
    // The copy is older than the bound: the route asks the engine itself.
    std::thread::sleep(canvas_daemon::state::ENGINE_CACHE_MAX_AGE + Duration::from_millis(400));
    let asked = rig.engine.asked_stats();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut shown = None;
    while Instant::now() < deadline {
        let answer = ask(rig.router.clone(), "/api/v1/status").expect("the status route answered");
        let status: Status = answer.json();
        shown = Some(status.fps_measured);
        if shown == Some(99.0) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(shown, Some(99.0), "the status shows the engine's change");
    assert!(
        rig.engine.asked_stats() > asked,
        "the route did not ask the engine: it answered from a copy that had gone stale"
    );
}

/// The daemon's watcher threads are the production path: they keep the numbers fresh and
/// the routes read them.
///
/// The route answers within a second of the engine's change, and it answers in a moment:
/// a route that asked the engine itself would wait for the engine's answer, and the
/// engine takes three seconds for a route.
#[test]
fn the_routes_answer_from_the_cache_within_a_second_of_an_engine_change() {
    let rig = build();
    // The copy stays fresh for the whole test, so a slow machine cannot push the route
    // onto the engine while it is measured. The bound itself is proved by
    // `the_status_route_asks_the_engine_again_once_its_copy_is_old`.
    set_max_age(&rig.app, Duration::from_secs(60));
    // A route that asks the engine waits three seconds for the answer.
    rig.engine.set_slow_for_routes(Duration::from_secs(3));
    rig.engine.set_numbers(RenderStats { fps_measured: 42.0, ..RenderStats::default() });
    let watch = canvas_daemon::state::watch_engine(Arc::clone(&rig.app));
    wait_for_stats_asks(&rig.engine, 1);
    // The engine moves on.
    rig.engine.set_numbers(RenderStats { fps_measured: 99.0, ..RenderStats::default() });
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut shown = false;
    while Instant::now() < deadline {
        let answer = ask(rig.router.clone(), "/api/v1/status").expect("the status route answered");
        let status: Status = answer.json();
        if status.fps_measured == 99.0 {
            shown = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(shown, "the status route did not show the engine's change within a second");
    // The route answers from the daemon's copy: it does not ask the engine itself.
    drop(watch);
    wait_for_the_watcher_to_stop(&rig.engine);
    let asked = rig.engine.asked_stats();
    for _ in 0..5 {
        let answer = ask(rig.router.clone(), "/api/v1/status").expect("the status route answered");
        let status: Status = answer.json();
        assert_eq!(status.fps_measured, 99.0, "the route answered with the copy");
    }
    assert_eq!(
        rig.engine.asked_stats(),
        asked,
        "the route asked the engine instead of reading the daemon's copy"
    );
}

/// A quit that could not reach the main task puts the daemon's quit flag back to zero and
/// the daemon keeps running. The watcher must not have stopped: a watcher that stopped
/// there froze the numbers for the rest of the daemon's life.
#[test]
fn the_watcher_keeps_the_numbers_fresh_after_a_quit_that_was_cancelled() {
    let rig = build();
    rig.engine.set_numbers(RenderStats { fps_measured: 42.0, ..RenderStats::default() });
    let _watch = canvas_daemon::state::watch_engine(Arc::clone(&rig.app));
    wait_for_stats_asks(&rig.engine, 1);
    // The quit sets the flag and takes it back when the quit fails.
    rig.app.shutting_down.store(1, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(400));
    rig.app.shutting_down.store(0, Ordering::SeqCst);
    let asked = rig.engine.asked_stats();
    let deadline = Instant::now() + Duration::from_secs(10);
    while rig.engine.asked_stats() == asked && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        rig.engine.asked_stats() > asked,
        "the watcher stopped at the quit flag: the numbers froze"
    );
    // And the numbers still move with the engine.
    rig.engine.set_numbers(RenderStats { fps_measured: 99.0, ..RenderStats::default() });
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut shown = false;
    while Instant::now() < deadline {
        let answer = ask(rig.router.clone(), "/api/v1/status").expect("the status route answered");
        let status: Status = answer.json();
        if status.fps_measured == 99.0 {
            shown = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(shown, "the status route did not show the engine's change within a second");
}

/// The settings route must answer while a scenes request is stuck.
///
/// The scenes route used to hold the store's lock while it waited for the engine. The
/// store's lock is a fair one: a writer that queues behind that reader stops every
/// other reader too, and `/api/v1/settings` is one of them. This is the route the
/// capture polls, and the one the report says timed out.
///
/// The answer has to come well inside the route's own bound on the engine
/// ([`canvas_daemon::state::ENGINE_CALL_LIMIT`], 500 ms): a route that waits for the
/// engine shows up here as a wait of that size.
#[test]
fn the_settings_route_answers_while_a_scenes_request_is_stuck() {
    let rig = build();
    rig.engine.set_delay(ENGINE_WAIT);
    // A scenes request that waits inside the engine.
    let stuck = {
        let router = rig.router.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("the runtime is built");
            let _ = runtime.block_on(async move {
                router.oneshot(request("/api/v1/scenes")).await.map(|response| response.status())
            });
        })
    };
    wait_for_group_asks(&rig.engine, 1);
    // A writer queues behind the reader that holds the store's lock. It is a route that
    // takes the store's lock and never asks the engine, so it queues for the lock and
    // for nothing else.
    let writer = {
        let router = rig.router.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("the runtime is built");
            let _ = runtime.block_on(async move {
                router
                    .oneshot(request_with("POST", "/api/v1/first-run/dismiss-zone-panel"))
                    .await
                    .map(|response| response.status())
            });
        })
    };
    // The writer has the time to reach the lock.
    std::thread::sleep(Duration::from_millis(100));
    // The settings route is the one the capture polls. It must still answer.
    let answer = ask(rig.router, "/api/v1/settings")
        .expect("the settings route never answered: a stuck scenes request held the store's lock");
    assert_eq!(answer.status, StatusCode::OK, "the body: {}", answer.text());
    assert!(
        answer.took < Duration::from_millis(250),
        "the settings route waited {:?} for the store's lock",
        answer.took
    );
    let _ = stuck.join();
    let _ = writer.join();
}
