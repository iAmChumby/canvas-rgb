//! #290: `DeviceAdapter::rescan` asks OpenRGB to look at the hardware again.
//!
//! Packet 140 is client-only and gets no reply, so it goes out as a fire-and-forget write. The
//! server reports the end of detection as a broadcast (101, then 103 and 100), which must never be
//! taken for the reply to a pending request. Everything runs against the fake OpenRGB server on
//! loopback: no real hardware. Each wait is on state the assert reads; the long limit only ends a
//! test that would hang.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_proto::{AdapterError, DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
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

/// Waits until `ready` is true. The wait is on the thing the assert reads; the long limit only
/// ends a test that would hang.
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
}

/// The rescan packet goes out and the link stays up. A `request()` here would wait for a reply the
/// server never sends, hit the 5 s timeout and drop the link; `send()` leaves it usable.
#[tokio::test]
async fn a_rescan_sends_packet_140_and_keeps_the_link_up() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("first discover");
    assert_eq!(server.rescan_count(), 0);

    adapter.rescan().await.expect("rescan");
    until("packet 140 reached OpenRGB", || server.rescan_count() == 1).await;
    assert!(adapter.is_connected(), "a fire-and-forget write broke the link");

    // The link is still usable: a request after the rescan gets its own answer.
    let devices = adapter.discover().await.expect("discover after the rescan");
    assert_eq!(devices.len(), 1, "the link did not survive the rescan");
}

/// A device plugged in after OpenRGB started (hidden until detection) is found by the next
/// discover after the rescan makes OpenRGB re-detect it.
#[tokio::test]
async fn a_device_revealed_by_detection_is_found_by_the_next_discover() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.add_hidden_device(stick("two"));
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");

    let before = adapter.discover().await.expect("first discover");
    assert_eq!(before.len(), 1, "the hidden device is not announced yet");

    adapter.rescan().await.expect("rescan");
    let after = adapter.discover().await.expect("discover after the rescan");
    let ids: Vec<_> = after.iter().map(|d| d.id.0.as_str()).collect();
    assert_eq!(ids, ["openrgb:one", "openrgb:two"]);
}

/// A detection that never finishes times out, and the link stays up so the plain re-read can
/// still answer with the list as it stands.
#[tokio::test]
async fn a_detection_that_never_finishes_times_out_with_the_link_up() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.hold_detection(true);
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");

    let failed = adapter.rescan().await.expect_err("detection never finished");
    assert!(
        matches!(&failed, AdapterError::Protocol(reason) if reason == "OpenRGB did not finish detection in time"),
        "{failed:?}"
    );
    assert!(adapter.is_connected(), "a detection timeout broke the link");
    assert_eq!(server.rescan_count(), 1, "packet 140 still went out");
    assert_eq!(
        adapter.discover().await.expect("discover after the timeout").len(),
        1,
        "the link did not survive the timeout"
    );
}

/// An OpenRGB that only speaks protocol 4 has no rescan packet: the call is refused as unsupported
/// and nothing is written to the wire.
#[tokio::test]
async fn a_protocol_4_server_is_refused_no_rescan_packet() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.server_protocol_version(4);
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");

    let failed = adapter.rescan().await.expect_err("protocol 4 has no rescan packet");
    assert!(matches!(failed, AdapterError::Unsupported(_)), "{failed:?}");
    assert_eq!(server.rescan_count(), 0, "a protocol 4 server got a packet 140");
    assert!(adapter.is_connected());
}

/// A detection broadcast (101/103/100) is never taken for the reply to a request that is still
/// waiting. The request must fail on its own timeout and drop the link, not be answered early.
#[tokio::test]
async fn a_detection_packet_is_not_taken_for_a_reply() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("first discover fills the controller cache");
    let id = DeviceId("openrgb:one".into());

    // OpenRGB goes silent on requests, so the read-back will never get its own reply.
    server.stall_replies(true);
    let mut asking = adapter.clone();
    let asking_id = id.clone();
    let ask = tokio::spawn(async move { asking.read_back(&asking_id).await });
    until("the request is waiting for its reply", || server.swallowed_requests() == 1).await;

    // A rescan broadcasts detection packets while that request is still unanswered.
    let mut scanner = adapter.clone();
    scanner.rescan().await.expect("rescan");
    until("packet 140 went out", || server.rescan_count() == 1).await;

    // The broadcasts did not answer the request: it still fails on its own reply timeout.
    let failed = tokio::time::timeout(Duration::from_secs(60), ask)
        .await
        .expect("the request ends")
        .expect("its task ends");
    assert!(failed.is_err(), "a detection packet was taken for the request's reply");
    server.stall_replies(false);
}
