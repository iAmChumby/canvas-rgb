//! #290: `DeviceAdapter::rescan` asks OpenRGB to look at the hardware again.
//!
//! Packet 140 is client-only and gets no reply, so it goes out as a fire-and-forget write. The
//! fake server models OpenRGB release_1.1: a protocol 5 client sees the list cleared and then
//! re-registered as list changes (100) and no 101 or 103, which that release sends only to a
//! protocol 6 client (`NetworkServer.cpp:4003-4006`, `:3949-3952`). So a rescan is over when the
//! list has been quiet for a settle window. Everything runs against the fake OpenRGB server on
//! loopback: no real hardware. Each wait is on state the assert reads; the long limit only ends a
//! test that would hang.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::{rescan_miss, OpenRgbAdapter, RescanMiss, RescanTiming};
use canvas_proto::{AdapterError, DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8, ZoneInfo, ZoneKind};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Short windows so the tests run in a second or two. The fake's gap between two controllers
/// registering is 10 ms by default, far inside `settle`.
fn quick() -> RescanTiming {
    RescanTiming { first: Duration::from_millis(500), settle: Duration::from_millis(250), cap: Duration::from_secs(3) }
}

fn adapter(server: &FakeOpenRgbServer) -> OpenRgbAdapter {
    OpenRgbAdapter::new("127.0.0.1", server.port()).with_rescan_timing(quick())
}

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
    let mut adapter = adapter(&server);
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
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");

    let before = adapter.discover().await.expect("first discover");
    assert_eq!(before.len(), 1, "the hidden device is not announced yet");

    adapter.rescan().await.expect("rescan");
    let after = adapter.discover().await.expect("discover after the rescan");
    let ids: Vec<_> = after.iter().map(|d| d.id.0.as_str()).collect();
    assert_eq!(ids, ["openrgb:one", "openrgb:two"]);
}

/// OpenRGB release_1.1 answers packet 140 for a protocol 5 client with a list that is cleared and
/// then re-registered one controller at a time, each a list change (100). The rescan must wait the
/// burst out: ending on the first change would read the list while it is empty.
#[tokio::test]
async fn a_rescan_waits_out_the_burst_of_list_changes_and_the_list_is_whole() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two"), stick("three")]).await.expect("fake server");
    // The list is empty for 120 ms, then a controller registers every 40 ms.
    server.rescan_timing(Duration::from_millis(120), Duration::from_millis(40));
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("first discover").len(), 3);

    let done = adapter.rescan_shared().await.expect("the detection went quiet");
    assert_eq!(done.list_changes, 4, "one change when the list was cleared and one per controller");
    assert!(
        done.settled_after >= Duration::from_millis(120 + 40 * 2 + 250),
        "the wait ended before the list was quiet: {:?}",
        done.settled_after
    );
    let after = adapter.discover().await.expect("discover after the rescan");
    assert_eq!(after.len(), 3, "the list was read while detection was still re-registering controllers");
}

/// A protocol 5 client sees no 101 and no 103 (release_1.1 sends them to protocol 6 and later
/// only); a protocol 6 client sees 101 first and 103 last. The fake is what the rescan wait is
/// proven against, so it has to say what the real server says.
#[tokio::test]
async fn the_fake_sends_detection_started_and_completed_only_to_protocol_6() {
    async fn broadcast_ids(server: &FakeOpenRgbServer, version: u32) -> Vec<u32> {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", server.port())).await.expect("connect");
        let packet = |device: u32, id: u32, payload: &[u8]| {
            let mut bytes = b"ORGB".to_vec();
            for word in [device, id, payload.len() as u32] {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            bytes.extend_from_slice(payload);
            bytes
        };
        stream.write_all(&packet(0, 40, &version.to_le_bytes())).await.expect("handshake");
        let mut header = [0_u8; 16];
        stream.read_exact(&mut header).await.expect("the handshake reply");
        let mut version_payload = [0_u8; 4];
        stream.read_exact(&mut version_payload).await.expect("the handshake reply's version");
        stream.write_all(&packet(0, 140, &[])).await.expect("rescan");
        let mut ids = Vec::new();
        // Read until the stream has been quiet for a while: the whole detection.
        while let Ok(Ok(_)) = tokio::time::timeout(Duration::from_millis(400), stream.read_exact(&mut header)).await {
            ids.push(u32::from_le_bytes([header[8], header[9], header[10], header[11]]));
        }
        ids
    }
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    server.server_protocol_version(6);
    let v5 = broadcast_ids(&server, 5).await;
    assert_eq!(v5, [100, 100, 100], "a protocol 5 client: the list cleared, then one change per controller");
    let v6 = broadcast_ids(&server, 6).await;
    assert_eq!(v6, [101, 100, 100, 100, 103], "a protocol 6 client also gets 101 first and 103 last");
}

/// OpenRGB with detection off answers packet 140 with nothing: no list change follows. The rescan
/// says so after the first window, and the list was not touched.
#[tokio::test]
async fn no_list_change_after_packet_140_means_detection_did_not_start() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.disable_detection(true);
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("first discover");

    let started = std::time::Instant::now();
    let failed = adapter.rescan_shared().await.expect_err("detection never started");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::NotRedetected), "{failed:?}");
    assert!(started.elapsed() < Duration::from_secs(2), "waited past the first window: {:?}", started.elapsed());
    assert_eq!(server.rescan_count(), 1, "packet 140 still went out");
    assert!(adapter.is_connected());
    assert_eq!(adapter.discover().await.expect("discover").len(), 1, "the list was not touched");
}

/// A detection that keeps announcing list changes past the cap is unsettled: the call says so
/// within the cap, with the link up. The list is empty meanwhile, which is why the caller must not
/// read it.
#[tokio::test]
async fn a_detection_that_never_settles_is_reported_at_the_cap_with_the_link_up() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.hold_detection(true);
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("first discover");

    let started = std::time::Instant::now();
    let failed = adapter.rescan_shared().await.expect_err("detection never went quiet");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::Unsettled), "{failed:?}");
    let took = started.elapsed();
    assert!(took >= Duration::from_secs(3) && took < Duration::from_secs(5), "the cap is 3 s: {took:?}");
    assert!(adapter.is_connected(), "a detection timeout broke the link");
    assert_eq!(server.rescan_count(), 1, "packet 140 went out");
    // The server's list is empty while detection is stuck: a read now would hand back nothing.
    assert!(adapter.discover().await.expect("discover on a live link").is_empty());
}

/// A rescan that re-detected resets the custom-mode cache: the controllers OpenRGB registered
/// again sit in their built-in mode, so the next frame to each sends SET_CUSTOM_MODE first.
#[tokio::test]
async fn a_rescan_that_re_detected_makes_the_next_frame_set_the_custom_mode_again() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("discover");
    let id = DeviceId("openrgb:one".into());
    adapter.write_frame(&id, &[Rgb8::WHITE; 8]).await.expect("first frame");
    until("the first frame's mode landed", || server.custom_mode_writes(0) == 1).await;

    adapter.rescan_shared().await.expect("re-detected");
    adapter.discover().await.expect("discover after the rescan");
    adapter.write_frame(&id, &[Rgb8::WHITE; 8]).await.expect("frame after the rescan");
    until("the mode was set again", || server.custom_mode_writes(0) == 2).await;
}

/// A rescan that found OpenRGB's detection off sent nothing and changed nothing, so it leaves the
/// custom-mode cache alone: no frame sets the mode again for no reason.
#[tokio::test]
async fn a_rescan_that_re_detected_nothing_leaves_the_custom_mode_cache_alone() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.disable_detection(true);
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("discover");
    let id = DeviceId("openrgb:one".into());
    adapter.write_frame(&id, &[Rgb8::WHITE; 8]).await.expect("first frame");
    until("the first frame's mode landed", || server.custom_mode_writes(0) == 1).await;

    adapter.rescan_shared().await.expect_err("detection never started");
    adapter.write_frame(&id, &[Rgb8::WHITE; 8]).await.expect("frame after the rescan");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.custom_mode_writes(0), 1, "the mode was set again for no reason");
}

/// An OpenRGB that only speaks protocol 4 has no rescan packet: the call is refused as unsupported
/// and nothing is written to the wire.
#[tokio::test]
async fn a_protocol_4_server_is_refused_no_rescan_packet() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.server_protocol_version(4);
    let mut adapter = adapter(&server);
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
    let mut adapter = adapter(&server);
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

// ---- #290 review 2: the wait is for the list to come back, not for the clear to go quiet

/// The list is cleared at once and the first controller registers seconds later: the I2C and DRAM
/// detectors run before it (`DetectionManager.cpp:810-856`, the DRAM scan at `:1030-1100`), and
/// the first `RegisterRGBController` is what puts a controller back (`:452-491`). A wait that
/// treated the silence after the clear as "settled" read an empty list and answered with the old
/// device only. This test runs at the production windows, with the first controller 2.5 s after the
/// clear and the new device 400 ms after it.
#[tokio::test]
async fn a_rescan_at_the_production_windows_returns_the_whole_new_list() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    server.add_hidden_device(stick("two"));
    server.rescan_timing(Duration::from_millis(2500), Duration::from_millis(400));
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    assert_eq!(adapter.rescan_timing(), RescanTiming::default());
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("first discover").len(), 1);

    let done = adapter.rescan_shared().await.expect("the detection settled");
    assert_eq!(done.list_changes, 3, "the clear, then one change for each of the two controllers");
    assert!(
        done.settled_after >= Duration::from_millis(2500 + 400),
        "the wait ended before the second controller registered: {:?}",
        done.settled_after
    );
    let after = adapter.discover().await.expect("discover after the rescan");
    let ids: Vec<_> = after.iter().map(|d| d.id.0.as_str()).collect();
    assert_eq!(ids, ["openrgb:one", "openrgb:two"], "the rescan answered with a list that was mid-detection");
}

/// The same with the short windows: the quiet window starts counting only once a controller has
/// registered again, however long the list stays empty after the clear.
#[tokio::test]
async fn the_quiet_window_does_not_start_while_the_list_is_still_empty_after_the_clear() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    // The list is empty for 1.2 s (four quiet windows), then each controller registers 100 ms on.
    server.rescan_timing(Duration::from_millis(1200), Duration::from_millis(100));
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("first discover");

    let done = adapter.rescan_shared().await.expect("the detection settled");
    assert_eq!(done.list_changes, 3);
    assert_eq!(adapter.discover().await.expect("discover after the rescan").len(), 2);
}

/// A detection that cleared the list and never put a controller back is unsettled at the cap, not
/// settled after a quiet window: with protocol 5 there is no 103 to say it is over, and an empty
/// list is what a detection that has not found its first controller yet looks like.
#[tokio::test]
async fn a_list_that_stays_empty_after_the_clear_is_unsettled_at_the_cap() {
    let server = FakeOpenRgbServer::start(vec![stick("one")]).await.expect("fake server");
    // The first controller would register 20 s after the clear: past the 3 s cap.
    server.rescan_timing(Duration::from_secs(20), Duration::from_millis(10));
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("first discover");

    let failed = adapter.rescan_shared().await.expect_err("no controller came back");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::Unsettled), "{failed:?}");
    assert!(adapter.is_connected());
}

/// The lights of a rescan: OpenRGB deletes every controller at the clear and answers a frame for a
/// dead id with nothing (the link stays up), so frames keep going to the index the client held. The
/// adapter's list of controllers must not be replaced by the empty or part list a read mid-detection
/// returns (the daemon's 5 s refresh makes that read), or every following frame re-reads the list
/// and is dropped, and the lights go dark for as long as detection takes.
#[tokio::test]
async fn frames_keep_going_to_the_held_controllers_after_a_read_mid_detection() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    server.hold_detection(true);
    let quick = RescanTiming { first: Duration::from_millis(300), settle: Duration::from_millis(200), cap: Duration::from_millis(600) };
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port()).with_rescan_timing(quick);
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("first discover").len(), 2);

    let failed = adapter.rescan_shared().await.expect_err("the detection never settles");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::Unsettled), "{failed:?}");
    // The daemon's refresh reads the list: OpenRGB has none yet.
    assert!(adapter.discover().await.expect("the refresh read").is_empty());
    let reads = server.reads_during_detection();

    let one = DeviceId("openrgb:one".into());
    let two = DeviceId("openrgb:two".into());
    for _ in 0..5 {
        adapter.write_frame(&one, &[Rgb8::WHITE; 8]).await.expect("a frame for the held controller one");
        adapter.write_frame(&two, &[Rgb8::WHITE; 8]).await.expect("a frame for the held controller two");
    }
    until("the frames reached the server", || server.frames_to_unregistered() >= 10).await;
    assert_eq!(server.reads_during_detection(), reads, "a frame read the list while OpenRGB was mid-detection");
    assert!(adapter.is_connected());
}

/// Once the detection is settled the list is the new one, even a shorter one: a device unplugged
/// between two rescans is gone, and the frames must not go on to its old index.
#[tokio::test]
async fn a_settled_shorter_list_replaces_the_one_frames_are_written_to() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("first discover").len(), 2);
    // Device two is unplugged: detection registers only the first controller again.
    server.unplug_device(1);

    adapter.rescan_shared().await.expect("the detection settled");
    let after = adapter.discover().await.expect("discover after the rescan");
    assert_eq!(after.len(), 1);
    let gone = adapter.write_frame(&DeviceId("openrgb:two".into()), &[Rgb8::WHITE; 8]).await;
    assert!(matches!(gone, Err(AdapterError::UnknownDevice(_))), "{gone:?}");
}

/// A controller that registers again during the wait is a new object in OpenRGB, in its built-in
/// mode: the next frame to it must say SET_CUSTOM_MODE, or its lights stay on the built-in effect
/// until the wait is over. Frames flow all through the wait, none are re-reads of the list, and
/// every one that names a registered controller lands.
#[tokio::test]
async fn a_controller_that_registers_again_during_the_wait_gets_its_mode_and_its_frames() {
    let server = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    server.rescan_timing(Duration::from_millis(300), Duration::from_millis(400));
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("first discover");
    let one = DeviceId("openrgb:one".into());
    adapter.write_frame(&one, &[Rgb8::BLACK; 8]).await.expect("first frame");
    until("the first mode landed", || server.custom_mode_writes(0) == 1).await;

    let scanner = adapter.clone();
    let rescan = tokio::spawn(async move { scanner.rescan_shared().await });
    let mut sent = 0_u32;
    while !rescan.is_finished() {
        adapter.write_frame(&one, &[Rgb8::WHITE; 8]).await.expect("a frame during the wait");
        sent += 1;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let modes_when_the_wait_ended = server.custom_mode_writes(0);
    rescan.await.expect("the task ends").expect("the detection settled");
    assert!(sent > 10, "the loop did not run during the wait: {sent} frames");
    assert_eq!(server.reads_during_detection(), 0, "a frame read the list while OpenRGB was mid-detection");
    assert!(server.frames_to_unregistered() > 0, "no frame went to the controller while it was dead");
    assert_eq!(
        server.colors(0),
        vec![Rgb8::WHITE; 8],
        "the frames after the controller registered again did not land"
    );
    assert!(
        modes_when_the_wait_ended >= 2,
        "the controller that registered again was never put in custom mode during the wait: {modes_when_the_wait_ended}"
    );
}
