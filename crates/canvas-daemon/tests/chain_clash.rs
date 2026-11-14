//! Chain setup raises a header to its longest length while the owner drags (#338). The OpenRGB
//! supervisor re-applies every saved zone size each refresh, so a raise on a header that first run
//! sized to 120 is "wrong" to it. This test runs the real supervisor against the fake server to
//! settle whether the refresh resizes the raise back.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_daemon::openrgb_supervisor::supervise_with_rescans;
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::traits::DeviceAdapter;
use canvas_proto::DeviceId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// A board with one JARGB header: any size from 0 to 300.
fn board() -> Vec<DeviceInfo> {
    vec![DeviceInfo {
        id: DeviceId("openrgb:mb".into()),
        name: "MSI MPG B850 Mystic Light".into(),
        vendor: "Fake".into(),
        kind: DeviceKind::Motherboard,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "mb".into(),
        location: "usb".into(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "JARGB1".into(),
            kind: ZoneKind::Linear,
            led_count: 0,
            min_leds: 0,
            max_leds: 300,
            resizable: true,
        }],
        connected: true,
    }]
}

async fn wait_for(server: &FakeOpenRgbServer, leds: u32, within: Duration) -> bool {
    let end = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < end {
        if server.zone_leds(0, 0) == Some(leds) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
#[ignore = "proves the clash; flipped by Task 3"]
async fn the_supervisor_resizes_a_raised_header_back_to_its_saved_size() {
    let server = FakeOpenRgbServer::start(board()).await.expect("fake server");
    let adapter: Box<dyn DeviceAdapter> = Box::new(OpenRgbAdapter::new("127.0.0.1", server.port()));
    let adapter = Arc::new(tokio::sync::Mutex::new(adapter));
    let sizes = BTreeMap::from([("openrgb:mb".to_string(), BTreeMap::from([(0u32, 120u32)]))]);
    let zone_sizes = Arc::new(tokio::sync::RwLock::new(sizes));
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

    let task = tokio::spawn(supervise_with_rescans(
        adapter.clone(),
        zone_sizes,
        |_devices| async {},
        |_link| async {},
        None,
        stop_rx,
    ));

    assert!(wait_for(&server, 120, Duration::from_secs(5)).await, "connect applies the saved 120");

    // The raise chain setup does when it starts.
    adapter.lock().await.set_zone_size(&DeviceId("openrgb:mb".into()), 0, 300).await.expect("raise");
    assert_eq!(server.zone_leds(0, 0), Some(300));

    tokio::time::sleep(Duration::from_secs(6)).await;
    let after = server.zone_leds(0, 0);
    println!("zone_leds(0, 0) 6 s after the raise: {after:?}");

    stop_tx.send(true).ok();
    task.abort();
    assert_eq!(after, Some(300), "the supervisor must leave a raised header alone");
}
