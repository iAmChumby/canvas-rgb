//! A frame that arrives while the device list is being read is skipped, not queued behind it.
//!
//! The frame writers go out on the OpenRGB link itself, so they do not wait for the adapter
//! lock the supervisor holds (#233). But a frame for a device the adapter does not know yet
//! reads the list first, and that read takes the request gate a discovery holds. The frame
//! would then wait out the whole discovery: the lights would freeze for as long as OpenRGB
//! takes to answer.
//!
//! A frame is cheap to lose and the discovery is about to fill the list, so the frame is
//! skipped instead of waiting.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_proto::{DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8, ZoneInfo, ZoneKind};
use std::time::Duration;

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

/// Waits until `ready` is true. The wait is on the thing the assert reads; the long limit
/// only ends a test that would hang.
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
}

/// A frame that arrives while the device list is being read does not wait for the read.
#[tokio::test]
async fn a_frame_while_the_device_list_is_being_read_is_skipped() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");

    // The server keeps its replies back: the discovery waits for its reply, and the list
    // stays empty.
    server.hold_replies(true);
    let mut asking = adapter.clone();
    let discovery = tokio::spawn(async move { asking.discover().await });
    until("the discovery is waiting for its reply", || server.held_replies() == 1).await;

    // The frame does not wait for the discovery: it is skipped.
    let id = DeviceId("openrgb:one".into());
    // The wait is a hang guard: it ends a test that would hang.
    let frame = tokio::time::timeout(Duration::from_secs(10), adapter.write_frame_shared(&id, &[Rgb8::WHITE; 8])).await;
    assert!(frame.is_ok(), "the frame waited for the discovery");
    assert!(frame.expect("the frame ends").is_err(), "the frame was written");
    assert!(!discovery.is_finished(), "the discovery ended while the frame went out");
    assert_eq!(server.update_leds_writes(0), 0, "the frame reached OpenRGB");
}
