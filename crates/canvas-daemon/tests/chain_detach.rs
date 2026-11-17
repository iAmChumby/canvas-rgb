//! #457: a client that goes away in the middle of a chain step does not break the OpenRGB link,
//! and the setup state stays whole.
//!
//! The chain routes raise a header, resize it to the parts' sum and put it back. Every one of
//! those is a request to OpenRGB, and a request that is given up on breaks the link: the reply
//! may still come, and it would answer the next request. Every light would pause for one
//! reconnect, and the step would be left half done: the record on disk, the hold in memory and
//! the header somewhere in between.
//!
//! The fakes: the fake OpenRGB server and the real OpenRGB adapter, so the whole link is
//! walked. The daemon's supervisor is not running: nothing else may ask OpenRGB while the test
//! watches. No hardware is touched.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_daemon::api;
use canvas_daemon::chain_records::{read_intent, read_restore, RestoreRecord};
use canvas_daemon::state::App;
use canvas_daemon::storage::Store;
use canvas_proto::api::{routes, SetupCommit, SetupEmpty, SetupPart, SetupStarted};
use canvas_proto::config::{ZonesFile, ZONES_FILE};
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::scene::{
    zone_light_id, Arrangement, ChainPartKind, Group, GroupMode, Light, LightsFile, Plays, Scene,
    ScenesFile, LIGHTS_SCHEMA, SCENES_SCHEMA,
};
use canvas_proto::DeviceId;
use common::FakeEngine;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// The board whose first JARGB header is the chain under test.
const BOARD: &str = "openrgb:mb";
/// The earlier length of the board's first header: what first run sized it to.
const EARLIER: u32 = 120;
/// The length a setup raises the header to: the zone's maximum.
const RAISED: u32 = 300;
/// What seven parts of the header add up to.
const FINAL: u32 = 96;

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
        zones: vec![header(0, "JARGB1", first), header(1, "JARGB2", 60)],
        connected: true,
    }
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
        lights: vec![whole_zone(BOARD, 0), whole_zone(BOARD, 1)],
        chains: Vec::new(),
    }
}

/// One scene whose single group names every light. `Plays::Off` keeps the engine from building an
/// effect: what the daemon hands the engine still names the lights.
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
    server: FakeOpenRgbServer,
    home: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// The daemon, with the real OpenRGB adapter on the fake server's port and no supervisor: the
/// test connects the adapter itself, so nothing else asks OpenRGB while the test watches.
async fn rig(name: &str, server: FakeOpenRgbServer) -> Rig {
    let home = std::env::temp_dir()
        .join(format!("canvas-rgb-chain-detach-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp directory is created");
    let mut store = Store::at(&home);
    store.config.first_run_done = true;
    store.lights = rig_lights();
    store.scenes = rig_scenes(&store.lights.lights);
    store.zones = ZonesFile {
        schema_version: canvas_proto::config::ZONES_SCHEMA,
        sizes: std::collections::BTreeMap::from([(
            BOARD.to_string(),
            std::collections::BTreeMap::from([(0, EARLIER)]),
        )]),
    };
    store.save_zones().expect("zones.json");
    let sizes = store.zones.sizes.clone();
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine,
        Box::new(OpenRgbAdapter::new("127.0.0.1", server.port())),
        TOKEN.into(),
        "0.0.0-test".into(),
        tx,
    ));
    *app.zones.write().await = sizes;
    {
        let mut adapter = app.adapter.lock().await;
        adapter.connect().await.expect("connect");
        let devices = adapter.discover().await.expect("the devices are read");
        *app.devices.write().await = devices;
    }
    Rig { router: api::router(app.clone()), app, server, home }
}

/// Waits until `ready` is true. The wait is on the thing the assert reads; the long limit
/// only ends a test that would hang.
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
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

fn setup_path(zone: u32) -> String {
    routes::CHAIN_SETUP.replace("{device}", BOARD).replace("{zone}", &zone.to_string())
}

fn commit_path(zone: u32) -> String {
    routes::CHAIN_SETUP_COMMIT.replace("{device}", BOARD).replace("{zone}", &zone.to_string())
}

fn empty_path(zone: u32) -> String {
    routes::CHAIN_SETUP_EMPTY.replace("{device}", BOARD).replace("{zone}", &zone.to_string())
}

fn light_path(zone: u32) -> String {
    routes::CHAIN_SETUP_LIGHT.replace("{device}", BOARD).replace("{zone}", &zone.to_string())
}

/// The board's first header set up as seven fans: an even split of the 96 LEDs.
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

/// The LED count the daemon's device list holds for the board's first header.
fn listed_leds(rig: &Rig) -> Option<u32> {
    rig.app
        .devices
        .try_read()
        .ok()
        .and_then(|list| list.first().and_then(|device| device.zones.first()).map(|zone| zone.led_count))
}

/// The length `zones.json` holds for the board's first header.
fn saved_leds(rig: &Rig) -> Option<u32> {
    let bytes = std::fs::read(rig.home.join(ZONES_FILE)).ok()?;
    let zones: ZonesFile = serde_json::from_slice(&bytes).ok()?;
    zones.sizes.get(BOARD).and_then(|zones| zones.get(&0)).copied()
}

/// The hold the daemon keeps for the board's first header.
fn held_leds(rig: &Rig) -> Option<u32> {
    rig.app.zone_holds.try_read().ok().and_then(|holds| holds.get(&(BOARD.to_string(), 0)).copied())
}

/// Whether the OpenRGB link is whole.
async fn linked(rig: &Rig) -> bool {
    rig.app.adapter.lock().await.is_connected()
}

/// Opens a setup on the board's first header and returns what the route answered.
async fn started(rig: &Rig) -> SetupStarted {
    let response = rig
        .router
        .clone()
        .oneshot(request("POST", &setup_path(0), None))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK, "the setup did not start");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("the body reads");
    serde_json::from_slice(&bytes).expect("a setup start")
}

/// A light request: it renews the setup's idle clock, so the setup's own timeout does not end it
/// while the test watches.
async fn light(rig: &Rig, session: &str) {
    let body = serde_json::json!({
        "session": session,
        "runs": [{ "start": 0, "count": 100, "color": "#ff0000" }],
    });
    let response = rig
        .router
        .clone()
        .oneshot(request("POST", &light_path(0), Some(&body.to_string())))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK, "the light request was refused");
}

/// A client that goes away in the middle of the raise keeps the OpenRGB link, and the setup is
/// whole: the record names the earlier length, the hold names the raised one, the session is
/// open, and the device list carries the raised length.
///
/// The tab was closed while OpenRGB was re-configuring the controller. The request that was on
/// the wire must not be cancelled with the client: a cancelled request breaks the link, and
/// every light pauses for one reconnect. The step the client asked for goes on, so the setup is
/// a whole open setup and not a half-recorded one.
#[tokio::test]
async fn a_client_that_leaves_mid_start_keeps_the_link_and_the_setup_is_whole() {
    let server = FakeOpenRgbServer::start(vec![board(EARLIER)]).await.expect("fake server");
    let rig = rig("start", server).await;

    // The server keeps its replies back: the raise's re-read of the device list waits for one
    // that never comes.
    rig.server.hold_replies(true);
    let router = rig.router.clone();
    let path = setup_path(0);
    let client = tokio::spawn(async move { router.oneshot(request("POST", &path, None)).await });

    // The raise reached OpenRGB, and the device list is being read again.
    until("the raise waits for the device list", || rig.server.held_replies() >= 1).await;
    assert_eq!(rig.server.zone_leds(0, 0), Some(RAISED), "the header was raised");

    // The client goes away: the tab was closed in the middle of the raise.
    client.abort();
    let _ = client.await;

    // OpenRGB answers now.
    rig.server.hold_replies(false);
    rig.server.release_replies();

    // The link is whole: the request that went out was not cancelled with the client.
    assert!(linked(&rig).await, "the OpenRGB link broke when the client left");

    // The setup is whole: the device list carries the raised length, so the frames go out at
    // the right size.
    until("the device list has the raised length", || listed_leds(&rig) == Some(RAISED)).await;
    assert_eq!(listed_leds(&rig), Some(RAISED), "the device list still carries the earlier length");
    assert_eq!(
        read_restore(&rig.home),
        Some(RestoreRecord {
            device_id: DeviceId(BOARD.into()),
            zone_index: 0,
            leds: EARLIER,
        }),
        "the restore record names the earlier length"
    );
    assert_eq!(held_leds(&rig), Some(RAISED), "the hold names the raised length");
    assert!(rig.app.identify.setup_session().is_some(), "the setup is open");
}

/// A client that goes away in the middle of the commit keeps the OpenRGB link, and the commit
/// lands whole: the parts, their groups and the header's length are saved together, and both
/// records are gone.
///
/// The commit's resize is the second and last resize of the setup. A request that is given up on
/// breaks the link, and the commit would be left with its intent record on disk and its files
/// unwritten: a half-recorded setup the next start would undo.
#[tokio::test]
async fn a_client_that_leaves_mid_commit_keeps_the_link_and_the_commit_lands() {
    let server = FakeOpenRgbServer::start(vec![board(EARLIER)]).await.expect("fake server");
    let rig = rig("commit", server).await;
    let setup = started(&rig).await;
    until("the header is raised", || rig.server.zone_leds(0, 0) == Some(RAISED)).await;

    // The server keeps its replies back: the commit's resize waits for a reply that never comes.
    rig.server.hold_replies(true);
    let router = rig.router.clone();
    let path = commit_path(0);
    let body = SetupCommit { session: setup.session.clone(), parts: seven_parts() };
    let client = tokio::spawn(async move {
        router
            .oneshot(request("POST", &path, Some(&serde_json::to_string(&body).expect("a commit body"))))
            .await
    });

    // The commit's resize reached OpenRGB.
    until("the commit waits for the device list", || rig.server.held_replies() >= 1).await;
    assert_eq!(rig.server.zone_leds(0, 0), Some(FINAL), "the header took the parts' sum");

    // The client goes away: the window was closed in the middle of the commit.
    client.abort();
    let _ = client.await;

    // OpenRGB answers now.
    rig.server.hold_replies(false);
    rig.server.release_replies();

    // The link is whole: the request that went out was not cancelled with the client.
    assert!(linked(&rig).await, "the OpenRGB link broke when the client left");

    // The commit landed: both records are gone and the files hold the new length.
    until("the intent record is gone", || read_intent(&rig.home).is_none()).await;
    assert!(read_intent(&rig.home).is_none(), "the commit's intent record stayed");
    assert!(read_restore(&rig.home).is_none(), "the setup's restore record stayed");
    assert_eq!(saved_leds(&rig), Some(FINAL), "zones.json does not hold the parts' sum");
    assert_eq!(held_leds(&rig), None, "the hold stayed");
    assert!(rig.app.identify.setup_session().is_none(), "the setup is still open");
}

/// A client that goes away in the middle of the cancel keeps the OpenRGB link, and the header
/// goes back: the earlier length is back, the hold and the record are gone and the setup has
/// ended.
///
/// The resize back is the request that puts the header where it was. A request that is given up
/// on breaks the link, and the cancel would be left with its record on disk and its hold in
/// memory: a header the next start would put back again.
#[tokio::test]
async fn a_client_that_leaves_mid_cancel_keeps_the_link_and_the_header_goes_back() {
    let server = FakeOpenRgbServer::start(vec![board(EARLIER)]).await.expect("fake server");
    let rig = rig("cancel", server).await;
    let setup = started(&rig).await;
    until("the header is raised", || rig.server.zone_leds(0, 0) == Some(RAISED)).await;
    light(&rig, &setup.session).await;

    // The server keeps its replies back: the resize back waits for a reply that never comes.
    rig.server.hold_replies(true);
    let router = rig.router.clone();
    let path = setup_path(0);
    let body = SetupEmpty { session: setup.session.clone() };
    let client = tokio::spawn(async move {
        router
            .oneshot(request("DELETE", &path, Some(&serde_json::to_string(&body).expect("a cancel body"))))
            .await
    });

    // The resize back reached OpenRGB.
    until("the cancel waits for the device list", || rig.server.held_replies() >= 1).await;
    assert_eq!(rig.server.zone_leds(0, 0), Some(EARLIER), "the earlier length is back");

    // The client goes away: the window was closed in the middle of the cancel.
    client.abort();
    let _ = client.await;

    // OpenRGB answers now.
    rig.server.hold_replies(false);
    rig.server.release_replies();

    // The link is whole: the request that went out was not cancelled with the client.
    assert!(linked(&rig).await, "the OpenRGB link broke when the client left");

    // The setup is over: the record is gone, the hold is gone and the session ended.
    until("the restore record is gone", || read_restore(&rig.home).is_none()).await;
    until("the device list has the earlier length", || listed_leds(&rig) == Some(EARLIER)).await;
    assert!(read_restore(&rig.home).is_none(), "the setup's restore record stayed");
    assert_eq!(held_leds(&rig), None, "the hold stayed");
    assert!(rig.app.identify.setup_session().is_none(), "the setup is still open");
    assert_eq!(listed_leds(&rig), Some(EARLIER), "the device list still carries the raised length");
}

/// A client that goes away in the middle of the empty keeps the OpenRGB link, and the header
/// goes back: the earlier length is back, the hold and the record are gone and the chain is
/// recorded empty.
///
/// The empty route ends the setup the way a cancel does, and the resize back is the same
/// request: a request that is given up on breaks the link.
#[tokio::test]
async fn a_client_that_leaves_mid_empty_keeps_the_link_and_the_header_goes_back() {
    let server = FakeOpenRgbServer::start(vec![board(EARLIER)]).await.expect("fake server");
    let rig = rig("empty", server).await;
    let setup = started(&rig).await;
    until("the header is raised", || rig.server.zone_leds(0, 0) == Some(RAISED)).await;
    light(&rig, &setup.session).await;

    // The server keeps its replies back: the resize back waits for a reply that never comes.
    rig.server.hold_replies(true);
    let router = rig.router.clone();
    let path = empty_path(0);
    let body = SetupEmpty { session: setup.session.clone() };
    let client = tokio::spawn(async move {
        router
            .oneshot(request("POST", &path, Some(&serde_json::to_string(&body).expect("an empty body"))))
            .await
    });

    // The resize back reached OpenRGB.
    until("the empty waits for the device list", || rig.server.held_replies() >= 1).await;
    assert_eq!(rig.server.zone_leds(0, 0), Some(EARLIER), "the earlier length is back");

    // The client goes away: the window was closed in the middle of the empty.
    client.abort();
    let _ = client.await;

    // OpenRGB answers now.
    rig.server.hold_replies(false);
    rig.server.release_replies();

    // The link is whole: the request that went out was not cancelled with the client.
    assert!(linked(&rig).await, "the OpenRGB link broke when the client left");

    // The setup is over: the record is gone, the hold is gone and the session ended.
    until("the restore record is gone", || read_restore(&rig.home).is_none()).await;
    assert!(read_restore(&rig.home).is_none(), "the setup's restore record stayed");
    assert_eq!(held_leds(&rig), None, "the hold stayed");
    assert!(rig.app.identify.setup_session().is_none(), "the setup is still open");
}
