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
use canvas_adapters::openrgb::{OpenRgbAdapter, RescanTiming};
use canvas_daemon::api;
use canvas_daemon::openrgb_supervisor::RESCAN_BUDGET;
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

/// Short detection windows so a rescan takes under a second (the fake's gap between two
/// controllers registering is 10 ms). The daemon's own are `RescanTiming::default()`.
fn quick() -> RescanTiming {
    RescanTiming { first: Duration::from_millis(500), settle: Duration::from_millis(250), cap: Duration::from_secs(3) }
}

/// An app whose adapter is the real OpenRGB adapter on `port`, with the daemon's supervisor
/// running. `budget` is how long a rescan may hold the adapter.
async fn rig(name: &str, port: u16, budget: Duration) -> Rig {
    rig_timed(name, port, budget, quick()).await
}

async fn rig_timed(name: &str, port: u16, budget: Duration, timing: RescanTiming) -> Rig {
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
        Box::new(OpenRgbAdapter::new("127.0.0.1", port).with_rescan_timing(timing)),
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

/// Waits until the device has a frame sink, and returns it. The sink is added after the link
/// state is set, so a test that waits for the link alone can miss it.
async fn wait_for_sink(rig: &Rig, id: &str) -> Arc<dyn canvas_proto::traits::FrameSink> {
    let wanted = DeviceId(id.into());
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(sink) = rig.engine.sinks.lock().unwrap().get(&wanted).cloned() {
                return sink;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{id} never got a frame sink"))
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

/// `get` for a task that owns its own router.
async fn get_via(router: axum::Router, path: &str) -> (StatusCode, Vec<u8>) {
    let response = router.oneshot(request(path)).await.expect("the router answers");
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

/// #290: a rescan makes OpenRGB look at the hardware again, so a device it had not found is put in
/// the API's answer. The fake keeps the second stick hidden until packet 140 starts a detection
/// that completes; a plain GET never reveals it.
#[tokio::test]
async fn a_rescan_that_makes_openrgb_re_detect_puts_the_new_device_in_the_answer() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.add_hidden_device(stick("two"));
    let rig = rig("redetect", server.port(), RESCAN_BUDGET).await;
    wait_for_link(&rig, LinkState::Connected).await;
    assert_eq!(ids(&rig, "/api/v1/devices").await, ["openrgb:one"]);
    assert_eq!(server.rescan_count(), 0, "a plain read sent no rescan packet");

    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one", "openrgb:two"]);
    assert_eq!(server.rescan_count(), 1, "the rescan asked OpenRGB to look at the hardware");
    // The supervisor applied the new list, not just answered it.
    assert_eq!(rig.app.devices.read().await.len(), 2);
    assert_eq!(sink_ids(&rig), ["openrgb:one", "openrgb:two"]);
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
    let lights = rig.app.store.read().await.lights.lights.clone();
    assert!(
        lights.iter().any(|light| light.device_id.0 == "openrgb:one"),
        "the new device's zone is a light: {lights:?}"
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

    let placed = rig.app.store.read().await.lights.lights.clone();
    assert!(placed.iter().any(|light| light.device_id.0 == "openrgb:two"), "{placed:?}");
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

/// `?rescan=` follows the rule every boolean query parameter follows (#233): `true` or `1` is on,
/// `false` or `0` (or no parameter at all) is off, and anything else is a validation error in
/// the API's JSON shape. A typo is never a silent plain read.
#[tokio::test]
async fn the_rescan_flag_follows_the_rule_every_flag_follows() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let rig = rig("flag", server.port(), Duration::from_secs(5)).await;
    wait_for_link(&rig, LinkState::Connected).await;

    // OpenRGB finds a second stick after the daemon started.
    server.announce(stick("two"));
    // A flag that is on asks OpenRGB again: the new stick is in the answer.
    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one", "openrgb:two"]);
    assert_eq!(ids(&rig, "/api/v1/devices?rescan=true").await, ["openrgb:one", "openrgb:two"]);

    // A third stick, and a flag that is off: the plain read does not see it.
    server.announce(stick("three"));
    assert_eq!(ids(&rig, "/api/v1/devices?rescan=0").await, ["openrgb:one", "openrgb:two"]);
    assert_eq!(ids(&rig, "/api/v1/devices?rescan=false").await, ["openrgb:one", "openrgb:two"]);
    assert_eq!(ids(&rig, "/api/v1/devices").await, ["openrgb:one", "openrgb:two"]);
    // The same read with the flag on sees it.
    assert_eq!(
        ids(&rig, "/api/v1/devices?rescan=1").await,
        ["openrgb:one", "openrgb:two", "openrgb:three"]
    );

    // A typo is an error, not a silent plain read.
    let (status, body) = get(&rig, "/api/v1/devices?rescan=yes").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    // The flag twice is an error too.
    let (status, body) = get(&rig, "/api/v1/devices?rescan=1&rescan=1").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
}

/// A rescan against an OpenRGB that stops answering must not freeze the lights (#233): the frame
/// writer must not wait on the adapter lock the rescan holds.
///
/// The budget is the production one (`RESCAN_BUDGET`, 5 s), so the rescan may hold the adapter
/// that long. The frame must reach the lights while the rescan still holds it: the fake counts
/// the frames that arrive while it still owes a reply, and that count is what says the frame did
/// not wait for the rescan. The verdict is that count, not the clock: the old writer waited for
/// the rescan to give the adapter up, and no frame arrived while the reply was owed.
#[tokio::test]
async fn a_frame_reaches_the_lights_while_a_rescan_holds_the_adapter() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    // OpenRGB with detection off starts none, so its list is not changing and the frames flow; the
    // rescan goes straight to the re-read that holds the adapter.
    server.disable_detection(true);
    // OpenRGB takes packet 140 and acknowledges it, then stalls: it is the re-read that goes unanswered. A
    // server that stalls before the packet never acknowledges it, and the rescan then fails on the missing
    // ACK (`a_rescan_the_server_never_acknowledges_fails_and_keeps_the_held_list`).
    server.stall_after_the_rescan_packet(true);
    let rig = rig("stalled", server.port(), RESCAN_BUDGET).await;
    wait_for_link(&rig, LinkState::Connected).await;
    let sink = wait_for_sink(&rig, "openrgb:one").await;

    let rescan = {
        let router = rig.router.clone();
        tokio::spawn(async move { router.oneshot(request("/api/v1/devices?rescan=1")).await.expect("the router answers").status() })
    };
    // The supervisor now holds the adapter, waiting for a reply that never comes: the fake
    // swallowed the request.
    tokio::time::timeout(Duration::from_secs(30), async {
        while server.swallowed_requests() == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the rescan never reached OpenRGB");

    // A frame goes to the writer while the adapter is held, and must reach the lights.
    sink.submit(vec![Rgb8::WHITE; 8]);
    tokio::time::timeout(Duration::from_secs(30), async {
        while server.colors(0) != vec![Rgb8::WHITE; 8] {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the frame never reached the lights");
    // The frame arrived while the fake still owed the rescan its reply: it did not wait for the
    // rescan to give the adapter up.
    assert_eq!(
        server.frames_while_a_request_waited(),
        1,
        "the frame waited for the rescan to give the adapter up"
    );

    // The rescan itself answers that OpenRGB did not answer.
    let status = tokio::time::timeout(Duration::from_secs(30), rescan)
        .await
        .expect("the rescan answers")
        .expect("the request task ends");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    server.stall_replies(false);
}

/// Makes OpenRGB slow, not dead: every reply it holds (see `hold_replies`) goes out exactly `by`
/// after the request that asked for it. Abort the task to stop.
fn delay_replies(server: &Arc<FakeOpenRgbServer>, by: Duration) -> tokio::task::JoinHandle<()> {
    let server = Arc::clone(server);
    tokio::spawn(async move {
        loop {
            while server.held_replies() == 0 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            tokio::time::sleep(by).await;
            server.release_replies();
        }
    })
}

/// Starts `GET /api/v1/devices?rescan=1` and returns the task that waits for its status.
fn start_rescan(rig: &Rig) -> tokio::task::JoinHandle<StatusCode> {
    let router = rig.router.clone();
    tokio::spawn(async move {
        router.oneshot(request("/api/v1/devices?rescan=1")).await.expect("the router answers").status()
    })
}

/// Waits until `ready` is true; the long limit only ends a test that would hang.
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
}

/// #290: the wait for OpenRGB's detection holds no adapter lock. The keyboard's frame writer takes
/// that lock for every frame (`crates/canvas-daemon/src/writer.rs`, the non-OpenRGB path), so a
/// rescan that held it for the wait froze the keyboard for seconds.
///
/// Detection never settles here, so the wait runs to the 3 s cap; the keyboard-style frame
/// (a device id the composite serves natively) must be written inside one second of the rescan
/// starting to wait.
#[tokio::test]
async fn a_keyboard_frame_is_not_blocked_while_a_rescan_waits_for_detection() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.hold_detection(true);
    let rig = rig("keyboard-frame", server.port(), RESCAN_BUDGET).await;
    wait_for_link(&rig, LinkState::Connected).await;

    let rescan = start_rescan(&rig);
    // Packet 140 is at OpenRGB: the supervisor is now waiting for the list to go quiet.
    until("packet 140 reached OpenRGB", || server.rescan_count() == 1).await;

    let (frames, rx) = tokio::sync::watch::channel(Vec::new());
    let writer = tokio::spawn(canvas_daemon::writer::writer_loop(
        rig.app.adapter.clone(),
        DeviceId("royuan:keyboard".into()),
        rx,
        Arc::new(canvas_daemon::last_frame::LastFrame::default()),
    ));
    frames.send_replace(vec![Rgb8::WHITE; 4]);
    drop(frames);
    tokio::time::timeout(Duration::from_secs(1), writer)
        .await
        .expect("the keyboard's frame waited on the adapter lock for the rescan's detection wait")
        .expect("the writer ends");
    assert!(!rescan.is_finished(), "the rescan was already over, so the frame proved nothing");
    assert_eq!(rescan.await.expect("the request task ends"), StatusCode::OK);
}

/// #290: a detection that never goes quiet leaves OpenRGB's list empty and changing. The rescan
/// answers 200 with the list the daemon holds, applies nothing, never reads the list, and the link
/// stays up: the lights stay managed.
#[tokio::test]
async fn a_detection_that_never_settles_keeps_the_held_list_and_the_lights() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.hold_detection(true);
    let rig = rig("unsettled", server.port(), RESCAN_BUDGET).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:one").await;

    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one"], "the held list answers");
    assert_eq!(server.rescan_count(), 1);
    assert_eq!(server.reads_during_detection(), 0, "the rescan read the list while it was changing");
    assert_eq!(rig.app.devices.read().await.len(), 1, "the held list was replaced");
    assert_eq!(sink_ids(&rig), ["openrgb:one"], "the device lost its frame sink");
    assert!(rig.app.adapter.lock().await.is_connected(), "the rescan dropped the link");
    assert_eq!(*rig.app.openrgb.read().await, LinkState::Connected);
}

/// #290 review 3 (B1): OpenRGB's own detection is already running when the owner presses rescan, so
/// release_1.1 drops packet 140 without a word (`DetectionManager.cpp:1753-1756`) and the list is
/// the empty one the detection cleared. The answer is the list the daemon holds, the daemon's list
/// and its frame sinks stay, and so do they after the 5 s refresh reads the empty list too.
#[tokio::test]
async fn an_ignored_rescan_during_a_running_detection_keeps_the_daemons_list_and_sinks() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    // An older OpenRGB (protocol 5): the daemon's refresh reads the list mid-detection and the adapter
    // answers with the one it holds. Protocol 6 reads nothing then (the test after this one).
    server.server_protocol_version(5);
    // The detection is in its first minute: its list is empty, with no list change after the clear.
    server.rescan_timing(Duration::from_secs(60), Duration::from_millis(10));
    let rig = rig("ignored", server.port(), RESCAN_BUDGET).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:two").await;

    server.start_detection();
    // OpenRGB has announced the clear before the owner presses the button (the usual order).
    until("OpenRGB announced the cleared list", || server.list_cleared()).await;
    let answer = ids(&rig, "/api/v1/devices?rescan=1").await;
    assert_eq!(answer, ["openrgb:one", "openrgb:two"], "the held list answers");
    assert_eq!(server.rescan_dropped(), 1, "OpenRGB must have dropped the packet 140 of a running detection");
    assert_eq!(rig.app.devices.read().await.len(), 2, "the empty list wiped the daemon's list");
    assert_eq!(sink_ids(&rig), ["openrgb:one", "openrgb:two"], "the empty list removed the frame sinks");

    // The supervisor's own 5 s refresh reads the list too: empty, and the held list stays.
    let reads = server.reads_during_detection();
    until("the 5 s refresh read the list", || server.reads_during_detection() > reads).await;
    assert_eq!(rig.app.devices.read().await.len(), 2, "the refresh applied the empty list");
    assert_eq!(sink_ids(&rig), ["openrgb:one", "openrgb:two"], "the refresh removed the frame sinks");
    assert_eq!(*rig.app.openrgb.read().await, LinkState::Connected);
}

/// #290, protocol 6: a packet 140 that arrives while OpenRGB's own detection runs is dropped by OpenRGB
/// (`DetectionManager.cpp:1753-1756`), so it joins the running detection. The API answers the list the
/// daemon holds at the call's bound (15 s), the daemon's list and sinks stay, and the list is not even
/// read until 103, however long the detection takes: the new list is applied once, when it ends.
#[tokio::test(start_paused = true)]
async fn a_rescan_during_a_running_detection_joins_it_and_the_new_list_applies_after_103() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    server.add_hidden_device(stick("three"));
    // A minute for three controllers.
    server.rescan_timing(Duration::from_millis(100), Duration::from_secs(20));
    let rig = rig_timed("joins", server.port(), RESCAN_BUDGET, RescanTiming::default()).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:two").await;

    server.start_detection();
    until("OpenRGB announced the cleared list", || server.list_cleared()).await;
    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one", "openrgb:two"], "the held list answers");
    assert_eq!(server.rescan_dropped(), 1, "OpenRGB must have dropped the packet 140 of a running detection");
    assert_eq!(sink_ids(&rig), ["openrgb:one", "openrgb:two"], "the rescan removed a frame sink");
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if server.list_cleared() {
            assert_eq!(rig.app.devices.read().await.len(), 2, "the list changed while the detection ran");
        }
    }
    tokio::time::timeout(Duration::from_secs(60), until_devices(&rig, 3))
        .await
        .expect("the list the detection ended with was never applied");
    assert_eq!(server.reads_during_detection(), 0, "the list was read while OpenRGB was still detecting");
    assert_eq!(sink_ids(&rig), ["openrgb:one", "openrgb:three", "openrgb:two"]);
    assert_eq!(*rig.app.openrgb.read().await, LinkState::Connected);
}

/// #290 review 4: two controllers of one detection register further apart than the 2 s window the
/// protocol 5 wait used. That wait took the quiet between them for the end and applied a part list
/// (three devices to one, the frame sinks of the others removed). With 103 there is no window: the
/// daemon's list and sinks stay whole through the detection and the answer is the whole new list.
#[tokio::test(start_paused = true)]
async fn a_gap_longer_than_two_seconds_between_registrations_applies_no_part_list() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two"), stick("three")]).await.expect("fake server");
    server.rescan_timing(Duration::from_millis(100), Duration::from_secs(3));
    let rig = rig_timed("gap", server.port(), RESCAN_BUDGET, RescanTiming::default()).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:three").await;

    let rescan = {
        let router = rig.router.clone();
        tokio::spawn(async move { get_via(router, "/api/v1/devices?rescan=1").await })
    };
    let mut counts = Vec::new();
    while !rescan.is_finished() {
        counts.push((rig.app.devices.read().await.len(), sink_ids(&rig).len()));
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let (status, body) = rescan.await.expect("the request task ends");
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let answered: DevicesResponse = serde_json::from_slice(&body).expect("a device list");
    assert_eq!(answered.devices.len(), 3, "the answer was a part list");
    assert!(counts.len() > 10, "the loop did not run during the detection: {counts:?}");
    assert!(counts.iter().all(|&count| count == (3, 3)), "the daemon's list or frame sinks shrank mid-detection: {counts:?}");
    assert_eq!(rig.app.devices.read().await.len(), 3);
}

/// #290, protocol 6: every device really gone. OpenRGB said the detection is over (103) and has no
/// controller left: that list is the real one, and it is applied, sinks included. Without 103 an empty
/// list could be a detection that has found nothing yet, and the protocol 5 path never applies one.
#[tokio::test(start_paused = true)]
async fn every_device_unplugged_applies_the_empty_list_once_openrgb_says_detection_is_over() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    server.rescan_timing(Duration::from_millis(100), Duration::from_secs(1));
    let rig = rig_timed("all-gone", server.port(), RESCAN_BUDGET, RescanTiming::default()).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:two").await;
    server.unplug_device(1);
    server.unplug_device(0);

    assert!(ids(&rig, "/api/v1/devices?rescan=1").await.is_empty(), "the empty list after 103 was not the answer");
    assert!(rig.app.devices.read().await.is_empty(), "the unplugged devices stayed in the daemon's list");
    assert!(sink_ids(&rig).is_empty(), "the unplugged devices kept their frame sinks");
    assert_eq!(*rig.app.openrgb.read().await, LinkState::Connected);
}

/// #290 review 6: OpenRGB restarts and the daemon reconnects while its detection runs, so the connection
/// never saw the 101 and holds no list of its own. release_1.1 signals progress per detector, not per
/// controller (`DetectionManager.cpp:491`, `:720`, `:1009`), so all the connection sees of the detection is
/// list changes. Its first read is OpenRGB's part list (one controller of four), and the daemon used to take
/// it for the list: three devices became one, and the others lost their frame sinks. The daemon keeps the list
/// it had across the reconnect until the detection is over, then has the whole new one.
#[tokio::test(start_paused = true)]
async fn a_reconnect_into_a_running_detection_keeps_the_daemons_list_and_sinks_until_it_ends() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two"), stick("three")]).await.expect("fake server");
    server.add_hidden_device(stick("four"));
    server.quiet_progress(true);
    // A controller registers every 4 s from the moment a client is there to hear it: sixteen seconds.
    server.rescan_timing(Duration::ZERO, Duration::from_secs(4));
    let rig = rig_timed("restart-mid-detection", server.port(), RESCAN_BUDGET, RescanTiming::default()).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:three").await;
    assert_eq!(rig.app.devices.read().await.len(), 3);

    server.restart().await;
    server.start_detection();
    let mut seen = Vec::new();
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        seen.push((rig.app.devices.read().await.len(), sink_ids(&rig).len()));
    }
    assert_eq!(seen[0], (3, 3), "the first sample after the restart: {seen:?}");
    assert!(
        seen.iter().all(|&(devices, sinks)| devices == sinks && (devices == 3 || devices == 4)),
        "the daemon's list or frame sinks held a part list while OpenRGB detected: {seen:?}"
    );
    assert!(seen.windows(2).all(|pair| pair[0].0 <= pair[1].0), "the list went back down: {seen:?}");
    assert_eq!(*seen.last().expect("samples"), (4, 4), "the list the detection ended with was never applied: {seen:?}");
    assert_eq!(sink_ids(&rig), ["openrgb:four", "openrgb:one", "openrgb:three", "openrgb:two"]);
    assert_eq!(*rig.app.openrgb.read().await, LinkState::Connected);
}

/// #290 review 6: the same reconnect, and the owner presses rescan while the detection runs. The connection
/// never saw the 101, so OpenRGB drops its packet 140 (`DetectionManager.cpp:1753-1756`) and answers with an
/// ACK alone. That is a running detection, not "did not start" (a 503): the call waits for the 103, and the
/// API answers the whole new list, with the daemon's list and sinks whole all the way.
#[tokio::test(start_paused = true)]
async fn a_rescan_after_a_reconnect_into_a_running_detection_joins_its_103() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two"), stick("three")]).await.expect("fake server");
    server.add_hidden_device(stick("four"));
    server.quiet_progress(true);
    // Four controllers 2 s apart: the 103 comes 8 s after the reconnect, inside the call's 15 s bound.
    server.rescan_timing(Duration::ZERO, Duration::from_secs(2));
    let rig = rig_timed("rescan-after-restart", server.port(), RESCAN_BUDGET, RescanTiming::default()).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:three").await;

    server.restart().await;
    server.start_detection();
    until("the daemon reconnected into the detection", || server.rescan_count() == 0 && server.list_cleared()).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(rig.app.devices.read().await.len(), 3, "the reconnect applied a part list");

    let (status, body) = get(&rig, "/api/v1/devices?rescan=1").await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let answered: DevicesResponse = serde_json::from_slice(&body).expect("a device list");
    assert_eq!(answered.devices.len(), 4, "the answer was not the whole list the detection ended with");
    assert_eq!(server.rescan_dropped(), 1, "OpenRGB must have dropped the packet 140 of a running detection");
    assert_eq!(rig.app.devices.read().await.len(), 4);
    assert_eq!(sink_ids(&rig), ["openrgb:four", "openrgb:one", "openrgb:three", "openrgb:two"]);
}

/// A server that never acknowledges packet 140 (protocol 6) is not answering: the rescan fails with the
/// API's error, and is not taken for a detection that is still running.
#[tokio::test(start_paused = true)]
async fn a_rescan_the_server_never_acknowledges_fails_and_keeps_the_held_list() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let rig = rig_timed("no-ack", server.port(), RESCAN_BUDGET, RescanTiming::default()).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:one").await;
    server.stall_replies(true);

    let (status, body) = get(&rig, "/api/v1/devices?rescan=1").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{}", String::from_utf8_lossy(&body));
    assert!(String::from_utf8_lossy(&body).contains("did not acknowledge"), "{}", String::from_utf8_lossy(&body));
    assert_eq!(rig.app.devices.read().await.len(), 1, "the held list was replaced");
    server.stall_replies(false);
}

/// Waits (on the paused clock) until the app holds `count` devices.
async fn until_devices(rig: &Rig, count: usize) {
    loop {
        if rig.app.devices.read().await.len() == count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// #290 review 3 (B2): a detection can run for a minute. The old 45 s and 30 s distrust guesses ran
/// out and the 5 s refresh applied the empty list. No list OpenRGB has while it is changing is
/// applied at any point, however long it takes, and the final list is applied once it has settled.
/// The clock is paused: the minute is virtual, and a refresh happens every virtual 5 s.
#[tokio::test(start_paused = true)]
async fn a_detection_that_runs_a_minute_wipes_nothing_and_its_list_applies_once_settled() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    server.add_hidden_device(stick("three"));
    // The list is empty for a minute after the clear; then each controller registers 400 ms on.
    server.rescan_timing(Duration::from_secs(60), Duration::from_millis(400));
    let rig = rig("minute", server.port(), RESCAN_BUDGET).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:two").await;

    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one", "openrgb:two"], "the held list answers");
    for second in (5..=55).step_by(5) {
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(rig.app.devices.read().await.len(), 2, "the list was wiped {second} s into the detection");
        assert_eq!(sink_ids(&rig), ["openrgb:one", "openrgb:two"], "a frame sink went {second} s into the detection");
    }
    tokio::time::timeout(Duration::from_secs(30), until_devices(&rig, 3))
        .await
        .expect("the list the detection settled on was never applied");
    assert_eq!(sink_ids(&rig), ["openrgb:one", "openrgb:three", "openrgb:two"]);
}

/// #290 review 3: a device that is really gone is removed once the list settles, and not before. The
/// detection is slow (the list is empty for 6 s), so the first answer is the held list; the removal
/// follows by itself, within one refresh of the list settling, and the old 30 s distrust is gone.
#[tokio::test(start_paused = true)]
async fn a_device_unplugged_before_a_slow_detection_is_removed_when_the_list_settles() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    server.rescan_timing(Duration::from_secs(6), Duration::from_millis(10));
    let rig = rig("unplugged", server.port(), RESCAN_BUDGET).await;
    wait_for_link(&rig, LinkState::Connected).await;
    wait_for_sink(&rig, "openrgb:two").await;
    server.unplug_device(1);

    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one", "openrgb:two"], "the held list answers");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(rig.app.devices.read().await.len(), 2, "the device went before the list had settled");
    tokio::time::timeout(Duration::from_secs(15), until_devices(&rig, 1))
        .await
        .expect("the unplugged device was not removed once the list settled");
    assert_eq!(sink_ids(&rig), ["openrgb:one"], "the removed device kept its frame sink");
}

/// #290 review 3 (N3): while OpenRGB's list is changing no frame goes to any OpenRGB device, and
/// they resume once the list has settled. The list is empty for 700 ms after the clear; a frame sent
/// meanwhile would name an index OpenRGB has no controller for (or, with identical sticks, the wrong
/// one).
#[tokio::test]
async fn no_frame_reaches_openrgb_while_its_list_changes_and_the_lights_resume_after() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.rescan_timing(Duration::from_millis(700), Duration::from_millis(100));
    let rig = rig("gated", server.port(), RESCAN_BUDGET).await;
    wait_for_link(&rig, LinkState::Connected).await;
    let sink = wait_for_sink(&rig, "openrgb:one").await;

    let rescan = start_rescan(&rig);
    until("packet 140 reached OpenRGB", || server.rescan_count() == 1).await;
    let mut shade = 1_u8;
    while !rescan.is_finished() {
        sink.submit(vec![Rgb8 { r: shade, g: shade, b: shade }; 8]);
        shade = shade.wrapping_add(1).max(1);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let status = rescan.await.expect("the request task ends");
    let unregistered = server.frames_to_unregistered();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unregistered, 0, "a frame was sent to an index OpenRGB had not registered");

    sink.submit(vec![Rgb8::WHITE; 8]);
    assert_eq!(server.wait_for_colors(0, &[Rgb8::WHITE; 8]).await, vec![Rgb8::WHITE; 8], "the lights did not resume");
}

/// #290: the detection wait and the re-read have budgets of their own. Five sticks register again
/// 300 ms apart, so detection takes about 1.9 s (the last one at 1.5 s, then the 400 ms quiet
/// window); the re-read is six requests (the count and five controllers) whose replies come
/// 200 ms late each, 1.2 s in all. The rescan budget is 2.5 s: a budget that had to cover both
/// (3.1 s) would cut the re-read and drop the link, as it did.
#[tokio::test]
async fn a_slow_detection_does_not_eat_the_budget_of_the_re_read() {
    let sticks: Vec<DeviceInfo> = ["one", "two", "three", "four", "five"].into_iter().map(stick).collect();
    let server = Arc::new(FakeOpenRgbServer::start(sticks).await.expect("fake server"));
    server.rescan_timing(Duration::from_millis(20), Duration::from_millis(300));
    let timing = RescanTiming { first: Duration::from_secs(2), settle: Duration::from_millis(400), cap: Duration::from_secs(6) };
    let rig = rig_timed("budgets", server.port(), Duration::from_millis(2500), timing).await;
    wait_for_link(&rig, LinkState::Connected).await;

    // OpenRGB answers slowly: every reply of the re-read comes 200 ms after its request.
    server.hold_replies(true);
    let releasing = delay_replies(&server, Duration::from_millis(200));
    let status = start_rescan(&rig).await.expect("the request task ends");
    releasing.abort();
    assert_eq!(status, StatusCode::OK, "the re-read was cut by the budget that detection used up");
    assert!(rig.app.adapter.lock().await.is_connected(), "the rescan dropped the link");
    assert_eq!(*rig.app.openrgb.read().await, LinkState::Connected);
    assert_eq!(rig.app.devices.read().await.len(), 5);
}

/// #290: a rescan that gives up on detection can never drop the link, even while OpenRGB answers
/// slowly: the detection wait is outside the budget of the re-read, and a detection that never
/// settles makes no re-read at all.
#[tokio::test]
async fn a_detection_timeout_with_slow_replies_still_keeps_the_link_and_the_held_list() {
    let server = Arc::new(FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server"));
    server.hold_detection(true);
    let rig = rig("timeout-slow", server.port(), Duration::from_secs(2)).await;
    wait_for_link(&rig, LinkState::Connected).await;

    server.hold_replies(true);
    let releasing = delay_replies(&server, Duration::from_millis(400));
    let status = start_rescan(&rig).await.expect("the request task ends");
    releasing.abort();
    assert_eq!(status, StatusCode::OK);
    assert!(rig.app.adapter.lock().await.is_connected(), "a detection timeout dropped the link");
    assert_eq!(*rig.app.openrgb.read().await, LinkState::Connected);
    assert_eq!(rig.app.devices.read().await.len(), 1, "the held list was replaced");
}

/// #290: the API waits for the supervisor past the detection wait too. A detection that runs to a
/// 7 s cap is longer than the rescan budget (1 s) plus the queue wait (5 s) alone; the answer is
/// still the held list (200), not "OpenRGB did not answer the rescan in time" (503).
#[tokio::test]
async fn the_api_waits_out_the_detection_cap_before_it_gives_up_on_the_supervisor() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.hold_detection(true);
    let timing = RescanTiming { first: Duration::from_millis(500), settle: Duration::from_millis(250), cap: Duration::from_secs(7) };
    let rig = rig_timed("api-wait", server.port(), Duration::from_secs(1), timing).await;
    wait_for_link(&rig, LinkState::Connected).await;

    let (status, body) = get(&rig, "/api/v1/devices?rescan=1").await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

/// #290 review 2: the answer is the whole new list even when OpenRGB's detection is slow to bring
/// its first controller back. On release_1.1 the list is cleared when detection starts and the first
/// controller registers only after the I2C and DRAM detectors (`DetectionManager.cpp:810-856`,
/// `:1030-1100`), seconds later; the rescan used to answer 750 ms after the clear with the old
/// device only, while the new one appeared ~6 s on. The windows are the daemon's own
/// (`RescanTiming::default()`); the fake clears the list for 2.5 s, then registers a controller
/// every 400 ms.
#[tokio::test]
async fn a_rescan_with_a_slow_first_controller_answers_with_the_whole_new_list() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.add_hidden_device(stick("two"));
    server.rescan_timing(Duration::from_millis(2500), Duration::from_millis(400));
    let rig = rig_timed("slow-first", server.port(), RESCAN_BUDGET, RescanTiming::default()).await;
    wait_for_link(&rig, LinkState::Connected).await;
    assert_eq!(ids(&rig, "/api/v1/devices").await, ["openrgb:one"]);

    assert_eq!(ids(&rig, "/api/v1/devices?rescan=1").await, ["openrgb:one", "openrgb:two"]);
    assert_eq!(rig.app.devices.read().await.len(), 2, "the answer was not applied");
    assert_eq!(sink_ids(&rig), ["openrgb:one", "openrgb:two"]);
}
