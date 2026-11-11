//! A request that went out and was not answered leaves the link unusable.
//!
//! OpenRGB answers in order and puts no request id in a reply, so a reply that arrives late
//! would answer the next request. The link is dropped instead: the supervisor reconnects, and
//! a fresh connection has no stale replies.
//!
//! The fake OpenRGB server goes silent on command (`stall_replies`), so the test decides when
//! a reply never comes. Every wait is on the state the assert reads; the long limit only ends a
//! test that would hang.

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

/// A request whose reply never comes drops the link, and the next request gets its own answer.
///
/// The request goes out and OpenRGB goes silent: the reply may still come, and it would answer
/// the next request, because OpenRGB answers in order and puts no request id in a reply. The
/// link is unusable, so it is dropped and the supervisor reconnects. On the new connection the
/// next request gets its own answer.
#[tokio::test]
async fn a_request_whose_reply_never_comes_drops_the_link_and_the_next_one_gets_its_own_answer() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("a plain discover");

    let id = DeviceId("openrgb:one".into());
    // The lights are white, so the answer below says which colours came back.
    adapter.write_frame_shared(&id, &[Rgb8::WHITE; 8]).await.expect("the frame");
    until("the frame reached the lights", || server.colors(0) == vec![Rgb8::WHITE; 8]).await;

    // OpenRGB goes silent: the request goes out and no reply ever comes.
    server.stall_replies(true);
    let mut asking = adapter.clone();
    let asking_id = id.clone();
    let request = tokio::spawn(async move { asking.read_back(&asking_id).await });
    until("the request is waiting for a reply that never comes", || server.swallowed_requests() == 1).await;

    // The request gives up on its own reply timeout.
    let failed = tokio::time::timeout(Duration::from_secs(60), request)
        .await
        .expect("the request ends")
        .expect("its task ends");
    assert!(failed.is_err(), "the request answered anyway");

    // The link is unusable: a late reply would answer the next request. It is dropped, and the
    // supervisor reconnects.
    assert!(
        !adapter.is_connected(),
        "the link stayed up after a request that was not answered"
    );

    // OpenRGB answers again, and the supervisor reconnects.
    server.stall_replies(false);
    adapter.disconnect().await.expect("disconnect");
    adapter.connect().await.expect("connect again");

    // The next request goes out on the new link and gets its own answer.
    let mut next = adapter.clone();
    let next_id = id.clone();
    let colours = tokio::time::timeout(Duration::from_secs(30), async move { next.read_back(&next_id).await })
        .await
        .expect("the next request ends")
        .expect("the next request works");
    assert_eq!(colours, vec![Rgb8::WHITE; 8], "the next request got its own answer");
}
