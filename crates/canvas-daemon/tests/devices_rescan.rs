//! #233: `GET /api/v1/devices?rescan=1` asks OpenRGB again and answers with the new list; a plain
//! `GET` still answers from the list the daemon holds.
//!
//! The adapter is the real OpenRGB adapter talking to the fake OpenRGB server: no hardware. The
//! supervisor is the daemon's own (`device_sync::supervise_app`), so a rescan takes the path it
//! takes in the daemon: the one path every device list takes.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::api::{DevicesResponse, LinkState};
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::{DeviceId, Rgb8};
use common::FakeEngine;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn stick(serial: &str) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(format!("openrgb:{serial}")),
        name: format!("DDR5 {serial}"),
        vendor: "fake".into(),
        kind: DeviceKind::Dram,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: serial.into(),
        location: "smbus".into(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "RGB".into(),
            kind: ZoneKind::Linear,
            led_count: 8,
            min_leds: 8,
            max_leds: 8,
            resizable: false,
        }],
        connected: true,
    }
}

struct Rig {
    router: axum::Router,
    app: Arc<App>,
    engine: Arc<FakeEngine>,
    home: std::path::PathBuf,
    stop: tokio::sync::watch::Sender<bool>,
    supervisor: tokio::task::JoinHandle<()>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.supervisor.abort();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// An app whose adapter is the real OpenRGB adapter on `port`, with the daemon's supervisor
/// running. `budget` is how long a rescan may hold the adapter.
async fn rig(name: &str, port: u16, budget: Duration) -> Rig {
    let home = std::env::temp_dir().join(format!("canvas-rgb-rescan-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp directory is created");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let engine = Arc::new(FakeEngine::default());
    let mut store = canvas_daemon::storage::Store::at(&home);
    // The first run is its own test; here a new zone is placed the way it is after it.
    store.config.first_run_done = true;
    let mut app = App::new(
        store,
        engine.clone(),
        Box::new(OpenRgbAdapter::new("127.0.0.1", port)),
        TOKEN.into(),
        "0.0.0-test".into(),
        tx,
    );
    app.rescan_budget = budget;
    let app = Arc::new(app);
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let supervisor = tokio::spawn(canvas_daemon::device_sync::supervise_app(app.clone(), stop_rx));
    Rig { router: api::router(app.clone()), app, engine, home, stop, supervisor }
}

/// Waits until the app is in `state`. The wait is on the state the assert reads (#282); the long
/// limit only ends a test that would hang.
async fn wait_for_link(rig: &Rig, state: LinkState) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while *rig.app.openrgb.read().await != state {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the link never became {state:?}"));
}

fn request(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

async fn get(rig: &Rig, path: &str) -> (StatusCode, Vec<u8>) {
    let response = rig.router.clone().oneshot(request(path)).await.expect("the router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("the body reads");
    (status, bytes.to_vec())
}

async fn ids(rig: &Rig, path: &str) -> Vec<String> {
    let (status, body) = get(rig, path).await;
    assert_eq!(status, StatusCode::OK, "{path}: {}", String::from_utf8_lossy(&body));
    let list: DevicesResponse = serde_json::from_slice(&body).expect("a device list");
    list.devices.into_iter().map(|d| d.id.0).collect()
}

fn sink_ids(rig: &Rig) -> Vec<String> {
    rig.engine.sinks.lock().unwrap().keys().map(|id| id.0.clone()).collect()
}

/// A port nobody listens on. A plain listener closes its socket as it drops; the fake server
/// closes it a moment later, on its acceptor task, so it cannot hand out a dead port.
fn dead_port() -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
    listener.local_addr().expect("its address").port()
}

#[tokio::test]
async fn a_device_added_after_start_appears_only_after_a_rescan() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let rig = rig("appears", server.port(), Duration::from_secs(5)).await;
    wait_for_link(&rig, LinkState::Connected).await;
    assert_eq!(ids(&rig, "/api/v1/devices").await, ["openrgb:one"]);

    // OpenRGB finds a second stick after the daemon started.
    server.announce(stick("two"));

    // A plain GET answers from the list the daemon holds.
    assert_eq!(ids(&rig, "/api/v1/devices").await, ["openrgb:one"], "a plain GET does not ask OpenRGB");
    // The rescan asks, and answers with the new list.
    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one", "openrgb:two"]);
    // The held list is the new one too: a plain GET after the rescan sees the device.
    assert_eq!(ids(&rig, "/api/v1/devices").await, ["openrgb:one", "openrgb:two"]);
    assert_eq!(rig.app.devices.read().await.len(), 2);
}

/// The rescan goes through the path every device list takes (#233): when it answers, the status,
/// the zones and the frame sink are updated, with no supervisor tick in between.
#[tokio::test]
async fn a_rescan_that_connects_a_down_link_updates_status_zones_and_sinks_at_once() {
    let port = dead_port();
    let rig = rig("down-link", port, Duration::from_secs(5)).await;
    // The supervisor failed its first attempt and waits out its backoff (250 ms).
    wait_for_link(&rig, LinkState::Down).await;
    assert!(rig.app.devices.read().await.is_empty());
    assert!(sink_ids(&rig).is_empty());

    // OpenRGB comes up on that port, and the owner presses Try again.
    let _server = FakeOpenRgbServer::start_on_port(vec![stick("one")], port).await.expect("fake server");
    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one"]);

    // Right after the answer: the link, the list, the zones and the sink.
    assert_eq!(*rig.app.openrgb.read().await, LinkState::Connected, "the status follows the rescan");
    assert_eq!(rig.app.devices.read().await.len(), 1);
    let placed = rig.app.active_layout().await.expect("a layout").zones;
    assert!(
        placed.iter().any(|zone| zone.device_id.0 == "openrgb:one"),
        "the new device's zone is placed on the canvas: {placed:?}"
    );
    assert_eq!(sink_ids(&rig), ["openrgb:one"], "the new device has a frame sink");
}

/// A device that appears on a live link is placed and driven by the rescan, as by a refresh.
#[tokio::test]
async fn a_rescan_gives_a_new_device_its_zones_and_sink_at_once() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let rig = rig("new-device", server.port(), Duration::from_secs(5)).await;
    wait_for_link(&rig, LinkState::Connected).await;
    server.announce(stick("two"));

    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one", "openrgb:two"]);

    let placed = rig.app.active_layout().await.expect("a layout").zones;
    assert!(placed.iter().any(|zone| zone.device_id.0 == "openrgb:two"), "{placed:?}");
    assert_eq!(sink_ids(&rig), ["openrgb:one", "openrgb:two"]);
}

#[tokio::test]
async fn a_rescan_with_openrgb_down_says_so_and_keeps_the_held_list() {
    let port = dead_port();
    // Nothing answers on the port: a plain connection is refused, so the 503 below comes from
    // OpenRGB being down and from nothing else.
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err(),
        "the port must be dead before the rescan"
    );
    let rig = rig("down", port, Duration::from_secs(5)).await;
    wait_for_link(&rig, LinkState::Down).await;
    *rig.app.devices.write().await = vec![stick("one")];

    let (status, body) = get(&rig, "/api/v1/devices?rescan=1").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{}", String::from_utf8_lossy(&body));
    assert_eq!(ids(&rig, "/api/v1/devices").await, ["openrgb:one"], "the held list is untouched");
    assert_ne!(*rig.app.openrgb.read().await, LinkState::Connected);
}

/// A rescan against an OpenRGB that stops answering must not freeze the lights (#233): the frame
/// writer waits on the same adapter lock, so the rescan lets go of it after its budget.
///
/// The server stalls and never answers. The budget is 100 ms; the adapter's own reply timeout is
/// 5 s, so a rescan that held the adapter for the reply timeout (or for the old 25 s budget)
/// holds a frame back for longer than the 4 s this test allows.
#[tokio::test]
async fn a_rescan_against_a_stalled_openrgb_lets_the_frames_through() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let rig = rig("stalled", server.port(), Duration::from_millis(100)).await;
    wait_for_link(&rig, LinkState::Connected).await;
    let sink = rig
        .engine
        .sinks
        .lock()
        .unwrap()
        .get(&DeviceId("openrgb:one".into()))
        .cloned()
        .expect("a sink");

    server.stall_replies(true);
    let rescan = {
        let router = rig.router.clone();
        tokio::spawn(async move { router.oneshot(request("/api/v1/devices?rescan=1")).await.expect("the router answers").status() })
    };
    // The supervisor now holds the adapter, waiting for a reply that never comes.
    tokio::time::timeout(Duration::from_secs(30), async {
        while rig.app.adapter.try_lock().is_ok() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the rescan never took the adapter");

    // A frame goes to the writer while the adapter is held, and must reach the lights.
    sink.submit(vec![Rgb8::WHITE; 8]);
    let arrived = tokio::time::timeout(Duration::from_secs(4), async {
        while server.colors(0) != vec![Rgb8::WHITE; 8] {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(arrived.is_ok(), "the rescan held the frame back for 4 s");

    // The rescan itself answers that OpenRGB did not answer.
    let status = tokio::time::timeout(Duration::from_secs(30), rescan)
        .await
        .expect("the rescan answers")
        .expect("the request task ends");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(rig.app.adapter.try_lock().is_ok(), "the adapter is free once the rescan answered");
    server.stall_replies(false);
}
