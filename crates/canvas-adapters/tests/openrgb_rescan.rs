//! #290: `DeviceAdapter::rescan` against an OpenRGB that speaks protocol 5, an older one.
//!
//! Every server in this file is a fake that negotiates 5 (`v5_server`): the shipped OpenRGB
//! (release_1.1) speaks 6, and `openrgb_protocol6.rs` is about that. Packet 140 is client-only and
//! gets no reply, so it goes out as a fire-and-forget write. The fake models what OpenRGB
//! release_1.1 does to a protocol 5 client: the list is cleared and then re-registered as list
//! changes (100) and no 101 or 103, which that release sends only to a protocol 6 client
//! (`NetworkServer.cpp:4003-4006`, `:3949-3952`), and it drops a packet 140 that
//! arrives while a detection is already running (`DetectionManager.cpp:1753-1756`). So on this
//! protocol the adapter does not guess when detection ends: a list that is changing is never
//! applied, and a list that has settled (non-empty, quiet) is. Everything runs against the fake OpenRGB server on loopback:
//! no real hardware. Each wait is on state the assert reads; the long limit only ends a test that
//! would hang.

use canvas_adapters::fake::{DeviceEvent, FakeOpenRgbServer};
use canvas_adapters::openrgb::{rescan_miss, OpenRgbAdapter, RescanMiss, RescanTiming};
use canvas_proto::{AdapterError, DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8, ZoneInfo, ZoneKind};
use std::time::Duration;

/// Short windows so the tests run in a second or two. The fake's gap between two controllers
/// registering is 10 ms by default, far inside `settle`.
fn quick() -> RescanTiming {
    RescanTiming { first: Duration::from_millis(500), settle: Duration::from_millis(250), cap: Duration::from_secs(3) }
}

/// A fake OpenRGB that negotiates protocol 5.
async fn v5_server(devices: Vec<DeviceInfo>) -> FakeOpenRgbServer {
    let server = FakeOpenRgbServer::start(devices).await.expect("fake server");
    server.server_protocol_version(5);
    server
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
    let server = v5_server(vec![stick("one")]).await;
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
    let server = v5_server(vec![stick("one")]).await;
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
    let server = v5_server(vec![stick("one"), stick("two"), stick("three")]).await;
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

/// OpenRGB with detection off answers packet 140 with nothing: no list change follows. The rescan
/// says so after the first window, and the list was not touched.
#[tokio::test]
async fn no_list_change_after_packet_140_means_detection_did_not_start() {
    let server = v5_server(vec![stick("one")]).await;
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
/// within the cap, with the link up. OpenRGB's list is empty meanwhile, so the adapter answers every
/// read with the list it holds: an empty list is never the answer while the list is changing.
#[tokio::test]
async fn a_detection_that_never_settles_is_reported_at_the_cap_with_the_link_up() {
    let server = v5_server(vec![stick("one")]).await;
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
    // The server's list is empty while detection is stuck; the adapter still answers with the one
    // it holds.
    assert_eq!(adapter.discover().await.expect("discover on a live link").len(), 1, "the empty list replaced the held one");
}

/// A rescan that re-detected resets the custom-mode cache: the controllers OpenRGB registered
/// again sit in their built-in mode, so the next frame to each sends SET_CUSTOM_MODE first.
#[tokio::test]
async fn a_rescan_that_re_detected_makes_the_next_frame_set_the_custom_mode_again() {
    let server = v5_server(vec![stick("one")]).await;
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
    let server = v5_server(vec![stick("one")]).await;
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
    let server = v5_server(vec![stick("one")]).await;
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
    let server = v5_server(vec![stick("one")]).await;
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

    // A rescan broadcasts detection packets while that request is still unanswered. It is not
    // awaited: settling needs a read of the list, and OpenRGB answers no request meanwhile.
    let scanner = adapter.clone();
    let rescanning = tokio::spawn(async move { scanner.rescan_shared().await });
    until("packet 140 went out", || server.rescan_count() == 1).await;

    // The broadcasts did not answer the request: it still fails on its own reply timeout.
    let failed = tokio::time::timeout(Duration::from_secs(60), ask)
        .await
        .expect("the request ends")
        .expect("its task ends");
    assert!(failed.is_err(), "a detection packet was taken for the request's reply");
    server.stall_replies(false);
    rescanning.abort();
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
    let server = v5_server(vec![stick("one")]).await;
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
    let server = v5_server(vec![stick("one"), stick("two")]).await;
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
    let server = v5_server(vec![stick("one")]).await;
    // The first controller would register 20 s after the clear: past the 3 s cap.
    server.rescan_timing(Duration::from_secs(20), Duration::from_millis(10));
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("first discover");

    let failed = adapter.rescan_shared().await.expect_err("no controller came back");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::Unsettled), "{failed:?}");
    assert!(adapter.is_connected());
}

/// OpenRGB deletes every controller at the clear and registers them again one at a time, and
/// protocol 5 names a controller by its position, so a frame written meanwhile lands on whichever
/// device holds that position at that moment. While the list is changing no frame is written to any
/// OpenRGB device, and a read of the list is answered with the list held (never the empty or part
/// list OpenRGB has meanwhile): dark lights for a few seconds are better than colours on the wrong
/// device.
#[tokio::test]
async fn no_frame_is_written_and_the_held_list_stays_while_openrgbs_list_is_changing() {
    let server = v5_server(vec![stick("one"), stick("two")]).await;
    server.hold_detection(true);
    let quick = RescanTiming { first: Duration::from_millis(300), settle: Duration::from_millis(200), cap: Duration::from_millis(600) };
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port()).with_rescan_timing(quick);
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("first discover").len(), 2);

    let failed = adapter.rescan_shared().await.expect_err("the detection never settles");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::Unsettled), "{failed:?}");
    // The daemon's refresh reads the list: OpenRGB has none yet, and the answer is the held list.
    assert_eq!(adapter.discover().await.expect("the refresh read").len(), 2, "the refresh got a list that was mid-detection");

    let one = DeviceId("openrgb:one".into());
    let two = DeviceId("openrgb:two".into());
    for _ in 0..5 {
        adapter.write_frame(&one, &[Rgb8::WHITE; 8]).await.expect_err("a frame for the held controller one was written");
        adapter.write_frame(&two, &[Rgb8::WHITE; 8]).await.expect_err("a frame for the held controller two was written");
    }
    assert_eq!(server.update_leds_writes(0) + server.update_leds_writes(1), 0, "a frame reached OpenRGB while its list was changing");
    assert_eq!(server.frames_to_unregistered(), 0, "a frame was sent to an index OpenRGB had not registered");
    assert!(adapter.is_connected());
}

/// A detection that OpenRGB started by itself is already running when packet 140 goes out, and
/// release_1.1 drops that packet without a word (`DetectionManager.cpp:1753-1756`). The list it had
/// cleared is empty when the rescan reads it. The held list stays the answer: a wait that took "no
/// change after packet 140" for "OpenRGB did not look" handed the empty list over, and every device
/// was gone from the daemon.
#[tokio::test]
async fn an_ignored_packet_140_during_a_running_detection_keeps_the_held_list_when_the_list_is_empty() {
    let server = v5_server(vec![stick("one"), stick("two")]).await;
    // The detection is in its first minute: the list is empty and stays so.
    server.rescan_timing(Duration::from_secs(60), Duration::from_millis(10));
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("first discover").len(), 2);

    server.start_detection();
    let failed = adapter.rescan_shared().await.expect_err("the detection is still running");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::Unsettled), "{failed:?}");
    assert_eq!(server.rescan_dropped(), 1, "the fake must drop a 140 sent during a detection");
    assert_eq!(adapter.discover().await.expect("discover").len(), 2, "the empty list replaced the held one");
    adapter.write_frame(&DeviceId("openrgb:one".into()), &[Rgb8::WHITE; 8]).await.expect_err("a frame was written while the list was empty");
    assert!(server.reads_during_detection() >= 1, "the list was never read, so the empty list was never offered");
    assert!(adapter.is_connected());
}

/// A detection longer than the call's wait is not a failure: the call answers that detection is
/// still running, the held list stays, and the list is applied by itself once it has settled. There
/// is no cap on the hold.
#[tokio::test]
async fn a_detection_longer_than_the_wait_is_applied_when_it_settles() {
    let server = v5_server(vec![stick("one")]).await;
    server.add_hidden_device(stick("two"));
    // The list is empty for 1.5 s: well past the 1 s the call waits.
    server.rescan_timing(Duration::from_millis(1500), Duration::from_millis(50));
    let quick = RescanTiming { first: Duration::from_millis(300), settle: Duration::from_millis(200), cap: Duration::from_secs(1) };
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port()).with_rescan_timing(quick);
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("first discover").len(), 1);

    let failed = adapter.rescan_shared().await.expect_err("detection is still running at the cap");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::Unsettled), "{failed:?}");
    assert_eq!(adapter.discover().await.expect("discover mid-detection").len(), 1, "the held list stays");

    // The detection ends by itself, and the refresh that follows finds the new list.
    let two = DeviceId("openrgb:two".into());
    let found = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let found = adapter.discover().await.expect("discover");
            if found.len() == 2 {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the settled list was never applied");
    assert_eq!(found.len(), 2);
    adapter.write_frame(&two, &[Rgb8::WHITE; 8]).await.expect("a frame for the device the detection found");
}

/// Two identical sticks that register again in the other order swap indices, and protocol 5 names a
/// controller by its position: a frame for stick a that went to a's old index would light stick b.
/// While the list is unsettled no frame lands anywhere; once it has settled, the frames go to the
/// indices the sticks have now.
#[tokio::test]
async fn two_identical_sticks_that_register_in_the_other_order_never_get_each_others_frames() {
    fn twin(serial: &str) -> DeviceInfo {
        DeviceInfo { name: "DDR5 stick".into(), ..stick(serial) }
    }
    let server = v5_server(vec![twin("a"), twin("b")]).await;
    // Each stick registers again 100 ms after the one before: inside the 250 ms settle window.
    server.rescan_timing(Duration::from_millis(100), Duration::from_millis(100));
    server.rescan_order(vec![1, 0]);
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("first discover").len(), 2);
    let a = DeviceId("openrgb:a".into());
    let b = DeviceId("openrgb:b".into());
    let red = [Rgb8 { r: 255, g: 0, b: 0 }; 8];
    let blue = [Rgb8 { r: 0, g: 0, b: 255 }; 8];
    adapter.write_frame(&a, &red).await.expect("a frame for a");
    until("a's frame landed", || !server.landings().is_empty()).await;
    assert_eq!(server.landings()[0].serial, "a");

    let scanner = adapter.clone();
    let rescan = tokio::spawn(async move { scanner.rescan_shared().await });
    while !rescan.is_finished() {
        // Every frame is tried: the unsettled list refuses it, and the settled list takes it.
        let _ = adapter.write_frame(&a, &red).await;
        let _ = adapter.write_frame(&b, &blue).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    rescan.await.expect("the task ends").expect("the detection settled");
    for landing in server.landings() {
        let wanted = if landing.serial == "a" { red[0] } else { blue[0] };
        assert_eq!(landing.color, wanted, "a frame meant for the other stick landed on {landing:?}");
    }

    // The sticks swapped indices for good: the frames go to where they are now.
    adapter.discover().await.expect("discover after the rescan");
    let before = server.landings().len();
    adapter.write_frame(&a, &red).await.expect("a frame for a after the rescan");
    adapter.write_frame(&b, &blue).await.expect("a frame for b after the rescan");
    until("both frames landed", || server.landings().len() >= before + 2).await;
    let landed = server.landings();
    let (to_a, to_b) = (&landed[before], &landed[before + 1]);
    assert_eq!((to_a.serial.as_str(), to_a.index, to_a.color), ("a", 1, red[0]), "{to_a:?}");
    assert_eq!((to_b.serial.as_str(), to_b.index, to_b.color), ("b", 0, blue[0]), "{to_b:?}");
}

/// Once the detection is settled the list is the new one, even a shorter one: a device unplugged
/// between two rescans is gone, and the frames must not go on to its old index.
#[tokio::test]
async fn a_settled_shorter_list_replaces_the_one_frames_are_written_to() {
    let server = v5_server(vec![stick("one"), stick("two")]).await;
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

/// A controller that registers again is a new object in OpenRGB, in its built-in mode: once the
/// list has settled, the first frame to it must say SET_CUSTOM_MODE, or its lights stay on the
/// built-in effect. No frame is written during the wait (the list is changing), and every frame
/// after it lands.
///
/// The verdict is what the new object was sent, in the order the fake saw it (#499), read once the
/// frame after the wait has reached it. It is not a count read at some moment: the frames before the
/// detection leave the colours white and the mode count at 1 on the old object, so neither says that
/// this frame arrived, and a count read right after `write_frame` returns can come before the fake
/// has read the packet. Which task settles the list (the loop's frame or the rescan) and how many
/// frames the loop sends after it change nothing the assert reads.
#[tokio::test]
async fn frames_resume_with_the_custom_mode_set_again_once_the_list_has_settled() {
    let server = v5_server(vec![stick("one"), stick("two")]).await;
    // The list is empty for 300 ms, then a controller registers every 100 ms (inside the 250 ms
    // settle window).
    server.rescan_timing(Duration::from_millis(300), Duration::from_millis(100));
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("first discover");
    let one = DeviceId("openrgb:one".into());
    adapter.write_frame(&one, &[Rgb8::BLACK; 8]).await.expect("first frame");
    until("the first mode landed", || server.custom_mode_writes(0) == 1).await;

    let scanner = adapter.clone();
    let rescan = tokio::spawn(async move { scanner.rescan_shared().await });
    let mut refused = 0_u32;
    while !rescan.is_finished() {
        if adapter.write_frame(&one, &[Rgb8::WHITE; 8]).await.is_err() {
            refused += 1;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let settled = rescan.await.expect("the task ends");
    settled.expect("the detection settled");
    // The wait is 300 ms of an empty list and the loop tries every 5 ms; one refusal is enough to show
    // it ran while the list was changing (a starved machine runs it far fewer times than 10).
    assert!(refused > 0, "the loop did not run during the wait: {refused} frames refused");
    assert_eq!(server.frames_to_unregistered(), 0, "a frame was sent to a controller OpenRGB had not registered");

    adapter.write_frame(&one, &[Rgb8::WHITE; 8]).await.expect("a frame after the list settled");
    let told = server.wait_for_events(0, |events| events.contains(&DeviceEvent::UpdateLeds)).await;
    assert_eq!(
        told.get(..2),
        Some([DeviceEvent::SetCustomMode, DeviceEvent::UpdateLeds].as_slice()),
        "the controller that registered again was not put in custom mode before its first frame: {told:?}"
    );
    assert_eq!(
        told.iter().filter(|event| **event == DeviceEvent::SetCustomMode).count(),
        1,
        "the mode was set more than once on the new object: {told:?}"
    );
}
