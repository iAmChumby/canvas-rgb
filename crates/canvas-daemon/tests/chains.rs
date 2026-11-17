//! The chain routes (#338, Tasks 7a and 7b): `GET /api/v1/chains`, a setup session's start,
//! light, cancel and empty, with the header's earlier length put back when the session ends
//! short, and the commit that writes the parts, their groups and the header's length together or
//! leaves everything as it was.
//!
//! The rig is the one `tests/devices_rescan.rs` uses: the real OpenRGB adapter on the fake OpenRGB
//! server, the daemon's own supervisor (`device_sync::supervise_app`), a temporary home and a
//! router driven with `oneshot`. No hardware is touched and port 9430 is never bound.
//!
//! Every wait is a poll loop with a deadline: nothing reads its verdict off a wall clock, and no
//! test sleeps and then asserts that something happened.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_daemon::api;
use canvas_daemon::chain_plan::plan_commit;
use canvas_daemon::chain_records::{
    read_intent, read_restore, write_intent, write_restore, IntentRecord, RestoreRecord,
};
use canvas_daemon::scenes::Held;
use canvas_daemon::state::App;
use canvas_daemon::storage::{snapshot, Store};
use canvas_proto::api::{
    routes, ChainSetupState, ChainsResponse, LightView, SetupCommit, SetupCommitted, SetupEmpty,
    SetupPart, SetupStarted,
};
use canvas_proto::config::{ZonesFile, ZONES_FILE};
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::LedSlice;
use canvas_proto::scene::{
    part_light_id, zone_light_id, Arrangement, ChainPartKind, ChainState, Group, GroupMode, Light,
    LightsFile, Plays, Scene, ScenesFile, CHAIN_INTENT_FILE, CHAIN_RESTORE_FILE, LIGHTS_FILE,
    LIGHTS_SCHEMA, SCENES_FILE, SCENES_SCHEMA,
};
use canvas_proto::traits::FrameSink;
use canvas_proto::{DeviceId, Rgb8};
use common::FakeEngine;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// The board whose two JARGB headers are the chains under test.
const BOARD: &str = "openrgb:mb";
/// A device that is not a chain: its zone cannot be resized.
const RAM: &str = "openrgb:ram";
/// The earlier length of the board's first header: what first run sized it to.
const EARLIER: u32 = 120;
/// The second header's earlier length.
const SECOND_EARLIER: u32 = 60;

/// A chain header: resizable, 1 to 300 LEDs.
fn header(index: u32, name: &str, leds: u32) -> ZoneInfo {
    ZoneInfo {
        index,
        name: name.into(),
        kind: ZoneKind::Linear,
        led_count: leds,
        min_leds: 1,
        max_leds: 300,
        resizable: true,
    }
}

/// The motherboard with two JARGB headers. `first` is the length the first header reports, which
/// a crash mid-setup leaves raised.
fn board(first: u32) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(BOARD.into()),
        name: "MSI MPG B850 Mystic Light".into(),
        vendor: "Fake".into(),
        kind: DeviceKind::Motherboard,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "mb".into(),
        location: "usb".into(),
        zones: vec![header(0, "JARGB1", first), header(1, "JARGB2", SECOND_EARLIER)],
        connected: true,
    }
}

/// A board with four resizable ARGB headers, for the #470 walk of every path the UI offers.
fn board_four() -> DeviceInfo {
    let names = ["JAF", "JARGB 1", "JARGB 2", "JARGB 3"];
    DeviceInfo {
        id: DeviceId(BOARD.into()),
        name: "MSI MPG B850 Mystic Light".into(),
        vendor: "Fake".into(),
        kind: DeviceKind::Motherboard,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "mb".into(),
        location: "usb".into(),
        zones: names.iter().enumerate().map(|(i, name)| header(i as u32, name, 8)).collect(),
        connected: true,
    }
}

/// A second device that is not resizable, so it is not a chain.
fn ram() -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(RAM.into()),
        name: "DDR5".into(),
        vendor: "Fake".into(),
        kind: DeviceKind::Dram,
        adapter: "openrgb".into(),
        adapter_index: 1,
        serial: "ram".into(),
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

async fn fake(first: u32) -> FakeOpenRgbServer {
    FakeOpenRgbServer::start(vec![board(first), ram()]).await.expect("fake server")
}

/// The whole-zone light of a header, as first run leaves it: no parts, no name.
fn whole_zone(device: &str, zone: u32) -> Light {
    let id = DeviceId(device.into());
    Light {
        id: zone_light_id(&id, zone),
        device_id: id,
        zone_index: zone,
        leds: None,
        name: None,
        arrangement: Arrangement::Strip,
        reversed: false,
        kind: None,
        copies: None,
    }
}

/// The hardware map. No chain record: first run's sizes are not a setup.
fn rig_lights() -> LightsFile {
    LightsFile {
        schema_version: LIGHTS_SCHEMA,
        lights: vec![whole_zone(BOARD, 0), whole_zone(BOARD, 1), whole_zone(RAM, 0)],
        chains: Vec::new(),
    }
}

/// The hardware map of the four-header board: a whole-zone light per header, no chain record. This
/// is what first run and an OpenRGB size outside the flow leave behind (the "8 LEDs" state).
fn four_lights() -> LightsFile {
    LightsFile {
        schema_version: LIGHTS_SCHEMA,
        lights: (0..4).map(|zone| whole_zone(BOARD, zone)).collect(),
        chains: Vec::new(),
    }
}

/// One scene whose single group names every light. `Plays::Off` keeps the engine from building an
/// effect: what the daemon hands the engine still names the lights, which is what the empty-header
/// test reads.
fn rig_scenes(lights: &[Light]) -> ScenesFile {
    ScenesFile {
        schema_version: SCENES_SCHEMA,
        active: "everything".into(),
        scenes: vec![Scene {
            id: "everything".into(),
            name: "Everything".into(),
            rev: 1,
            groups: vec![Group {
                id: "all".into(),
                name: "Everything".into(),
                members: lights.iter().map(|light| light.id.clone()).collect(),
                plays: Plays::Off,
                mode: GroupMode::FlowAcross,
                brightness: 100,
            }],
        }],
    }
}

struct Rig {
    router: axum::Router,
    app: Arc<App>,
    engine: Arc<FakeEngine>,
    server: FakeOpenRgbServer,
    home: PathBuf,
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

/// A temp home of its own for one test.
fn temp_home(name: &str) -> PathBuf {
    let home =
        std::env::temp_dir().join(format!("canvas-rgb-chains-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp directory is created");
    home
}

/// An app whose adapter is the real OpenRGB adapter on the fake server, with the daemon's
/// supervisor running. With `saved`, `zones.json` holds first run's sizes (120 and 60), so the
/// supervisor re-applies them at connect; without it the file has no entry, as when first run never
/// sized the header.
async fn rig(name: &str, server: FakeOpenRgbServer, saved: bool) -> Rig {
    let home = temp_home(name);
    let mut store = Store::at(&home);
    // The first run is its own test; here the devices are placed the way it leaves them.
    store.config.first_run_done = true;
    store.lights = rig_lights();
    store.scenes = rig_scenes(&store.lights.lights);
    if saved {
        store.zones = ZonesFile {
            schema_version: canvas_proto::config::ZONES_SCHEMA,
            sizes: std::collections::BTreeMap::from([(
                BOARD.to_string(),
                std::collections::BTreeMap::from([(0, EARLIER), (1, SECOND_EARLIER)]),
            )]),
        };
        store.save_zones().expect("zones.json");
    }
    build_rig(home, store, server).await
}

/// A four-header rig with all four headers saved at 8 LEDs, as OpenRGB left them.
async fn rig_four(name: &str) -> Rig {
    let home = temp_home(name);
    let mut store = Store::at(&home);
    store.config.first_run_done = true;
    store.lights = four_lights();
    store.scenes = rig_scenes(&store.lights.lights);
    store.zones = ZonesFile {
        schema_version: canvas_proto::config::ZONES_SCHEMA,
        sizes: std::collections::BTreeMap::from([(
            BOARD.to_string(),
            (0..4).map(|zone| (zone, 8)).collect(),
        )]),
    };
    store.save_zones().expect("zones.json");
    let server = FakeOpenRgbServer::start(vec![board_four(), ram()]).await.expect("fake server");
    build_rig(home, store, server).await
}

/// A rig whose store is loaded from what the test wrote, as the daemon loads it at start: used for
/// a home a crash left behind. The first run is marked done, as it would be on a real one.
async fn rig_prepared(
    name: &str,
    server: FakeOpenRgbServer,
    prepare: impl FnOnce(&Path),
) -> Rig {
    let home = temp_home(name);
    prepare(&home);
    let mut store = Store::load_from(&home);
    store.config.first_run_done = true;
    build_rig(home, store, server).await
}

/// The app and its supervisor, over a store the caller built.
async fn build_rig(home: PathBuf, store: Store, server: FakeOpenRgbServer) -> Rig {
    let sizes = store.zones.sizes.clone();
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = App::new(
        store,
        engine.clone(),
        Box::new(OpenRgbAdapter::new("127.0.0.1", server.port())),
        TOKEN.into(),
        "0.0.0-test".into(),
        tx,
    );
    *app.zones.write().await = sizes;
    let app = Arc::new(app);
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let supervisor =
        tokio::spawn(canvas_daemon::device_sync::supervise_app(app.clone(), stop_rx));
    Rig { router: api::router(app.clone()), app, engine, server, home, stop, supervisor }
}

/// Waits for `check` to become true, or the deadline passes. Returns whether it became true.
async fn wait_until(within: Duration, mut check: impl FnMut() -> bool) -> bool {
    let end = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < end {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    check()
}

/// [`wait_until`] for a check that reads the app.
async fn wait_for<F, Fut>(within: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let end = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < end {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    check().await
}

/// Waits until the supervisor applied its first device list.
async fn connected(rig: &Rig) -> bool {
    wait_for(Duration::from_secs(60), || async {
        *rig.app.openrgb.read().await == canvas_proto::api::LinkState::Connected
            && !rig.app.devices.read().await.is_empty()
    })
    .await
}

/// Waits until a device has an engine frame sink, as the daemon gives every driven device.
async fn sink_of(rig: &Rig, id: &str) -> Option<Arc<dyn FrameSink>> {
    let wanted = DeviceId(id.into());
    let sink = std::sync::Arc::new(std::sync::Mutex::new(None));
    let found = {
        let slot = sink.clone();
        let engine = rig.engine.clone();
        wait_until(Duration::from_secs(60), move || {
            let found = engine.sinks.lock().unwrap().get(&wanted).cloned();
            let hit = found.is_some();
            *slot.lock().unwrap() = found;
            hit
        })
        .await
    };
    found.then(|| sink.lock().unwrap().clone()).flatten()
}

fn request(method: &str, path: &str, body: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"));
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("the request builds"),
        None => builder.body(Body::empty()).expect("the request builds"),
    }
}

async fn send(
    rig: &Rig,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let response = rig
        .router
        .clone()
        .oneshot(request(method, path, body))
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads");
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

fn setup_path(zone: u32) -> String {
    routes::CHAIN_SETUP.replace("{device}", BOARD).replace("{zone}", &zone.to_string())
}

fn light_path(zone: u32) -> String {
    routes::CHAIN_SETUP_LIGHT.replace("{device}", BOARD).replace("{zone}", &zone.to_string())
}

fn empty_path(zone: u32) -> String {
    routes::CHAIN_SETUP_EMPTY.replace("{device}", BOARD).replace("{zone}", &zone.to_string())
}

async fn chains(rig: &Rig) -> ChainsResponse {
    let (status, body) = send(rig, "GET", routes::CHAINS, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).expect("a chains answer")
}

/// Starts a setup on `zone` and returns what the route answered, parsed.
async fn start(rig: &Rig, zone: u32) -> (StatusCode, serde_json::Value) {
    send(rig, "POST", &setup_path(zone), None).await
}

async fn started(rig: &Rig, zone: u32) -> SetupStarted {
    let (status, body) = start(rig, zone).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).expect("a setup start")
}

/// Posts a light request with two runs: LEDs 0..100 red and 100..200 green.
async fn light(rig: &Rig, zone: u32, session: &str) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({
        "session": session,
        "runs": [
            { "start": 0, "count": 100, "color": "#ff0000" },
            { "start": 100, "count": 100, "color": "#00ff00" },
        ],
    });
    send(rig, "POST", &light_path(zone), Some(&body.to_string())).await
}

async fn cancel(rig: &Rig, zone: u32, session: &str) -> (StatusCode, serde_json::Value) {
    let body = SetupEmpty { session: session.into() };
    send(rig, "DELETE", &setup_path(zone), Some(&serde_json::to_string(&body).unwrap())).await
}

/// The chain record the store holds for a header.
async fn chain_state(rig: &Rig, zone: u32) -> Option<ChainState> {
    rig.app
        .store
        .read()
        .await
        .lights
        .chains
        .iter()
        .find(|record| record.device_id.0 == BOARD && record.zone_index == zone)
        .map(|record| record.state)
}

fn ids(view: &[LightView]) -> Vec<String> {
    view.iter().map(|part| part.light.id.clone()).collect()
}

fn red() -> Rgb8 {
    Rgb8::new(255, 0, 0)
}

fn green() -> Rgb8 {
    Rgb8::new(0, 255, 0)
}

/// `GET /api/v1/chains`: one view per resizable zone, and a header that first run sized (a light
/// with no chain record) reads as not set up, never as a known length.
#[tokio::test]
async fn list_marks_a_first_run_sized_header_as_not_set_up() {
    let rig = rig("list", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");

    let answer = chains(&rig).await;
    assert_eq!(answer.chains.len(), 2, "the two JARGB headers: {:?}", answer.chains);
    let first = &answer.chains[0];
    assert_eq!(first.device_id.0, BOARD);
    assert_eq!(first.device_name, "MSI MPG B850 Mystic Light");
    assert_eq!(first.zone_index, 0);
    assert_eq!(first.zone_name, "JARGB1");
    assert_eq!(first.state, ChainSetupState::NotSetUp, "a guessed length is not a setup");
    assert_eq!(first.leds, EARLIER);
    assert_eq!((first.min_leds, first.max_leds), (1, 300));
    assert_eq!(ids(&first.parts), vec!["openrgb:mb#z0"], "the whole-zone light is its one part");
    assert_eq!(first.parts[0].led_count, EARLIER);
    assert!(first.parts[0].connected);
    assert_eq!(answer.chains[1].zone_index, 1);
    assert_eq!(answer.chains[1].leds, SECOND_EARLIER);
    assert_eq!(answer.chains[1].state, ChainSetupState::NotSetUp);
    assert!(
        answer.chains.iter().all(|chain| chain.device_id.0 != RAM),
        "a zone that cannot be resized is not a chain"
    );
}

/// #470: walk every path the UI offers on a four-header board. The used header is answered with
/// parts (a fan and a strip); the other three are marked empty. `order` is the order the owner
/// answers them in, so the used header can be first or last. Each empty header is cancelled and
/// resumed once, as "Later" then "Set up" does. A completed flow always leaves one chain record
/// per header touched; only a cancel leaves none.
async fn walk_every_header(name: &str, order: &[u32]) {
    let rig = rig_four(name).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let used = 1;
    for &zone in order {
        if zone == used {
            let parts = vec![
                SetupPart {
                    name: "Fan".into(),
                    kind: ChainPartKind::Fan,
                    leds: 8,
                    copies: None,
                },
                SetupPart {
                    name: "Strip".into(),
                    kind: ChainPartKind::Strip,
                    leds: 8,
                    copies: None,
                },
            ];
            let setup = started(&rig, zone).await;
            assert!(
                wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, zone) == Some(300)).await,
                "the header was never raised"
            );
            let answer = committed(&rig, zone, &setup.session, &parts).await;
            assert_eq!(answer.final_leds, 16);
            assert_eq!(
                chain_state(&rig, zone).await,
                Some(ChainState::SetUp),
                "the used header is set up with its parts"
            );
        } else {
            // Cancel, then resume: neither a cancel nor a timeout writes a record, only the
            // completed empty call does.
            let first = started(&rig, zone).await;
            let (status, _) = cancel(&rig, zone, &first.session).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(chain_state(&rig, zone).await, None, "a cancelled setup writes no record");
            let setup = started(&rig, zone).await;
            let body = SetupEmpty { session: setup.session };
            let (status, body) = send(
                &rig,
                "POST",
                &empty_path(zone),
                Some(&serde_json::to_string(&body).unwrap()),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(
                chain_state(&rig, zone).await,
                Some(ChainState::Empty),
                "an emptied header has a record"
            );
        }
        let answered = order.iter().position(|z| *z == zone).unwrap() + 1;
        let records = { rig.app.store.read().await.lights.chains.len() };
        assert_eq!(records, answered, "one record per header answered so far");
        assert_eq!(
            chain_state(&rig, used).await.is_some(),
            order.iter().take(answered).any(|z| *z == used),
            "the used header's record appears exactly when it is answered"
        );
    }

    let answer = chains(&rig).await;
    assert_eq!(answer.chains.len(), 4, "one view per resizable header");
    for &zone in order {
        let view = answer.chains.iter().find(|chain| chain.zone_index == zone).expect("a header view");
        let expected = if zone == used { ChainSetupState::SetUp } else { ChainSetupState::Empty };
        assert_eq!(view.state, expected, "zone {zone}");
        if zone == used {
            assert_eq!(view.leds, 16, "the used header ends at its parts' sum");
            assert_eq!(view.parts.len(), 2, "the used header keeps its fan and strip parts");
        }
    }
}

#[tokio::test]
async fn every_completed_flow_writes_one_record_per_header_when_the_used_header_is_first() {
    walk_every_header("four-used-first", &[1, 0, 2, 3]).await;
}

#[tokio::test]
async fn every_completed_flow_writes_one_record_per_header_when_the_used_header_is_last() {
    walk_every_header("four-used-last", &[0, 2, 3, 1]).await;
}

/// Start writes the earlier length to `chain-restore.json` before the raise, raises the header to
/// its maximum once, and answers both lengths. The fake holds the raise's reply so the record's
/// ordering is judged at the moment the header is about to be raised, not against a timer.
#[tokio::test]
async fn start_raises_to_the_zone_maximum_once_and_writes_the_restore_record_first() {
    let rig = rig("start", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    // The connect applied the saved sizes; every later count is measured from here.
    let before = rig.server.resize_count(0);
    // Hold the fake's replies: the start's raise waits for a reply that never comes, so the daemon
    // is parked at the point where the record must already be on disk. The wait is on the server's
    // own held replies and resize count, a handshake with the raise (#411, #479), never a timer: a
    // delayed resize does not park the start, because the adapter re-enumerates the list at once
    // (fake.rs REQUEST_CONTROLLER_COUNT) and the delay only moves when the new size lands.
    let held_before = rig.server.held_replies();
    rig.server.hold_replies(true);
    let router = rig.router.clone();
    let path = setup_path(0);
    let pending =
        tokio::spawn(async move { router.oneshot(request("POST", &path, None)).await });
    assert!(
        wait_until(Duration::from_secs(30), || {
            rig.server.resize_count(0) > before && rig.server.held_replies() > held_before
        })
        .await,
        "the start never sent its raise to OpenRGB, so the record's ordering cannot be judged"
    );
    // The daemon is parked at the raise: the restore record is already on disk and holds the
    // earlier length. This is the ordering a crash in the middle of the raise depends on.
    assert!(
        rig.home.join(CHAIN_RESTORE_FILE).exists(),
        "the restore record was not written before the raise"
    );
    assert_eq!(
        read_restore(&rig.home).map(|record| record.leds),
        Some(EARLIER),
        "the record holds the earlier length"
    );

    // OpenRGB answers now: the start runs on to rediscover and its answer.
    rig.server.hold_replies(false);
    rig.server.release_replies();

    let response = tokio::time::timeout(Duration::from_secs(60), pending)
        .await
        .expect("the start answers")
        .expect("the request task ends")
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("the body reads");
    let answer: SetupStarted = serde_json::from_slice(&bytes).expect("a setup start");
    assert_eq!(answer.restore_to, EARLIER);
    assert_eq!(answer.raised_to, 300);
    assert_eq!(
        read_restore(&rig.home),
        Some(RestoreRecord {
            device_id: DeviceId(BOARD.into()),
            zone_index: 0,
            leds: EARLIER,
        })
    );

    assert!(
        wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await,
        "the header was never raised"
    );
    assert_eq!(
        rig.server.resize_count(0),
        before + 1,
        "one raise, and no second resize was sent"
    );
}

/// The raise is never saved: `zones.json` and the app's saved sizes keep the earlier length.
#[tokio::test]
async fn the_raise_is_never_written_to_zones_json() {
    let rig = rig("zones-file", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let answer = started(&rig, 0).await;
    assert_eq!(answer.raised_to, 300);
    assert!(
        wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await,
        "the header was never raised"
    );

    let saved: ZonesFile = serde_json::from_slice(
        &std::fs::read(rig.home.join(canvas_proto::config::ZONES_FILE)).expect("zones.json"),
    )
    .expect("zones.json parses");
    assert_eq!(
        saved.sizes.get(BOARD).and_then(|zones| zones.get(&0)),
        Some(&EARLIER),
        "the raise must not be saved"
    );
    assert_eq!(
        rig.app.zones.read().await.get(BOARD).and_then(|zones| zones.get(&0)),
        Some(&EARLIER),
        "the app's saved sizes keep the earlier length too"
    );
}

/// A light request lights its runs on the fake hardware, in their own colours, and every other
/// device stays black.
#[tokio::test]
async fn light_shows_the_runs_on_the_fake_hardware_and_blacks_every_other_device() {
    let rig = rig("light", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let answer = started(&rig, 0).await;
    assert!(
        wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await,
        "the header was never raised"
    );

    let (status, body) = light(&rig, 0, &answer.session).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let lit = wait_until(Duration::from_secs(30), || {
        let colors = rig.server.colors(0);
        colors.len() == 360 && colors[..100].iter().all(|c| *c == red())
    })
    .await;
    assert!(lit, "the runs never reached the fake hardware: {:?}", rig.server.colors(0));
    let colors = rig.server.colors(0);
    assert!(colors[100..200].iter().all(|c| *c == green()), "the second run's own colour");
    assert!(colors[200..].iter().all(|c| *c == Rgb8::BLACK), "nothing else of the board is lit");
    let ram = rig.server.colors(1);
    assert_eq!(ram.len(), 8);
    assert!(ram.iter().all(|c| *c == Rgb8::BLACK), "the other device is black: {ram:?}");
}

/// Dragging sends many light requests and no resize: the header is raised once, by the start.
#[tokio::test]
async fn dragging_causes_no_resize() {
    let rig = rig("drag", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let before = rig.server.resize_count(0);
    let answer = started(&rig, 0).await;
    assert!(
        wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await,
        "the header was never raised"
    );

    for start in 0..20u32 {
        let body = serde_json::json!({
            "session": answer.session,
            "runs": [{ "start": start * 5, "count": 5, "color": "#ff0000" }],
        });
        let (status, body) = send(&rig, "POST", &light_path(0), Some(&body.to_string())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    assert_eq!(
        rig.server.resize_count(0),
        before + 1,
        "the drag sent no resize: the header was raised once"
    );
    assert_eq!(rig.server.zone_leds(0, 0), Some(300));
}

/// Cancel puts the length back, drops the hold and the record, and hands the lights to the engine
/// again.
#[tokio::test]
async fn cancel_restores_the_length_and_the_lights_and_deletes_the_record() {
    let rig = rig("cancel", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let answer = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);

    let (status, _) = light(&rig, 0, &answer.session).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = cancel(&rig, 0, &answer.session).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rig.server.zone_leds(0, 0), Some(EARLIER), "the earlier length is back");
    assert!(!rig.home.join(CHAIN_RESTORE_FILE).exists(), "the record is deleted");
    assert!(
        rig.app.zone_holds.read().await.is_empty(),
        "the hold is dropped: {:?}",
        rig.app.zone_holds.read().await
    );
    assert!(rig.app.identify.setup_session().is_none());
    assert_eq!(chain_state(&rig, 0).await, None, "nothing was set up");
    let answer = chains(&rig).await;
    let first = answer.chains.iter().find(|chain| chain.zone_index == 0).expect("the first header");
    assert_eq!(first.leds, EARLIER);
    assert_eq!(first.parts[0].led_count, EARLIER);

    // The lights are back with the scene: an engine frame reaches the hardware again.
    let sink = sink_of(&rig, BOARD).await.expect("the board has a frame sink");
    sink.submit(vec![Rgb8::WHITE; 180]);
    assert!(
        wait_until(Duration::from_secs(30), || {
            let colors = rig.server.colors(0);
            colors.len() == 180 && colors.iter().all(|c| *c == Rgb8::WHITE)
        })
        .await,
        "the engine's frames never reached the lights again: {:?}",
        rig.server.colors(0)
    );
}

/// A setup that goes quiet for the spine's timeout puts the length back by itself.
#[tokio::test]
async fn the_timeout_restores_the_length_and_the_lights() {
    let rig = rig("timeout", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let answer = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let (status, _) = light(&rig, 0, &answer.session).await;
    assert_eq!(status, StatusCode::OK);

    // No further light request: the setup's own timeout (6 s after the last one) ends it.
    assert!(
        wait_until(Duration::from_secs(40), || rig.server.zone_leds(0, 0) == Some(EARLIER)).await,
        "the timeout never put the header's length back"
    );
    // The daemon deletes the record, drops the hold and ends the setup after it puts the
    // length back, so each is waited for, not read the moment the length is seen.
    assert!(
        wait_until(Duration::from_secs(30), || !rig.home.join(CHAIN_RESTORE_FILE).exists()).await,
        "the record is deleted"
    );
    assert!(
        wait_until(Duration::from_secs(30), || {
            rig.app.zone_holds.try_read().is_ok_and(|holds| holds.is_empty())
        })
        .await,
        "the hold is dropped"
    );
    assert!(
        wait_until(Duration::from_secs(30), || rig.app.identify.setup_session().is_none()).await,
        "the setup ended"
    );

    let sink = sink_of(&rig, BOARD).await.expect("the board has a frame sink");
    sink.submit(vec![Rgb8::WHITE; 180]);
    assert!(
        wait_until(Duration::from_secs(30), || {
            let colors = rig.server.colors(0);
            colors.len() == 180 && colors.iter().all(|c| *c == Rgb8::WHITE)
        })
        .await,
        "the engine's frames never reached the lights again: {:?}",
        rig.server.colors(0)
    );
}

/// One setup at a time: a second start, on another header, is a 409 and raises nothing.
#[tokio::test]
async fn a_second_session_for_another_zone_is_a_409() {
    let rig = rig("second-session", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let before = rig.server.resize_count(0);
    let first = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);

    let (status, body) = start(&rig, 1).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["message"].as_str().unwrap_or_default().contains("already open"),
        "the answer says a setup is open: {body}"
    );
    assert_eq!(
        rig.app.identify.setup_session().as_deref(),
        Some(first.session.as_str()),
        "the open setup keeps its session"
    );
    assert_eq!(
        rig.server.resize_count(0),
        before + 1,
        "the refused start raised nothing"
    );
    assert_eq!(rig.server.zone_leds(0, 1), Some(SECOND_EARLIER));
}

/// A session id the open setup does not answer to is a 409, for a light request and for a cancel,
/// and changes nothing.
#[tokio::test]
async fn a_stale_session_is_a_409() {
    let rig = rig("stale", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let answer = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);

    let (status, body) = light(&rig, 0, "stale").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = cancel(&rig, 0, "stale").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        rig.app.identify.setup_session().as_deref(),
        Some(answer.session.as_str()),
        "the open setup is untouched"
    );
    assert!(rig.home.join(CHAIN_RESTORE_FILE).exists(), "the record is untouched");
    assert_eq!(rig.server.zone_leds(0, 0), Some(300), "the header is still raised");
}

/// A raise the device refuses leaves the header where it was, with no record and no hold, and the
/// answer carries the device's own error.
#[tokio::test]
async fn a_failed_raise_leaves_the_header_and_no_record() {
    let rig = rig("failed-raise", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    rig.server.fail_resizes(true);

    let (status, body) = start(&rig, 0).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let message = body["message"].as_str().unwrap_or_default();
    assert!(message.contains("raise"), "the answer names what failed: {body}");
    assert!(
        message.contains("no reply"),
        "the answer carries the device's own error, not a sentence of its own: {body}"
    );
    assert_eq!(rig.server.zone_leds(0, 0), Some(EARLIER), "the header is where it was");
    assert!(!rig.home.join(CHAIN_RESTORE_FILE).exists(), "no record is left behind");
    assert!(rig.app.zone_holds.read().await.is_empty(), "no hold is left behind");
    assert!(rig.app.identify.setup_session().is_none(), "no session is left behind");

    // The gate must not be left counting a ticker that never ran: a later identify has to be
    // told to start one, or it would never light anything.
    let board = DeviceId(BOARD.into());
    assert!(
        wait_until(Duration::from_secs(5), || {
            let needs_ticker = rig.app.identify.start(board.clone(), 0, 0, 1, std::time::Instant::now());
            rig.app.identify.stop();
            needs_ticker
        })
        .await,
        "after a failed start the gate still believes a ticker runs: identify would stay dark"
    );
}

/// With no readable restore record, ending the setup still drops the hold, so the supervisor can
/// size the header again.
#[tokio::test]
async fn an_unreadable_record_still_drops_the_hold_when_the_setup_ends() {
    let rig = rig("unreadable-end", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let session = started(&rig, 0).await.session;
    assert!(!rig.app.zone_holds.read().await.is_empty(), "the setup holds the header");
    std::fs::write(rig.home.join(CHAIN_RESTORE_FILE), b"{ not json").expect("damage the record");

    let (status, body) = cancel(&rig, 0, &session).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(rig.app.zone_holds.read().await.is_empty(), "no hold outlives the setup");
    assert!(rig.home.join(CHAIN_RESTORE_FILE).exists(), "an unreadable record is left on disk");
}

/// Review Focus 2: a crash mid-setup leaves the record and a raised header; the first device list
/// after a restart puts the length back and deletes the record.
#[tokio::test]
async fn start_up_replays_a_leftover_restore_record() {
    // The crash left the header raised and no saved size (first run never sized it).
    let rig = rig("replay", fake(300).await, false).await;
    canvas_daemon::chain_records::write_restore(
        &rig.home,
        &RestoreRecord { device_id: DeviceId(BOARD.into()), zone_index: 0, leds: EARLIER },
    )
    .expect("the restore record");

    // The supervisor connects inside `rig`; the replay runs on the first device list.
    assert!(
        wait_until(Duration::from_secs(60), || rig.server.zone_leds(0, 0) == Some(EARLIER)).await,
        "start-up never put the header's length back: {:?}",
        rig.server.zone_leds(0, 0)
    );
    // The record goes after the resize answers, so it is waited for, not sampled.
    assert!(
        wait_until(Duration::from_secs(30), || !rig.home.join(CHAIN_RESTORE_FILE).exists()).await,
        "the record is deleted"
    );
    assert!(
        wait_for(Duration::from_secs(30), || async {
            rig.app
                .devices
                .read()
                .await
                .iter()
                .find(|device| device.id.0 == BOARD)
                .is_some_and(|device| device.zones[0].led_count == EARLIER)
        })
        .await,
        "the daemon's device list still carries the raised length"
    );
    assert!(connected(&rig).await);
    let answer = chains(&rig).await;
    assert_eq!(answer.chains[0].leds, EARLIER);
}

/// An unreadable record is the only copy of what a crash interrupted: it is left exactly as it is,
/// and the daemon says so and carries on.
#[tokio::test]
async fn start_up_leaves_an_unreadable_record_alone_and_says_so() {
    let logs = common::global_logs();
    let server = fake(300).await;
    let rig = rig("unreadable", server, false).await;
    let path = rig.home.join(CHAIN_RESTORE_FILE);
    let wrecked = b"{ this is not a restore record";
    std::fs::write(&path, wrecked).expect("the wrecked record");

    assert!(connected(&rig).await, "the daemon never applied a device list");
    // The replay runs before the sinks are given out, so a sink means it has been reached.
    assert!(sink_of(&rig, BOARD).await.is_some(), "the daemon never reached the replay");
    assert_eq!(std::fs::read(&path).expect("the record is still there"), wrecked);
    let said = wait_until(Duration::from_secs(10), || {
        logs.text().contains("chain-restore.json did not parse")
    })
    .await;
    assert!(said, "the daemon did not say the record could not be read: {}", logs.text());
    assert_eq!(rig.server.zone_leds(0, 0), Some(300), "nothing is treated as held");
}

/// An empty header: the earlier length comes back, the zone stops being a light, and the scene no
/// longer names it, so the engine drives it black.
#[tokio::test]
async fn an_empty_header_goes_back_to_its_length_is_not_a_light_and_is_driven_black() {
    let rig = rig("empty", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let before = rig.server.resize_count(0);
    let answer = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let (status, _) = light(&rig, 0, &answer.session).await;
    assert_eq!(status, StatusCode::OK);

    let body = SetupEmpty { session: answer.session.clone() };
    let (status, body) = send(
        &rig,
        "POST",
        &empty_path(0),
        Some(&serde_json::to_string(&body).unwrap()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rig.server.zone_leds(0, 0), Some(EARLIER), "the earlier length is back");
    assert_eq!(rig.server.resize_count(0), before + 2, "one raise, one put back, nothing more");
    assert!(!rig.home.join(CHAIN_RESTORE_FILE).exists(), "the record is deleted");
    assert_eq!(chain_state(&rig, 0).await, Some(ChainState::Empty));
    let lights = rig.app.store.read().await.lights.clone();
    assert!(
        lights.lights.iter().all(|light| light.id != "openrgb:mb#z0"),
        "an empty header is not a light: {:?}",
        lights.lights.iter().map(|light| light.id.clone()).collect::<Vec<_>>()
    );
    assert!(
        lights.lights.iter().any(|light| light.id == "openrgb:mb#z1"),
        "the other header keeps its light"
    );
    let answer = chains(&rig).await;
    let first = answer.chains.iter().find(|chain| chain.zone_index == 0).expect("the first header");
    assert_eq!(first.state, ChainSetupState::Empty);
    assert!(first.parts.is_empty(), "no light, no parts: {:?}", first.parts);

    // Driven black: the scene handed to the engine names no light of this header, so no group
    // paints it.
    let scene_lights = rig.engine.scene_lights.lock().unwrap().clone();
    assert!(
        scene_lights.iter().all(|light| light.id != "openrgb:mb#z0"),
        "the engine still has a light for the empty header"
    );
    let members: Vec<String> =
        rig.engine.scene_members.lock().unwrap().iter().flatten().cloned().collect();
    assert!(
        !members.iter().any(|member| member == "openrgb:mb#z0"),
        "a group still names the empty header: {members:?}"
    );
}

/// Review Focus 1, through the API: a rescan while a setup holds the raise leaves the header
/// raised.
#[tokio::test]
async fn a_rescan_during_a_session_does_not_resize_the_header_back() {
    let rig = rig("rescan", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let before = rig.server.resize_count(0);
    let answer = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let (status, _) = light(&rig, 0, &answer.session).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(&rig, "GET", "/api/v1/devices?rescan=1", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rig.server.zone_leds(0, 0), Some(300), "the rescan must leave the raise alone");
    assert_eq!(
        rig.server.resize_count(0),
        before + 1,
        "the rescan resized nothing: the hold kept the raise"
    );
    assert_eq!(
        rig.app.identify.setup_session().as_deref(),
        Some(answer.session.as_str()),
        "the setup is still open"
    );

    // The header is still the setup's: a light request still lands.
    let (status, _) = light(&rig, 0, &answer.session).await;
    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------------------------------------
// Task 7b: the commit, all or nothing
// ---------------------------------------------------------------------------------------------

/// What the seven parts of the board's first header add up to.
const FINAL: u32 = 96;

fn commit_path(zone: u32) -> String {
    routes::CHAIN_SETUP_COMMIT.replace("{device}", BOARD).replace("{zone}", &zone.to_string())
}

fn part_id(n: u32) -> String {
    part_light_id(&DeviceId(BOARD.into()), 0, n)
}

fn part_ids(count: u32) -> Vec<String> {
    (1..=count).map(part_id).collect()
}

/// The board's first header set up as seven fans: an even split of the 96 LEDs the flow shows.
fn seven_parts() -> Vec<SetupPart> {
    [14, 14, 14, 14, 14, 13, 13]
        .iter()
        .enumerate()
        .map(|(index, leds)| SetupPart {
            name: format!("Fan {}", index + 1),
            kind: ChainPartKind::Fan,
            leds: *leds,
            copies: None,
        })
        .collect()
}

/// The same 96 LEDs set up again as six parts: the first six parts keep their place.
fn six_parts() -> Vec<SetupPart> {
    (1..=6)
        .map(|index| SetupPart {
            name: format!("Fan {index}"),
            kind: ChainPartKind::Fan,
            leds: FINAL / 6,
            copies: None,
        })
        .collect()
}

async fn commit(
    rig: &Rig,
    zone: u32,
    session: &str,
    parts: &[SetupPart],
) -> (StatusCode, serde_json::Value) {
    let body = SetupCommit { session: session.into(), parts: parts.to_vec() };
    send(rig, "POST", &commit_path(zone), Some(&serde_json::to_string(&body).expect("a commit body")))
        .await
}

async fn committed(rig: &Rig, zone: u32, session: &str, parts: &[SetupPart]) -> SetupCommitted {
    let (status, body) = commit(rig, zone, session, parts).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).expect("a commit answer")
}

fn read_lights(home: &Path) -> LightsFile {
    let bytes = std::fs::read(home.join(LIGHTS_FILE)).expect("lights.json");
    serde_json::from_slice(&bytes).expect("lights.json parses")
}

fn read_scenes(home: &Path) -> ScenesFile {
    let bytes = std::fs::read(home.join(SCENES_FILE)).expect("scenes.json");
    serde_json::from_slice(&bytes).expect("scenes.json parses")
}

fn read_zones(home: &Path) -> ZonesFile {
    let bytes = std::fs::read(home.join(ZONES_FILE)).expect("zones.json");
    serde_json::from_slice(&bytes).expect("zones.json parses")
}

/// Writes the lights and scenes the rig was built with, so a failed commit has files to leave as
/// they were.
async fn save_rig_files(rig: &Rig) {
    let store = rig.app.store.read().await;
    store.save_lights().expect("lights.json");
    store.save_scenes().expect("scenes.json");
}

/// A header's lights as the file holds them, in chain order.
fn zone_parts(lights: &LightsFile, zone: u32) -> Vec<Light> {
    let mut parts: Vec<Light> = lights
        .lights
        .iter()
        .filter(|light| light.device_id.0 == BOARD && light.zone_index == zone)
        .cloned()
        .collect();
    parts.sort_by_key(|part| part.leds.map_or(0, |slice| slice.start));
    parts
}

/// The lists and groups a commit made: one `run` per part, 1-based.
fn run_of(index: u32) -> LedSlice {
    let start = [0, 14, 28, 42, 56, 70, 83][index as usize - 1];
    let count = [14, 14, 14, 14, 14, 13, 13][index as usize - 1];
    LedSlice { start, count }
}

/// Review Focus 3, the happy path: one raise at the start and one resize at the commit, and no
/// other resize over the whole setup.
#[tokio::test]
async fn commit_resizes_exactly_twice_over_a_whole_setup() {
    let rig = rig("commit-twice", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let before = rig.server.resize_count(0);
    let setup = started(&rig, 0).await;
    assert!(
        wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await,
        "the header was never raised"
    );

    let answer = committed(&rig, 0, &setup.session, &seven_parts()).await;

    assert_eq!(answer.final_leds, FINAL);
    assert_eq!(
        rig.server.resize_count(0),
        before + 2,
        "one raise and one final length, and nothing else"
    );
    assert_eq!(rig.server.zone_leds(0, 0), Some(FINAL), "the header ends at the parts' sum");
}

/// The commit writes the three files together: the parts, the groups that hold them in chain
/// order, and the header's length in `zones.json`.
#[tokio::test]
async fn commit_writes_parts_groups_and_zone_size_together() {
    let rig = rig("commit-files", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);

    let answer = committed(&rig, 0, &setup.session, &seven_parts()).await;

    let lights = read_lights(&rig.home);
    let made = zone_parts(&lights, 0);
    assert_eq!(made.iter().map(|part| part.id.clone()).collect::<Vec<_>>(), part_ids(7));
    for (index, part) in seven_parts().iter().enumerate() {
        assert_eq!(made[index].name.as_deref(), Some(part.name.as_str()), "part {}", index + 1);
        assert_eq!(made[index].kind, Some(part.kind), "part {}", index + 1);
        assert_eq!(made[index].leds, Some(run_of(index as u32 + 1)), "part {}", index + 1);
        assert_eq!(made[index].arrangement, Arrangement::Ring, "a fan is a ring");
    }
    let record = lights
        .chains
        .iter()
        .find(|record| record.device_id.0 == BOARD && record.zone_index == 0)
        .expect("the chain record is written");
    assert_eq!(record.state, ChainState::SetUp);

    let scenes = read_scenes(&rig.home);
    let mut expected = part_ids(7);
    expected.push("openrgb:mb#z1".into());
    expected.push("openrgb:ram#z0".into());
    assert_eq!(
        scenes.scenes[0].groups[0].members, expected,
        "the parts take the whole-zone light's place in the group, in chain order"
    );

    let zones = read_zones(&rig.home);
    assert_eq!(zones.sizes[BOARD][&0], FINAL, "zones.json holds the parts' sum");
    assert_eq!(zones.sizes[BOARD][&1], SECOND_EARLIER, "the other header keeps its length");
    assert_eq!(
        rig.app.zones.read().await.get(BOARD).and_then(|sizes| sizes.get(&0)),
        Some(&FINAL),
        "the sizes the supervisor reads hold the final length too"
    );

    assert_eq!(answer.lights.len(), 7);
    assert_eq!(
        answer.lights.iter().map(|part| part.light.id.clone()).collect::<Vec<_>>(),
        part_ids(7),
        "the answer names the parts in chain order"
    );
    assert_eq!(answer.lights[0].led_count, 14);
    assert_eq!(answer.lights[6].led_count, 13);
}

/// The setup ends with the commit: the hold and the session go, and the scene plays on the new
/// parts again.
#[tokio::test]
async fn commit_ends_the_hold_and_the_scene_plays_again() {
    let rig = rig("commit-ends", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);

    committed(&rig, 0, &setup.session, &seven_parts()).await;

    assert!(rig.app.zone_holds.read().await.is_empty(), "the hold is dropped");
    assert!(rig.app.identify.setup_session().is_none(), "the session ended");
    assert!(!rig.home.join(CHAIN_RESTORE_FILE).exists(), "the restore record is deleted");
    assert!(!rig.home.join(CHAIN_INTENT_FILE).exists(), "the intent record is deleted");

    // The scene was handed the new parts, so the engine paints them from now on.
    let scene_lights = rig.engine.scene_lights.lock().unwrap().clone();
    let ids: Vec<String> = scene_lights.iter().map(|light| light.id.clone()).collect();
    for id in part_ids(7) {
        assert!(ids.contains(&id), "the engine's scene does not name {id}: {ids:?}");
    }

    // The gate is gone: an engine frame reaches the header again. The board now has 96 + 60 LEDs.
    let sink = sink_of(&rig, BOARD).await.expect("the board has a frame sink");
    sink.submit(vec![Rgb8::WHITE; 156]);
    assert!(
        wait_until(Duration::from_secs(30), || {
            let colors = rig.server.colors(0);
            colors.len() == 156 && colors.iter().all(|color| *color == Rgb8::WHITE)
        })
        .await,
        "the engine's frames never reached the lights again: {:?}",
        rig.server.colors(0)
    );
}

/// Review Focus 3: a resize the device refuses leaves everything as it was, and the session open
/// so the same commit can be tried again. The refusal outlasts the setup's idle limit (#429):
/// a commit that waits on a slow device keeps its session until it answers.
#[tokio::test]
async fn a_failed_resize_changes_nothing_and_keeps_the_session_for_a_retry() {
    let rig = rig("commit-resize-fails", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    save_rig_files(&rig).await;
    let lights_before = std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json");
    let scenes_before = std::fs::read(rig.home.join(SCENES_FILE)).expect("scenes.json");
    let zones_before = std::fs::read(rig.home.join(ZONES_FILE)).expect("zones.json");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    rig.server.fail_resizes(true);
    // #429: a refused resize is five seconds of silence, most of the setup's six-second idle
    // limit between light requests. A loaded runner pushed the rest past it, and the session
    // ended under the commit. The limit is shortened here so the commit outlasts it by ten
    // times on any machine: the test stops reading a verdict off how fast the machine is.
    rig.app.identify.set_setup_timeout(Duration::from_millis(500));

    let measured = std::time::Instant::now();
    let (status, body) = commit(&rig, 0, &setup.session, &seven_parts()).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    // The commit outlasted the limit the rig gave the setup: that is what used to end the
    // session under it, so a fast refusal must not quietly make this test prove nothing.
    assert!(
        measured.elapsed() > Duration::from_millis(500),
        "the refused commit took {:?}, under the setup's shortened limit",
        measured.elapsed()
    );
    let message = body["message"].as_str().unwrap_or_default();
    assert!(message.contains("no reply"), "the answer carries the device's own error: {body}");
    assert!(!rig.home.join(CHAIN_INTENT_FILE).exists(), "the intent is deleted: nothing was written");
    assert!(rig.home.join(CHAIN_RESTORE_FILE).exists(), "the restore record stays for the session");
    assert_eq!(
        rig.app.identify.setup_session().as_deref(),
        Some(setup.session.as_str()),
        "the session stays open for a retry"
    );
    assert!(!rig.app.zone_holds.read().await.is_empty(), "the header is still the setup's");
    assert_eq!(rig.server.zone_leds(0, 0), Some(300), "the failed resize changed no length");
    assert_eq!(std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json"), lights_before);
    assert_eq!(std::fs::read(rig.home.join(SCENES_FILE)).expect("scenes.json"), scenes_before);
    assert_eq!(std::fs::read(rig.home.join(ZONES_FILE)).expect("zones.json"), zones_before);
    assert_eq!(chain_state(&rig, 0).await, None, "no chain record was written");
    assert_eq!(zone_parts(&read_lights(&rig.home), 0).len(), 1, "the whole-zone light is still it");

    // The daemon's own limit again: the checks above are not requests, and the retry is one
    // request that ends the setup.
    rig.app.identify.set_setup_timeout(canvas_daemon::identify::SETUP_TIMEOUT);
    // The retry the open session allows: the same commit lands.
    rig.server.fail_resizes(false);
    let answer = committed(&rig, 0, &setup.session, &seven_parts()).await;
    assert_eq!(answer.final_leds, FINAL);
    assert_eq!(rig.server.zone_leds(0, 0), Some(FINAL));
    assert_eq!(chain_state(&rig, 0).await, Some(ChainState::SetUp));
}

/// Review Focus 3: a store write that fails after the resize puts the files back and the header
/// back to the raised length the setup holds, and the session stays open for a retry or a cancel.
#[tokio::test]
async fn a_failed_store_write_resizes_back_and_leaves_the_files() {
    let rig = rig("commit-store-fails", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    save_rig_files(&rig).await;
    let lights_before = std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json");
    let scenes_before = std::fs::read(rig.home.join(SCENES_FILE)).expect("scenes.json");
    let zones_before = std::fs::read(rig.home.join(ZONES_FILE)).expect("zones.json");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    // The store refuses its writes, as it does when a file could not be read at start.
    rig.app.store.write().await.lights_blocked = true;

    let (status, body) = commit(&rig, 0, &setup.session, &seven_parts()).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let message = body["message"].as_str().unwrap_or_default();
    assert!(message.contains("lights.json"), "the answer names the write that failed: {body}");
    assert_eq!(std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json"), lights_before);
    assert_eq!(std::fs::read(rig.home.join(SCENES_FILE)).expect("scenes.json"), scenes_before);
    assert_eq!(std::fs::read(rig.home.join(ZONES_FILE)).expect("zones.json"), zones_before);
    assert_eq!(rig.server.zone_leds(0, 0), Some(300), "the header is back at the raised length");
    assert!(!rig.home.join(CHAIN_INTENT_FILE).exists(), "the intent is deleted");
    assert!(rig.home.join(CHAIN_RESTORE_FILE).exists(), "the restore record stays for the session");
    assert_eq!(
        rig.app.identify.setup_session().as_deref(),
        Some(setup.session.as_str()),
        "the session stays open for a retry"
    );
    assert_eq!(
        rig.app.zone_holds.read().await.get(&(BOARD.to_string(), 0)).copied(),
        Some(300),
        "the setup still holds the header raised"
    );
    let lights = rig.app.store.read().await.lights.clone();
    assert_eq!(lights.chains, Vec::new(), "memory is put back: no chain record");
    assert_eq!(zone_parts(&lights, 0).len(), 1, "memory is put back: the whole-zone light");
    assert_eq!(lights, read_lights(&rig.home), "memory and the file agree");

    // The retry: with the store able to write, the same commit lands.
    rig.app.store.write().await.lights_blocked = false;
    let answer = committed(&rig, 0, &setup.session, &seven_parts()).await;
    assert_eq!(answer.final_leds, FINAL);
    assert_eq!(rig.server.zone_leds(0, 0), Some(FINAL));
    assert_eq!(zone_parts(&read_lights(&rig.home), 0).len(), 7);
}

/// When the rollback itself fails, both records stay for the next start to replay; nothing here
/// pretends the rollback happened.
#[tokio::test]
async fn a_failed_rollback_keeps_both_records() {
    let rig = rig("commit-rollback-fails", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    save_rig_files(&rig).await;
    let lights_before = std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json");
    let scenes_before = std::fs::read(rig.home.join(SCENES_FILE)).expect("scenes.json");
    let zones_before = std::fs::read(rig.home.join(ZONES_FILE)).expect("zones.json");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    rig.app.store.write().await.lights_blocked = true;
    // The fake keeps its replies back, so the commit is parked before its store write and its
    // rollback: the snapshot the commit names can be damaged inside that gap. A snapshot that
    // cannot be read is a rollback that cannot run. The wait is on the server's own held replies,
    // a handshake with the commit, never a timer (#411, #479): a delayed resize does not park the
    // commit, because the adapter re-enumerates the list at once (fake.rs REQUEST_CONTROLLER_COUNT)
    // and the delay only moves when the new size lands.
    // Read before the hold starts, so the first held reply is the one this wait is for.
    let held_before = rig.server.held_replies();
    rig.server.hold_replies(true);
    let router = rig.router.clone();
    let path = commit_path(0);
    let body = serde_json::to_string(&SetupCommit {
        session: setup.session.clone(),
        parts: seven_parts(),
    })
    .expect("a commit body");
    let pending = tokio::spawn(async move { router.oneshot(request("POST", &path, Some(&body))).await });

    let named = wait_until(Duration::from_secs(30), || rig.home.join(CHAIN_INTENT_FILE).exists()).await;
    assert!(named, "the commit never wrote its intent");
    assert!(
        wait_until(Duration::from_secs(30), || rig.server.held_replies() > held_before).await,
        "the commit never waited on the device, so it cannot be held between its resize and its write"
    );
    let intent = read_intent(&rig.home).expect("the intent record");
    let folder = canvas_daemon::paths::backups_dir_in(&rig.home).join(&intent.snapshot);
    assert!(folder.join(LIGHTS_FILE).exists(), "the snapshot holds the lights it would put back");
    std::fs::remove_file(folder.join(LIGHTS_FILE)).expect("the snapshot's lights.json goes");
    std::fs::create_dir(folder.join(LIGHTS_FILE)).expect("a folder takes its place");

    // OpenRGB answers now: the commit runs on to its refused store write and its rollback, which
    // cannot read the damaged snapshot.
    rig.server.hold_replies(false);
    rig.server.release_replies();

    let response = tokio::time::timeout(Duration::from_secs(60), pending)
        .await
        .expect("the commit answers")
        .expect("the request task ends")
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(rig.home.join(CHAIN_INTENT_FILE).exists(), "the intent stays: the rollback did not run");
    assert!(rig.home.join(CHAIN_RESTORE_FILE).exists(), "the restore record stays too");
    // The store's own rollback still put memory and the two files it had changed back, so the
    // owner's files are as they were even though the snapshot could not be read.
    assert_eq!(std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json"), lights_before);
    assert_eq!(std::fs::read(rig.home.join(SCENES_FILE)).expect("scenes.json"), scenes_before);
    assert_eq!(std::fs::read(rig.home.join(ZONES_FILE)).expect("zones.json"), zones_before);
    assert_eq!(rig.app.store.read().await.lights.chains, Vec::new(), "no chain record was kept");
    assert!(
        !rig.app.zone_holds.read().await.is_empty(),
        "the unfinished setup still holds its header"
    );
    assert!(
        wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await,
        "the setup's hold no longer keeps the header: {:?}",
        rig.server.zone_leds(0, 0)
    );

    // The owner cancels once the snapshot can be read again: the unfinished commit is undone now,
    // and no intent is left for a later start to replay over edits made in between.
    std::fs::remove_dir(folder.join(LIGHTS_FILE)).expect("the folder goes");
    std::fs::write(folder.join(LIGHTS_FILE), &lights_before).expect("the snapshot's lights.json is back");
    let (status, body) = cancel(&rig, 0, &setup.session).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!rig.home.join(CHAIN_INTENT_FILE).exists(), "the cancel finished the intent");
    assert!(!rig.home.join(CHAIN_RESTORE_FILE).exists(), "and the restore record");
    assert!(
        wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(EARLIER)).await,
        "the header is not at its earlier length: {:?}",
        rig.server.zone_leds(0, 0)
    );
    // Each is waited for, not read the moment the length is seen (#411).
    assert!(
        wait_until(Duration::from_secs(30), || {
            rig.app.zone_holds.try_read().is_ok_and(|holds| holds.is_empty())
        })
        .await,
        "nothing is held"
    );
    assert!(
        wait_until(Duration::from_secs(30), || rig.app.identify.setup_session().is_none()).await,
        "the setup is over"
    );
}

/// Review Focus 3: a daemon start that finds an intent record undoes the interrupted commit: the
/// files come back from the snapshot, the store reloads them, and the header returns to its
/// earlier length.
#[tokio::test]
async fn start_up_rolls_back_a_leftover_intent() {
    let device = DeviceId(BOARD.into());
    let before_lights = rig_lights();
    let before_scenes = rig_scenes(&before_lights.lights);
    let before_zones = ZonesFile {
        schema_version: canvas_proto::config::ZONES_SCHEMA,
        sizes: std::collections::BTreeMap::from([(
            BOARD.to_string(),
            std::collections::BTreeMap::from([(0, EARLIER), (1, SECOND_EARLIER)]),
        )]),
    };
    let plan = plan_commit(&before_lights, &before_scenes, &device, 0, &seven_parts(), 300)
        .expect("the plan fits");
    let final_leds = plan.final_leds;
    let written_lights = before_lights.clone();
    let written_scenes = before_scenes.clone();
    let written_zones = before_zones.clone();
    let prepare = move |home: &Path| {
        // The files as they were before the interrupted commit.
        std::fs::write(home.join(LIGHTS_FILE), serde_json::to_vec_pretty(&written_lights).unwrap())
            .unwrap();
        std::fs::write(home.join(SCENES_FILE), serde_json::to_vec_pretty(&written_scenes).unwrap())
            .unwrap();
        std::fs::write(home.join(ZONES_FILE), serde_json::to_vec_pretty(&written_zones).unwrap())
            .unwrap();
        let snapshot = snapshot(home, "before-chain-setup").expect("the commit's snapshot");
        let name = snapshot.file_name().unwrap().to_str().unwrap().to_string();
        // The commit got as far as the resize and the three writes, then the daemon died.
        std::fs::write(home.join(LIGHTS_FILE), serde_json::to_vec_pretty(&plan.lights).unwrap())
            .unwrap();
        std::fs::write(home.join(SCENES_FILE), serde_json::to_vec_pretty(&plan.scenes).unwrap())
            .unwrap();
        let after_zones = ZonesFile {
            schema_version: canvas_proto::config::ZONES_SCHEMA,
            sizes: std::collections::BTreeMap::from([(
                BOARD.to_string(),
                std::collections::BTreeMap::from([(0, final_leds), (1, SECOND_EARLIER)]),
            )]),
        };
        std::fs::write(home.join(ZONES_FILE), serde_json::to_vec_pretty(&after_zones).unwrap())
            .unwrap();
        write_intent(
            home,
            &IntentRecord {
                device_id: DeviceId(BOARD.into()),
                zone_index: 0,
                restore_leds: EARLIER,
                final_leds,
                snapshot: name,
            },
        )
        .expect("the intent record");
        write_restore(
            home,
            &RestoreRecord { device_id: DeviceId(BOARD.into()), zone_index: 0, leds: EARLIER },
        )
        .expect("the restore record");
    };
    // The header is at the length the interrupted commit set, with no hold: a restart has none.
    let rig = rig_prepared("leftover-intent", fake(final_leds).await, prepare).await;

    assert!(
        wait_until(Duration::from_secs(60), || rig.server.zone_leds(0, 0) == Some(EARLIER)).await,
        "start-up never put the header back: {:?}",
        rig.server.zone_leds(0, 0)
    );
    // Each is waited for, not read the moment the length is seen (#411).
    assert!(
        wait_until(Duration::from_secs(30), || !rig.home.join(CHAIN_INTENT_FILE).exists()).await,
        "the intent is deleted"
    );
    assert!(
        wait_until(Duration::from_secs(30), || !rig.home.join(CHAIN_RESTORE_FILE).exists()).await,
        "the restore record is deleted"
    );
    assert_eq!(zone_parts(&read_lights(&rig.home), 0).len(), 1, "lights.json is the earlier one");
    assert_eq!(read_lights(&rig.home), before_lights);
    assert_eq!(read_scenes(&rig.home), before_scenes);
    assert_eq!(read_zones(&rig.home), before_zones);
    assert!(
        wait_for(Duration::from_secs(30), || async {
            let store = rig.app.store.read().await;
            store.lights == before_lights && store.scenes == before_scenes
        })
        .await,
        "the store did not reload the files the rollback put back"
    );
    assert_eq!(
        rig.app.zones.read().await.get(BOARD).and_then(|sizes| sizes.get(&0)),
        Some(&EARLIER),
        "the supervisor's sizes are the restored ones"
    );
    assert_eq!(chain_state(&rig, 0).await, None, "the chain is not set up any more");
    // The device list catches up with the header after the rollback's own re-read; a list the
    // supervisor read before it can land in between, so the answer is waited for, not sampled.
    assert!(
        wait_for(Duration::from_secs(30), || async { chains(&rig).await.chains[0].leds == EARLIER }).await,
        "the device list still carries the interrupted commit's length"
    );
    assert_eq!(chains(&rig).await.chains[0].state, ChainSetupState::NotSetUp);
}

/// Review Focus 4 in the files: setting the same chain up again with fewer parts keeps the ids by
/// position and drops the vanished part from every group.
#[tokio::test]
async fn setting_up_again_keeps_part_identity_in_the_files() {
    let rig = rig("commit-again", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let first = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    committed(&rig, 0, &first.session, &seven_parts()).await;
    assert_eq!(part_ids(7), zone_parts(&read_lights(&rig.home), 0).iter().map(|p| p.id.clone()).collect::<Vec<_>>());

    let second = started(&rig, 0).await;
    assert_eq!(second.restore_to, FINAL, "the re-setup starts from the length it has");
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    committed(&rig, 0, &second.session, &six_parts()).await;

    let lights = read_lights(&rig.home);
    let made = zone_parts(&lights, 0);
    assert_eq!(made.len(), 6);
    assert_eq!(
        made.iter().map(|part| part.id.clone()).collect::<Vec<_>>(),
        part_ids(6),
        "the parts keep their ids by position"
    );
    assert_eq!(made.iter().map(|part| part.name.clone().unwrap()).collect::<Vec<_>>(),
        (1..=6).map(|n| format!("Fan {n}")).collect::<Vec<_>>());
    let scenes = read_scenes(&rig.home);
    let members = &scenes.scenes[0].groups[0].members;
    assert!(!members.contains(&part_id(7)), "the vanished part left every group: {members:?}");
    let mut expected = part_ids(6);
    expected.push("openrgb:mb#z1".into());
    expected.push("openrgb:ram#z0".into());
    assert_eq!(*members, expected, "the first six keep their place in the group");
    assert_eq!(read_zones(&rig.home).sizes[BOARD][&0], FINAL);
}

// ---------------------------------------------------------------------------------------------
// #381, #382: a header whose devices mirror each other, and the two-run session the counting uses
// ---------------------------------------------------------------------------------------------

/// Luke's JARGB 1: seven fans and a strip on a splitter, every one showing the same 8 LEDs. The
/// owner counts 8 orange lights and 8 LEDs on one device, so the flow sends one row.
fn mirror_row(copies: Option<u32>) -> SetupPart {
    SetupPart {
        name: "Same lights, shown 8 times".into(),
        kind: ChainPartKind::Other,
        leds: 8,
        copies,
    }
}

/// A mirrored header ends as ONE light of 8 LEDs with `copies` 8, and the header is resized to 8,
/// never to 64. Fails if an implementation multiplies the LEDs by `copies`.
#[tokio::test]
async fn a_mirror_row_commits_one_light_and_resizes_to_eight() {
    let rig = rig("mirror-commit", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let before = rig.server.resize_count(0);
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);

    let answer = committed(&rig, 0, &setup.session, &[mirror_row(Some(8))]).await;

    assert_eq!(answer.final_leds, 8, "the run counts once");
    assert_eq!(answer.lights.len(), 1, "one light, not eight");
    assert_eq!(answer.lights[0].led_count, 8);
    assert_eq!(answer.lights[0].light.copies, Some(8));
    assert_eq!(rig.server.resize_count(0), before + 2, "one raise and one resize, to 8");
    assert_eq!(rig.server.zone_leds(0, 0), Some(8), "the header resized to 8, not 64");
    let made = zone_parts(&read_lights(&rig.home), 0);
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].id, part_id(1));
    assert_eq!(made[0].leds, Some(LedSlice { start: 0, count: 8 }));
    assert_eq!(made[0].copies, Some(8));
    assert_eq!(made[0].kind, Some(ChainPartKind::Other), "devices, not fans");
    assert_eq!(made[0].name.as_deref(), Some("Same lights, shown 8 times"));
    assert_eq!(read_zones(&rig.home).sizes[BOARD][&0], 8);
    assert_eq!(chain_state(&rig, 0).await, Some(ChainState::SetUp));
    // The chain list reads the same: one part, 8 LEDs, and the header's own length is 8.
    let listed = chains(&rig).await;
    let first = listed.chains.iter().find(|chain| chain.zone_index == 0).expect("the header");
    assert_eq!(first.leds, 8);
    assert_eq!(first.parts.len(), 1);
    assert_eq!(first.parts[0].light.copies, Some(8));
}

/// A `copies` of 0, above the cap, or of 2 or more beside other rows is a 4xx with a plain
/// sentence, and it changes nothing; the cap itself is accepted and `Some(1)` is stored as none.
#[tokio::test]
async fn copies_zero_above_cap_and_beside_other_rows_are_refused() {
    let rig = rig("mirror-refused", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    save_rig_files(&rig).await;
    let lights_before = std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let resizes = rig.server.resize_count(0);

    let fan = |leds: u32| SetupPart {
        name: "Fan".into(),
        kind: ChainPartKind::Fan,
        leds,
        copies: None,
    };
    let cases: [(&str, Vec<SetupPart>, &str); 4] = [
        ("zero", vec![mirror_row(Some(0))], "shown 0 times"),
        ("33", vec![mirror_row(Some(33))], "counts up to 32"),
        ("two beside none", vec![mirror_row(Some(2)), fan(8)], "only part on that header"),
        ("none beside two", vec![fan(8), mirror_row(Some(2))], "only part on that header"),
    ];
    for (name, parts, sentence) in cases {
        let (status, body) = commit(&rig, 0, &setup.session, &parts).await;
        assert!(status.is_client_error(), "{name}: {status} {body}");
        let message = body["message"].as_str().unwrap_or_default();
        assert!(message.contains(sentence), "{name}: the answer says why: {body}");
    }
    assert_eq!(rig.server.resize_count(0), resizes, "a refused commit resizes nothing");
    assert_eq!(std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json"), lights_before);
    assert_eq!(chain_state(&rig, 0).await, None, "nothing was set up");

    // The session stayed open: the cap itself is accepted.
    let capped = SetupPart { leds: 8, ..mirror_row(Some(32)) };
    let answer = committed(&rig, 0, &setup.session, &[capped]).await;
    assert_eq!(answer.lights[0].light.copies, Some(32));
    assert_eq!(answer.final_leds, 8);

    // And once is stored as none.
    let again = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let answer = committed(&rig, 0, &again.session, &[mirror_row(Some(1))]).await;
    assert_eq!(answer.lights[0].light.copies, None, "Some(1) is stored as none");
}

/// A series (three fans of 12) is stored with no `copies` anywhere in `lights.json`.
#[tokio::test]
async fn series_output_has_no_copies() {
    let rig = rig("series-no-copies", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let fans: Vec<SetupPart> = (1..=3)
        .map(|n| SetupPart {
            name: format!("Fan {n}"),
            kind: ChainPartKind::Fan,
            leds: 12,
            copies: None,
        })
        .collect();

    let answer = committed(&rig, 0, &setup.session, &fans).await;

    assert_eq!(answer.final_leds, 36);
    assert_eq!(answer.lights.len(), 3);
    let text = std::fs::read_to_string(rig.home.join(LIGHTS_FILE)).expect("lights.json");
    assert!(!text.contains("copies"), "a series writes no copies: {text}");
}

/// A store loaded from `home`, ready to write: the scenes' wait for the first device list is over.
fn store_loaded(home: &Path) -> Store {
    let mut store = Store::load_from(home);
    store.scenes_pending = None;
    store
}

/// An old `lights.json` (no `copies` anywhere) loads and saves unchanged; a file with `copies`
/// keeps it across a load and a save.
#[test]
fn an_old_lights_file_loads_and_a_copies_file_round_trips() {
    let home = temp_home("copies-file");
    let mut store = Store::at(&home);
    store.lights = rig_lights();
    store.save_lights().expect("lights.json");
    let old = std::fs::read(home.join(LIGHTS_FILE)).expect("lights.json");
    assert!(!String::from_utf8_lossy(&old).contains("copies"), "an old file has no copies");

    let loaded = store_loaded(&home);
    assert_eq!(loaded.lights, rig_lights(), "an old file loads as it was");
    assert!(loaded.lights.lights.iter().all(|l| l.copies.is_none()));
    loaded.save_lights().expect("saves again");
    assert_eq!(std::fs::read(home.join(LIGHTS_FILE)).expect("lights.json"), old, "saved unchanged");

    let mut mirrored = store_loaded(&home);
    mirrored.lights.lights[0].copies = Some(8);
    mirrored.save_lights().expect("saves copies");
    let with = std::fs::read(home.join(LIGHTS_FILE)).expect("lights.json");
    assert!(String::from_utf8_lossy(&with).contains(r#""copies": 8"#), "copies is written");
    let reloaded = store_loaded(&home);
    assert_eq!(reloaded.lights.lights[0].copies, Some(8), "copies loads back");
    reloaded.save_lights().expect("saves again");
    assert_eq!(std::fs::read(home.join(LIGHTS_FILE)).expect("lights.json"), with, "and round-trips");
    let _ = std::fs::remove_dir_all(&home);
}

/// The counting step's slide: a white trail (LEDs 0..N-1) and an orange end (LED N-1) are two runs
/// in one light request, and the fake hardware shows exactly those LEDs. This path had no
/// coverage on the daemon side before (guard: the runs path itself is old).
#[tokio::test]
async fn a_white_trail_and_an_orange_end_light_the_fake() {
    let rig = rig("trail", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);

    let orange = Rgb8::new(255, 77, 0);
    let body = serde_json::json!({
        "session": setup.session,
        "runs": [
            { "start": 0, "count": 7, "color": "#ffffff" },
            { "start": 7, "count": 1, "color": "#ff4d00" },
        ],
    });
    let (status, answer) = send(&rig, "POST", &light_path(0), Some(&body.to_string())).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let lit = wait_until(Duration::from_secs(30), || {
        let colors = rig.server.colors(0);
        colors.len() == 360 && colors[7] == orange
    })
    .await;
    assert!(lit, "the runs never reached the fake: {:?}", &rig.server.colors(0)[..10]);
    let colors = rig.server.colors(0);
    assert!(colors[..7].iter().all(|c| *c == Rgb8::WHITE), "the trail is white: {:?}", &colors[..8]);
    assert_eq!(colors[7], orange, "the last LED is orange");
    assert!(colors[8..].iter().all(|c| *c == Rgb8::BLACK), "nothing past the end is lit");
}

/// The counting step starts with ONE LED on. One run of count 1 lights only LED 0; a run that
/// starts past the zone's end lights nothing (guard: `identify.rs` clamps runs to the zone).
#[tokio::test]
async fn one_led_start_lights_only_led_zero() {
    let rig = rig("one-led", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);

    let orange = Rgb8::new(255, 77, 0);
    let one = serde_json::json!({
        "session": setup.session,
        "runs": [{ "start": 0, "count": 1, "color": "#ff4d00" }],
    });
    let (status, answer) = send(&rig, "POST", &light_path(0), Some(&one.to_string())).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(
        wait_until(Duration::from_secs(30), || {
            let colors = rig.server.colors(0);
            colors.len() == 360 && colors[0] == orange
        })
        .await,
        "LED 0 never lit"
    );
    let colors = rig.server.colors(0);
    assert!(colors[1..].iter().all(|c| *c == Rgb8::BLACK), "only LED 0 is on: {:?}", &colors[..4]);

    let past = serde_json::json!({
        "session": setup.session,
        "runs": [{ "start": 300, "count": 1, "color": "#ff4d00" }],
    });
    let (status, answer) = send(&rig, "POST", &light_path(0), Some(&past.to_string())).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(
        wait_until(Duration::from_secs(30), || {
            rig.server.colors(0).iter().take(300).all(|c| *c == Rgb8::BLACK)
        })
        .await,
        "a run past the end must light nothing in the zone"
    );
}

/// A stale session is a 409 and changes nothing: no resize, no file, no record.
#[tokio::test]
async fn commit_with_a_stale_session_is_a_409_and_changes_nothing() {
    let rig = rig("commit-stale", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    save_rig_files(&rig).await;
    let lights_before = std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json");
    let scenes_before = std::fs::read(rig.home.join(SCENES_FILE)).expect("scenes.json");
    let zones_before = std::fs::read(rig.home.join(ZONES_FILE)).expect("zones.json");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let resizes = rig.server.resize_count(0);

    let (status, body) = commit(&rig, 0, "stale", &seven_parts()).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(rig.server.resize_count(0), resizes, "the stale commit resized nothing");
    assert_eq!(rig.server.zone_leds(0, 0), Some(300), "the header is still the setup's");
    assert_eq!(std::fs::read(rig.home.join(LIGHTS_FILE)).expect("lights.json"), lights_before);
    assert_eq!(std::fs::read(rig.home.join(SCENES_FILE)).expect("scenes.json"), scenes_before);
    assert_eq!(std::fs::read(rig.home.join(ZONES_FILE)).expect("zones.json"), zones_before);
    assert!(!rig.home.join(CHAIN_INTENT_FILE).exists(), "no intent was written");
    assert!(rig.home.join(CHAIN_RESTORE_FILE).exists(), "the setup's record is untouched");
    assert_eq!(
        rig.app.identify.setup_session().as_deref(),
        Some(setup.session.as_str()),
        "the open setup is untouched"
    );
}

/// The commit saves while a takeover holds the lights, and the scene lands when the takeover ends.
#[tokio::test]
async fn commit_during_a_takeover_saves_and_lands_when_the_takeover_ends() {
    let rig = rig("commit-takeover", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    save_rig_files(&rig).await;
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    // A takeover holds the lights, as the editor's draft does.
    //
    // It lands under the draft gate, the way every takeover lands (`draft.rs`: the draft, the
    // greeting and the pattern edit all take `app.draft.gate()` before they land one). The gate
    // is what keeps a takeover and a scene apply apart: a scene apply holds it across the whole
    // apply (`scenes.rs`, `apply_for_devices`), and its end clears the takeover flag. A takeover
    // that lands outside the gate can be lost by an apply that was already running, and the
    // commit would then put the scene over the takeover it was asked to leave alone (#494).
    {
        let _gate = rig.app.draft.gate().lock().await;
        rig.app
            .scene
            .land_takeover(Held { groups: vec!["all".into()], takeover: vec!["all".into()] });
    }

    let answer = committed(&rig, 0, &setup.session, &seven_parts()).await;

    assert_eq!(answer.final_leds, FINAL);
    assert_eq!(zone_parts(&read_lights(&rig.home), 0).len(), 7, "the files landed");
    assert_eq!(chain_state(&rig, 0).await, Some(ChainState::SetUp));
    assert!(rig.app.zone_holds.read().await.is_empty(), "the hold is dropped");
    assert!(rig.app.identify.setup_session().is_none(), "the session ended");
    let during: Vec<String> =
        rig.engine.scene_lights.lock().unwrap().iter().map(|light| light.id.clone()).collect();
    assert!(
        !during.iter().any(|id| id == &part_id(1)),
        "the scene was not put over the takeover: {during:?}"
    );
    // The takeover is still the one on the lights: the commit ended the setup, not the takeover,
    // and nothing may put the scene over it while it holds.
    assert!(
        !rig.app.scene.held().takeover.is_empty(),
        "the takeover still holds the lights: {:?}",
        rig.app.scene.held()
    );

    // The takeover ends the way the editor's own restore ends it: the active scene goes on.
    canvas_daemon::scenes::restore_active(&rig.app).await;
    let landed = wait_for(Duration::from_secs(30), || async {
        rig.engine.scene_lights.lock().unwrap().iter().any(|light| light.id == part_id(1))
    })
    .await;
    assert!(landed, "the committed parts never reached the scene after the takeover ended");
    let sink = sink_of(&rig, BOARD).await.expect("the board has a frame sink");
    sink.submit(vec![Rgb8::WHITE; 156]);
    assert!(
        wait_until(Duration::from_secs(30), || {
            let colors = rig.server.colors(0);
            colors.len() == 156 && colors.iter().all(|color| *color == Rgb8::WHITE)
        })
        .await,
        "the lights stayed dark after the takeover ended: {:?}",
        rig.server.colors(0)
    );
}

fn zone_path(zone: u32) -> String {
    format!("/api/v1/devices/{}/zones/{zone}", BOARD.replace(':', "%3A"))
}

/// The store's copy of the sizes is what a commit writes `zones.json` from: a size the owner
/// patched since start (or the first run saved) must survive another header's commit.
#[tokio::test]
async fn a_commit_keeps_the_other_headers_saved_lengths() {
    let rig = rig("commit-keeps-sizes", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    save_rig_files(&rig).await;
    let (status, body) = send(&rig, "PATCH", &zone_path(1), Some(r#"{"led_count":80}"#)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(read_zones(&rig.home).sizes[BOARD].get(&1).copied(), Some(80));

    let setup = started(&rig, 0).await;
    committed(&rig, 0, &setup.session, &seven_parts()).await;

    let zones = read_zones(&rig.home);
    assert_eq!(zones.sizes[BOARD].get(&0).copied(), Some(FINAL), "the committed header's length");
    assert_eq!(zones.sizes[BOARD].get(&1).copied(), Some(80), "the other header keeps its patched length");
}

/// A setup owns the header it raised: a zone PATCH on it is refused and resizes nothing.
#[tokio::test]
async fn a_zone_patch_on_a_header_being_set_up_is_a_409() {
    let rig = rig("patch-during-setup", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let before = rig.server.resize_count(0);

    let (status, body) = send(&rig, "PATCH", &zone_path(0), Some(r#"{"led_count":100}"#)).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(rig.server.resize_count(0), before, "no resize");
    assert_eq!(read_zones(&rig.home).sizes[BOARD].get(&0).copied(), Some(EARLIER), "zones.json is unchanged");
    let (status, body) = cancel(&rig, 0, &setup.session).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rig.server.zone_leds(0, 0), Some(EARLIER), "the cancel puts the earlier length back");
}

/// A restore record still on disk is a header not put back yet: a new setup would overwrite it,
/// so the start puts that header back first, as start-up does, and is refused only while it
/// cannot.
#[tokio::test]
async fn a_start_puts_an_earlier_header_back_first() {
    let rig = rig("start-pending-record", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    assert!(
        wait_until(Duration::from_secs(30), || rig
            .app
            .leftovers_replayed
            .load(std::sync::atomic::Ordering::SeqCst))
        .await,
        "the start-up replay never ran"
    );
    // A restore of header 1 failed and left its record.
    let pending = RestoreRecord { device_id: DeviceId(BOARD.into()), zone_index: 1, leds: SECOND_EARLIER };
    canvas_daemon::chain_records::write_restore(&rig.home, &pending).expect("the restore record");

    // While the device still refuses, the start is refused and the record is left as it was.
    rig.server.fail_resizes(true);
    let (status, body) = start(&rig, 0).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(rig.app.identify.setup_session(), None, "no session opened");
    assert_eq!(canvas_daemon::chain_records::read_restore(&rig.home), Some(pending.clone()));

    // Once it answers, the start puts header 1 back and then opens the setup on header 0.
    rig.server.fail_resizes(false);
    // The silent device dropped the connection; the supervisor makes a new one.
    assert!(
        wait_for(Duration::from_secs(30), || async {
            matches!(*rig.app.openrgb.read().await, canvas_proto::api::LinkState::Connected)
                && rig.app.adapter.lock().await.discover().await.is_ok()
        })
        .await,
        "the daemon never reconnected"
    );
    let resizes = rig.server.resize_count(0);
    let setup = started(&rig, 0).await;
    assert_eq!(
        rig.server.resize_count(0),
        resizes + 2,
        "the start put header 1 back, then raised header 0"
    );
    assert_eq!(rig.server.zone_leds(0, 1), Some(SECOND_EARLIER), "header 1 is at its earlier length");
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    assert_eq!(
        canvas_daemon::chain_records::read_restore(&rig.home),
        Some(RestoreRecord { device_id: DeviceId(BOARD.into()), zone_index: 0, leds: EARLIER }),
        "the record is the new setup's"
    );
    let (status, body) = cancel(&rig, 0, &setup.session).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Two ends of one setup at once (a cancel and the timeout): the header goes back once.
#[tokio::test]
async fn two_ends_at_once_put_the_header_back_once() {
    let rig = rig("two-ends", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let before = rig.server.resize_count(0);

    let (first, second) = tokio::join!(
        canvas_daemon::api::chains::finish_setup(&rig.app),
        canvas_daemon::api::chains::finish_setup(&rig.app),
    );

    assert!(first.is_ok() && second.is_ok());
    assert_eq!(rig.server.resize_count(0), before + 1, "one resize back, not two");
    assert_eq!(rig.server.zone_leds(0, 0), Some(EARLIER));
}

/// A cancel that arrives while a commit runs waits for it, then finds the session closed: the
/// commit stands, the header keeps its final length and the parts stay.
#[tokio::test]
async fn a_cancel_racing_a_commit_waits_and_the_commit_stands() {
    let rig = rig("cancel-racing-commit", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    save_rig_files(&rig).await;
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let before = rig.server.resize_count(0);

    let parts = seven_parts();
    let ((commit_status, commit_body), (cancel_status, cancel_body)) =
        tokio::join!(commit(&rig, 0, &setup.session, &parts), cancel(&rig, 0, &setup.session));

    assert_eq!(commit_status, StatusCode::OK, "{commit_body}");
    assert_eq!(cancel_status, StatusCode::CONFLICT, "{cancel_body}");
    assert_eq!(rig.server.resize_count(0), before + 1, "the commit's resize only");
    assert_eq!(rig.server.zone_leds(0, 0), Some(FINAL));
    assert_eq!(zone_parts(&read_lights(&rig.home), 0).len(), 7, "the parts stay");
    assert_eq!(read_zones(&rig.home).sizes[BOARD].get(&0).copied(), Some(FINAL));
}

/// A start-up replay that waited for the step lock behind a start runs after that setup opened:
/// the open setup's record and raised header are its own, and the replay leaves them alone.
#[tokio::test]
async fn a_late_replay_leaves_an_open_setup_alone() {
    let rig = rig("late-replay", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    let setup = started(&rig, 0).await;
    assert!(wait_until(Duration::from_secs(30), || rig.server.zone_leds(0, 0) == Some(300)).await);
    let resizes = rig.server.resize_count(0);

    canvas_daemon::api::chains::replay_leftovers(&rig.app).await;

    assert_eq!(rig.server.resize_count(0), resizes, "no resize");
    assert_eq!(rig.server.zone_leds(0, 0), Some(300), "the header stays raised");
    assert!(rig.home.join(CHAIN_RESTORE_FILE).exists(), "the setup keeps its record");
    let (status, body) = cancel(&rig, 0, &setup.session).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rig.server.zone_leds(0, 0), Some(EARLIER));
}
