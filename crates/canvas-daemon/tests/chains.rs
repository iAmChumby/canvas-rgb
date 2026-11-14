//! The chain routes (#338, Task 7a): `GET /api/v1/chains`, and a setup session's start, light,
//! cancel and empty, with the header's earlier length put back when the session ends short.
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
use canvas_daemon::chain_records::{read_restore, RestoreRecord};
use canvas_daemon::state::App;
use canvas_daemon::storage::Store;
use canvas_proto::api::{
    routes, ChainSetupState, ChainsResponse, LightView, SetupEmpty, SetupStarted,
};
use canvas_proto::config::ZonesFile;
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::scene::{
    zone_light_id, Arrangement, ChainState, Group, GroupMode, Light, LightsFile, Plays, Scene,
    ScenesFile, CHAIN_RESTORE_FILE, LIGHTS_SCHEMA, SCENES_SCHEMA,
};
use canvas_proto::traits::FrameSink;
use canvas_proto::{DeviceId, Rgb8};
use common::FakeEngine;
use std::path::PathBuf;
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

/// An app whose adapter is the real OpenRGB adapter on the fake server, with the daemon's
/// supervisor running. With `saved`, `zones.json` holds first run's sizes (120 and 60), so the
/// supervisor re-applies them at connect; without it the file has no entry, as when first run never
/// sized the header.
async fn rig(name: &str, server: FakeOpenRgbServer, saved: bool) -> Rig {
    let home =
        std::env::temp_dir().join(format!("canvas-rgb-chains-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp directory is created");
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

/// Start writes the earlier length to `chain-restore.json` before the raise lands, raises the
/// header to its maximum once, and answers both lengths.
#[tokio::test]
async fn start_raises_to_the_zone_maximum_once_and_writes_the_restore_record_first() {
    let rig = rig("start", fake(EARLIER).await, true).await;
    assert!(connected(&rig).await, "the daemon never applied a device list");
    // The connect applied the saved sizes; every later count is measured from here.
    let before = rig.server.resize_count(0);
    // A raise the fake holds back: while it is still landing the record must already be there.
    rig.server.delay_resizes(1_500);

    let router = rig.router.clone();
    let path = setup_path(0);
    let pending =
        tokio::spawn(async move { router.oneshot(request("POST", &path, None)).await });
    let record = wait_until(Duration::from_secs(30), || {
        rig.home.join(CHAIN_RESTORE_FILE).exists()
    })
    .await;
    assert!(record, "the restore record was never written");
    assert_eq!(
        rig.server.zone_leds(0, 0),
        Some(EARLIER),
        "the record is written before the raise lands"
    );
    assert_eq!(
        read_restore(&rig.home).map(|record| record.leds),
        Some(EARLIER),
        "the record holds the earlier length"
    );

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
        "one raise, and the delay did not add a second"
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
    assert!(!rig.home.join(CHAIN_RESTORE_FILE).exists(), "the record is deleted");
    assert!(rig.app.zone_holds.read().await.is_empty(), "the hold is dropped");
    assert!(rig.app.identify.setup_session().is_none(), "the setup ended");

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
    assert!(!rig.home.join(CHAIN_RESTORE_FILE).exists(), "the record is deleted");
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
