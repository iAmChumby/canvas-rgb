//! A request the caller gave up on (the daemon's rescan budget drops one) must leave the link
//! clean: OpenRGB answers in order and puts no request id in a reply, so the late reply of a
//! dropped request must not answer the next request.
//!
//! The fake OpenRGB server keeps replies back on command (`hold_replies`), so the test decides
//! when a reply is late. Every wait is on the state the assert reads; the long limit only ends a
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

async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
}

/// The late reply of a request that was given up on must not answer the next request.
///
/// The first discovery is dropped with its reply still owed: that is what the daemon's rescan
/// budget does to a request that is still waiting. OpenRGB answers in order and puts no request
/// id in a reply, so the late reply of the dropped request carries the old device count. The
/// next discovery must read the devices again and get its own answer.
#[tokio::test]
async fn the_late_reply_of_a_dropped_request_does_not_answer_the_next_one() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("a plain discover").len(), 1);

    // A discovery starts; OpenRGB is slow, so its reply is kept back. The caller gives up on it
    // (the rescan budget ran out) by dropping the future.
    server.hold_replies(true);
    let mut first = adapter.clone();
    tokio::select! {
        _ = first.discover() => panic!("the reply was held, so the discovery cannot have finished"),
        () = until("the first reply is being held", || server.held_replies() == 1) => {}
    }
    // The select dropped the discovery: the request went out and its reply is owed.

    // OpenRGB finds a second stick, and the next discovery starts while the first reply is still
    // owed. Its own reply is held too, so the late one is the first thing to arrive.
    server.announce(stick("two"));
    let mut second = adapter.clone();
    let next = tokio::spawn(async move { second.discover().await });
    // The next request goes out on its own, unless the link waits for the owed reply first: the
    // server holds that reply too, so the count says the request is out. The wait ends a test
    // that would hang; it is not the verdict, which is the device list below.
    let _ = tokio::time::timeout(
        Duration::from_millis(500),
        until("the next request is out", || server.held_replies() == 2),
    )
    .await;

    // The slow OpenRGB catches up: the late reply (one stick), then answers on time again.
    server.hold_replies(false);
    server.release_replies();
    let found = tokio::time::timeout(Duration::from_secs(30), next)
        .await
        .expect("the second discovery ends")
        .expect("its task ends")
        .expect("the second discovery works");
    let ids: Vec<_> = found.iter().map(|d| d.id.0.as_str()).collect();
    assert_eq!(ids, ["openrgb:one", "openrgb:two"], "the next request got its own answer");
}

/// A request whose reply timed out does not make the next request wait for that reply.
///
/// The reply of a request that timed out may never come: OpenRGB went silent. The next
/// request used to wait out the reply timeout for it, so a rescan spent its whole budget
/// waiting and never went out. The next request goes out at once, and it still gets its own
/// answer: the late reply of the abandoned request is dropped by the reader.
#[tokio::test]
async fn a_request_that_timed_out_does_not_make_the_next_one_wait() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("a plain discover");

    // The server keeps the reply back: the request waits for it, and gives up on it when the
    // adapter's own reply timeout passes.
    server.hold_replies(true);
    let id = DeviceId("openrgb:one".into());
    let mut asking = adapter.clone();
    let first_id = id.clone();
    let first = tokio::spawn(async move { asking.read_back(&first_id).await });
    until("the request is waiting for its reply", || server.held_replies() == 1).await;
    let failed = tokio::time::timeout(Duration::from_secs(60), first)
        .await
        .expect("the request ends")
        .expect("its task ends");
    assert!(failed.is_err(), "the request answered anyway");

    // The colours the next read must see: the frame lands while the reply is still held.
    adapter.write_frame_shared(&id, &[Rgb8::WHITE; 8]).await.expect("the frame");
    until("the frame reached the lights", || server.colors(0) == vec![Rgb8::WHITE; 8]).await;

    // The next request goes out at once: it does not wait for the late reply.
    let mut next = adapter.clone();
    let next_id = id.clone();
    let second = tokio::spawn(async move { next.read_back(&next_id).await });
    until("the next request is on the wire", || server.held_replies() == 2).await;

    // The slow OpenRGB catches up: the late reply first (it answers nobody), then the reply
    // of the request that is waiting for it.
    server.release_replies();
    let colours = tokio::time::timeout(Duration::from_secs(60), second)
        .await
        .expect("the request ends")
        .expect("its task ends")
        .expect("the request works");
    assert_eq!(colours, vec![Rgb8::WHITE; 8], "the next request got the old answer");
}
