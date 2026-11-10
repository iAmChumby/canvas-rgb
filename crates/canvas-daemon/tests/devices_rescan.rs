//! #233: `GET /api/v1/devices?rescan=1` asks OpenRGB again and answers with the new list; a plain
//! `GET` still answers from the list the daemon holds.
//!
//! The adapter is the real OpenRGB adapter talking to the fake OpenRGB server: no hardware.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::api::DevicesResponse;
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::DeviceId;
use common::FakeEngine;
use std::sync::Arc;
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
    home: std::path::PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// An app whose adapter is the real OpenRGB adapter on `port`. `connected` connects it and loads
/// its device list into the app, as the supervisor does at start.
async fn rig(name: &str, port: u16, connected: bool) -> Rig {
    let home = std::env::temp_dir().join(format!("canvas-rgb-rescan-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp directory is created");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        canvas_daemon::storage::Store::at(&home),
        Arc::new(FakeEngine::default()),
        Box::new(OpenRgbAdapter::new("127.0.0.1", port)),
        TOKEN.into(),
        "0.0.0-test".into(),
        tx,
    ));
    if connected {
        let devices = {
            let mut adapter = app.adapter.lock().await;
            canvas_daemon::openrgb_supervisor::connect_and_prepare(adapter.as_mut(), &Default::default())
                .await
                .expect("the fake server answers")
        };
        *app.devices.write().await = devices;
    }
    Rig { router: api::router(app.clone()), app, home }
}

async fn get(rig: &Rig, path: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds");
    let response = rig.router.clone().oneshot(request).await.expect("the router answers");
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

#[tokio::test]
async fn a_device_added_after_start_appears_only_after_a_rescan() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let rig = rig("appears", server.port(), true).await;
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

#[tokio::test]
async fn a_rescan_with_openrgb_down_says_so_and_keeps_the_held_list() {
    // A port nobody listens on: the fake server is gone.
    let port = {
        let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
        server.port()
    };
    let rig = rig("down", port, false).await;
    *rig.app.devices.write().await = vec![stick("one")];

    let (status, body) = get(&rig, "/api/v1/devices?rescan=1").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{}", String::from_utf8_lossy(&body));
    assert_eq!(ids(&rig, "/api/v1/devices").await, ["openrgb:one"], "the held list is untouched");
}
