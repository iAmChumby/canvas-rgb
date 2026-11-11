//! A frame goes out while a request waits for its reply, and it is not taken for that reply.
//!
//! The frame is `SET_CUSTOM_MODE` and `UPDATE_LEDS`: neither has a reply, and neither is ever
//! awaited. It shares the link with the adapter, so it does not wait on the request the daemon's
//! supervisor is making (#233).

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

async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
}

#[tokio::test]
async fn a_frame_goes_out_while_a_request_waits_and_is_not_taken_for_its_reply() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("a plain discover");

    server.hold_replies(true);
    let mut asking = adapter.clone();
    let discovery = tokio::spawn(async move { asking.discover().await });
    until("the discovery is waiting on its reply", || server.held_replies() == 1).await;

    // The frame goes through the shared link, with the request still unanswered.
    let id = DeviceId("openrgb:one".into());
    adapter.write_frame_shared(&id, &[Rgb8::WHITE; 8]).await.expect("the frame is written");
    until("the frame reaches the lights", || server.colors(0) == vec![Rgb8::WHITE; 8]).await;
    assert!(!discovery.is_finished(), "the frame did not answer the request");

    // The reply comes, and it is the reply to the request.
    server.hold_replies(false);
    server.release_replies();
    let found = tokio::time::timeout(Duration::from_secs(30), discovery)
        .await
        .expect("the discovery ends")
        .expect("its task ends")
        .expect("the discovery works");
    assert_eq!(found.len(), 1);
}
