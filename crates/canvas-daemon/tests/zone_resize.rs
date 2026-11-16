//! #308: a client that goes away in the middle of a zone resize does not break the OpenRGB
//! link, and the device list carries the new LED count.
//!
//! The resize asks OpenRGB to change the zone and then reads the device list again. The
//! second one is a request, and a request that is given up on breaks the OpenRGB link: the
//! reply may still come, and it would answer the next request. Every light would pause for
//! one reconnect.
//!
//! The fakes: the fake OpenRGB server and the real OpenRGB adapter, so the whole link is
//! walked. No hardware is touched.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A controller whose zone can be resized, as an MSI JARGB header.
fn header() -> DeviceInfo {
    DeviceInfo {
        id: DeviceId("openrgb:jar".into()),
        name: "JARGB".into(),
        vendor: "fake".into(),
        kind: DeviceKind::Motherboard,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "jar".into(),
        location: "smbus".into(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "Header".into(),
            kind: ZoneKind::Linear,
            led_count: 8,
            min_leds: 4,
            max_leds: 16,
            resizable: true,
        }],
        connected: true,
    }
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

/// The daemon, with the real OpenRGB adapter on the fake server's port.
struct Rig {
    router: axum::Router,
    app: Arc<App>,
    home: std::path::PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

async fn rig(port: u16, name: &str) -> Rig {
    let home =
        std::env::temp_dir().join(format!("canvas-rgb-zone-resize-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp directory is created");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let engine = Arc::new(common::FakeEngine::default());
    let app = Arc::new(App::new(
        canvas_daemon::storage::Store::at(&home),
        engine,
        Box::new(OpenRgbAdapter::new("127.0.0.1", port)),
        TOKEN.into(),
        "0.0.0-test".into(),
        tx,
    ));
    // The daemon's supervisor connects the adapter and reads the devices. Here the test does
    // it, so nothing else asks OpenRGB while the test watches.
    {
        let mut adapter = app.adapter.lock().await;
        adapter.connect().await.expect("connect");
        let devices = adapter.discover().await.expect("the devices are read");
        *app.devices.write().await = devices;
    }
    Rig { router: api::router(app.clone()), app, home }
}

fn patch_request(id: &str, zone: u32, leds: u32) -> Request<Body> {
    Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/devices/{id}/zones/{zone}"))
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(format!(r#"{{"led_count":{leds}}}"#)))
        .expect("the request builds")
}

/// The LED count the daemon's device list holds for the header's zone.
fn listed_leds(rig: &Rig) -> Option<u32> {
    rig.app
        .devices
        .try_read()
        .ok()
        .and_then(|list| list.first().and_then(|device| device.zones.first()).map(|zone| zone.led_count))
}

/// The zone size the daemon saved.
fn saved_leds(rig: &Rig) -> Option<u32> {
    rig.app
        .zones
        .try_read()
        .ok()
        .and_then(|zones| zones.get("openrgb:jar").and_then(|zone| zone.get(&0)).copied())
}

/// A client that goes away in the middle of the resize keeps the OpenRGB link, and the
/// device list carries the new LED count.
///
/// The tab was closed while OpenRGB was re-configuring the controller. The request that was
/// on the wire must not be cancelled with the client: a cancelled request breaks the link,
/// and every light pauses for one reconnect. The work the client asked for goes on, so the
/// device list carries the new LED count and the frames go out at the right size.
#[tokio::test]
async fn a_client_that_leaves_mid_resize_keeps_the_link_and_the_new_led_count() {
    let server = FakeOpenRgbServer::start(vec![header()]).await.expect("fake server");
    let rig = rig(server.port(), "leaves").await;

    // The server keeps its replies back: the resize's re-read of the device list waits for
    // one that never comes.
    server.hold_replies(true);
    let router = rig.router.clone();
    let request = tokio::spawn(async move {
        router.oneshot(patch_request("openrgb:jar", 0, 12)).await
    });
    // The resize reached OpenRGB, and the device list is being read again.
    until("the resize waits for the device list", || server.held_replies() >= 1).await;
    assert_eq!(server.zone_leds(0, 0), Some(12), "the zone was resized");

    // The client goes away: the tab was closed in the middle of the resize.
    request.abort();
    let _ = request.await;

    // OpenRGB answers now.
    server.hold_replies(false);
    server.release_replies();

    // The link is whole: the request that went out was not cancelled with the client.
    let connected = rig.app.adapter.lock().await.is_connected();
    assert!(connected, "the OpenRGB link broke when the client left");

    // The device list carries the new LED count: the frames go out at the right size, and
    // not at the old one until the next refresh re-applies the saved size.
    until("the device list has the new LED count", || listed_leds(&rig) == Some(12)).await;
    assert_eq!(listed_leds(&rig), Some(12), "the device list has the old LED count");

    // The size is saved, so the next OpenRGB connect re-applies it.
    assert_eq!(saved_leds(&rig), Some(12), "the zone size was not saved");
}

/// The resize still answers the client that stayed.
#[tokio::test]
async fn a_resize_that_the_client_stays_for_answers_ok() {
    let server = FakeOpenRgbServer::start(vec![header()]).await.expect("fake server");
    let rig = rig(server.port(), "stays").await;

    let response = rig
        .router
        .clone()
        .oneshot(patch_request("openrgb:jar", 0, 12))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(listed_leds(&rig), Some(12), "the device list has the new LED count");
    assert_eq!(saved_leds(&rig), Some(12), "the zone size was not saved");
    assert!(rig.app.adapter.lock().await.is_connected(), "the OpenRGB link broke");
}
