//! #290: a rescan against OpenRGB protocol 6 and later, the protocol the shipped OpenRGB (release_1.1,
//! `installer/openrgb.lock.json`) speaks.
//!
//! Protocol 6 says what protocol 5 made the client guess: 101 when detection starts and 103 when it is
//! over (`NetworkServer.cpp:4003-4015`, `:3949-3961`), an ACK after every packet (`:3562-3580`), and a
//! number for each controller that is never reused (`:682-711`), so a packet for a controller that was
//! re-detected reaches nobody instead of another device. The adapter negotiates 6 with a server that
//! speaks it and keeps its protocol 5 path for one that does not (`openrgb_rescan.rs`).
//!
//! Everything runs against the fake OpenRGB server on loopback, which models release_1.1 at both
//! protocols. The clock is paused where a test spans seconds or minutes: the fake's detection and the
//! adapter's wait both run on it, so a minute-long detection takes no time.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::{rescan_miss, OpenRgbAdapter, RescanMiss};
use canvas_proto::{AdapterError, DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8, ZoneInfo, ZoneKind};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::Instant;

fn stick(serial: &str) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(format!("openrgb:{serial}")),
        name: format!("DDR5 {serial}"),
        vendor: "fake".into(),
        kind: DeviceKind::Dram,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: serial.into(),
        location: format!("smbus {serial}"),
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

/// Two sticks of one model: the same name and vendor, told apart by their serial only.
fn twin(serial: &str) -> DeviceInfo {
    DeviceInfo { name: "DDR5 stick".into(), ..stick(serial) }
}

async fn server(devices: Vec<DeviceInfo>) -> FakeOpenRgbServer {
    let server = FakeOpenRgbServer::start(devices).await.expect("fake server");
    server.server_protocol_version(6);
    server
}

/// The adapter with the daemon's own settings: the production `RescanTiming`.
fn adapter(server: &FakeOpenRgbServer) -> OpenRgbAdapter {
    OpenRgbAdapter::new("127.0.0.1", server.port())
}

async fn connected(server: &FakeOpenRgbServer, count: usize) -> OpenRgbAdapter {
    let mut adapter = adapter(server);
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("first discover").len(), count);
    adapter
}

/// Waits until `ready` is true. The long limit only ends a test that would hang.
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
}

const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };
const BLUE: Rgb8 = Rgb8 { r: 0, g: 0, b: 255 };

// ---- what a client of each protocol hears

/// A client of protocol 6 hears 101, then the list cleared (100), then the server's ACK of the packet
/// 140 that began it (queued after the 101, as the server handles both on one thread), then a list
/// change and a progress report for each controller, and 103 last. A client of protocol 5 hears the
/// list changes only (`NetworkServer.cpp:4003-4006`, `:3949-3952`, `:4017`).
#[tokio::test]
async fn a_protocol_6_client_hears_start_progress_and_end_and_a_protocol_5_client_only_the_list() {
    async fn heard(server: &FakeOpenRgbServer, version: u32) -> Vec<u32> {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", server.port())).await.expect("connect");
        let packet = |device: u32, id: u32, payload: &[u8]| {
            let mut bytes = b"ORGB".to_vec();
            for word in [device, id, payload.len() as u32] {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            bytes.extend_from_slice(payload);
            bytes
        };
        // One packet at a time, payload and all.
        async fn next(stream: &mut tokio::net::TcpStream) -> Option<u32> {
            let mut header = [0_u8; 16];
            tokio::time::timeout(Duration::from_millis(400), stream.read_exact(&mut header)).await.ok()?.ok()?;
            let size = u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as usize;
            let mut payload = vec![0_u8; size];
            stream.read_exact(&mut payload).await.ok()?;
            Some(u32::from_le_bytes([header[8], header[9], header[10], header[11]]))
        }
        stream.write_all(&packet(0, 40, &version.to_le_bytes())).await.expect("handshake");
        // The reply, then at protocol 6 the server's name and the ACK of the request.
        while next(&mut stream).await.is_some() {}
        stream.write_all(&packet(0, 140, &[])).await.expect("rescan");
        let mut ids = Vec::new();
        // Read until the stream has been quiet for a while: the whole detection.
        while let Some(id) = next(&mut stream).await {
            ids.push(id);
        }
        ids
    }
    let server = server(vec![stick("one"), stick("two")]).await;
    let v5 = heard(&server, 5).await;
    assert_eq!(v5, [100, 100, 100], "a protocol 5 client: the list cleared, then one change per controller");
    let v6 = heard(&server, 6).await;
    assert_eq!(
        v6,
        [101, 100, 10, 100, 102, 100, 102, 103],
        "a protocol 6 client: 101, the clear, the ACK of 140, then each controller with its progress, and 103"
    );
}

/// The adapter asks for 6 and uses what the server answers: 6 from release_1.1, 5 from an older OpenRGB.
#[tokio::test]
async fn the_adapter_uses_protocol_6_with_a_server_that_speaks_it_and_5_with_one_that_does_not() {
    let new = server(vec![stick("one"), stick("two")]).await;
    let mut adapter_new = adapter(&new);
    adapter_new.connect().await.expect("connect to release_1.1");
    assert_eq!(adapter_new.discover().await.expect("discover").len(), 2);
    assert_eq!(new.negotiated_version(1), Some(6));

    let old = FakeOpenRgbServer::start(vec![stick("one"), stick("two")]).await.expect("fake server");
    old.server_protocol_version(5);
    let mut adapter_old = adapter(&old);
    adapter_old.connect().await.expect("connect to an older OpenRGB");
    assert_eq!(adapter_old.discover().await.expect("discover").len(), 2);
    assert_eq!(old.negotiated_version(1), Some(5));
    // The older server still re-detects, as it always did.
    adapter_old.rescan_shared().await.expect("a protocol 5 rescan");
    assert_eq!(old.rescan_count(), 1);
    assert_eq!(adapter_old.discover().await.expect("discover after").len(), 2);
}

// ---- a rescan ends on 103, whatever the gaps between controllers

/// The review's case: two controllers of one detection register further apart than the 2 s quiet window
/// the protocol 5 path waits. With 103 there is no window: the list is not applied while the detection
/// runs, however long the gaps, and it is whole when 103 arrives.
#[tokio::test(start_paused = true)]
async fn a_gap_longer_than_two_seconds_between_registrations_never_shrinks_the_list() {
    let server = server(vec![stick("one"), stick("two"), stick("three")]).await;
    // Empty for 100 ms, then a controller every 3 s: nine seconds in all, past the 2 s window.
    server.rescan_timing(Duration::from_millis(100), Duration::from_secs(3));
    let adapter = connected(&server, 3).await;

    let scanner = adapter.clone();
    let rescan = tokio::spawn(async move { scanner.rescan_shared().await });
    let mut reader = adapter.clone();
    let mut seen = Vec::new();
    while !rescan.is_finished() {
        seen.push(reader.discover().await.expect("a read mid-detection").len());
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    rescan.await.expect("the task ends").expect("the detection ended");
    assert!(seen.len() > 10, "the loop did not run during the detection: {seen:?}");
    assert!(seen.iter().all(|&count| count == 3), "a read mid-detection answered with a part list: {seen:?}");
    assert_eq!(reader.discover().await.expect("a read after 103").len(), 3, "the list after 103 is whole");
}

/// A detection that runs a minute. The call answers "still running" at its bound (15 s) and the held
/// list stays; frames to the held ids keep going (the server drops them, none reaches a device); and
/// the list the detection ends with is applied once, when 103 arrives.
#[tokio::test(start_paused = true)]
async fn a_detection_of_a_minute_keeps_the_held_list_and_applies_the_new_one_after_103() {
    let server = server(vec![stick("one"), stick("two"), stick("three")]).await;
    server.add_hidden_device(stick("four"));
    // 100 ms, then a controller every 15 s: a minute and a bit for the four of them.
    server.rescan_timing(Duration::from_millis(100), Duration::from_secs(15));
    let mut adapter = connected(&server, 3).await;
    let one = DeviceId("openrgb:one".into());

    let started = Instant::now();
    let failed = adapter.rescan_shared().await.expect_err("the detection is still running at the bound");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::Unsettled), "{failed:?}");
    assert!(
        started.elapsed() >= Duration::from_secs(15) && started.elapsed() < Duration::from_secs(16),
        "the bound is 15 s and the call's bound only: {:?}",
        started.elapsed()
    );
    assert!(adapter.is_connected(), "the bound broke the link");

    let mut counts = Vec::new();
    let mut frames = 0_u32;
    while started.elapsed() < Duration::from_secs(70) {
        counts.push(adapter.discover().await.expect("a read").len());
        adapter.write_frame(&one, &[RED; 8]).await.expect("a frame for a held id is sent");
        frames += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert_eq!(counts[0], 3, "the held list answers while the detection runs");
    assert_eq!(*counts.last().expect("reads"), 4, "the list the detection ended with was never applied");
    let changes = counts.windows(2).filter(|pair| pair[0] != pair[1]).count();
    assert_eq!(changes, 1, "the list changed more than once: {counts:?}");
    assert!(counts.iter().all(|&count| count >= 3), "the list shrank mid-detection: {counts:?}");
    assert!(frames > 30 && server.frames_to_stale_ids() > 0, "frames went to the held ids while the detection ran");
    // The frames the detection made stale reached no device; the ones after the new list was read do.
    let landed_before = server.landings().len();
    adapter.write_frame(&DeviceId("openrgb:four".into()), &[BLUE; 8]).await.expect("a frame for the new device");
    until("the new device's frame landed", || server.landings().len() > landed_before).await;
    assert_eq!(server.landings().last().expect("landing").serial, "four");
}

/// A device that is really gone: after 103 the list is shorter and a frame to it is refused, while the
/// devices that remain take frames at their new ids.
#[tokio::test(start_paused = true)]
async fn a_real_unplug_applies_once_the_detection_is_over() {
    let server = server(vec![stick("one"), stick("two"), stick("three")]).await;
    server.rescan_timing(Duration::from_millis(100), Duration::from_secs(3));
    let mut adapter = connected(&server, 3).await;
    server.unplug_device(2);

    let scanner = adapter.clone();
    let rescan = tokio::spawn(async move { scanner.rescan_shared().await });
    let mut reader = adapter.clone();
    let mut seen = Vec::new();
    while !rescan.is_finished() {
        seen.push(reader.discover().await.expect("a read").len());
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    rescan.await.expect("the task ends").expect("the detection ended");
    // The read that happens to follow 103 by a moment sees the new list, so it is not counted.
    let during = &seen[..seen.len() - 1];
    assert!(during.len() > 10 && during.iter().all(|&count| count == 3), "the unplug applied before 103: {seen:?}");

    let after = adapter.discover().await.expect("a read after 103");
    let ids: Vec<_> = after.iter().map(|device| device.id.0.as_str()).collect();
    assert_eq!(ids, ["openrgb:one", "openrgb:two"], "the unplugged device is gone");
    let gone = adapter.write_frame(&DeviceId("openrgb:three".into()), &[RED; 8]).await;
    assert!(matches!(gone, Err(AdapterError::UnknownDevice(_))), "{gone:?}");
    adapter.write_frame(&DeviceId("openrgb:two".into()), &[BLUE; 8]).await.expect("a frame for a device that remains");
    until("the frame landed", || !server.landings().is_empty()).await;
    assert_eq!(server.landings()[0].serial, "two");
}

/// A packet 140 sent while a detection runs is dropped by OpenRGB (`DetectionManager.cpp:1753-1756`): no
/// new 101 follows, only the ACK. The call joins the running detection and ends on its 103.
#[tokio::test(start_paused = true)]
async fn a_packet_140_during_a_running_detection_joins_its_103() {
    let server = server(vec![stick("one"), stick("two")]).await;
    server.add_hidden_device(stick("three"));
    server.rescan_timing(Duration::from_millis(100), Duration::from_secs(2));
    let mut adapter = connected(&server, 2).await;

    // OpenRGB's own window starts a detection; the daemon's rescan arrives while it runs.
    server.start_detection();
    let started = Instant::now();
    adapter.rescan_shared().await.expect("the call joined the running detection and it ended");
    assert_eq!(server.rescan_dropped(), 1, "OpenRGB dropped the packet 140: a detection was running");
    assert!(
        started.elapsed() >= Duration::from_secs(6),
        "the call ended before the running detection's 103: {:?}",
        started.elapsed()
    );
    assert_eq!(adapter.discover().await.expect("a read after 103").len(), 3);
}

/// OpenRGB with detection off takes packet 140 and starts nothing. The ACK says the packet was handled
/// and no 101 came before it, so the rescan knows at once, with no window to wait out.
#[tokio::test(start_paused = true)]
async fn a_rescan_that_started_no_detection_says_so_when_the_ack_arrives() {
    let server = server(vec![stick("one")]).await;
    server.disable_detection(true);
    let adapter = connected(&server, 1).await;

    // Paused clock: the protocol 5 path waits 1.5 virtual seconds for a list change that never comes; this
    // path knows as soon as the ACK is in, and no virtual time passes.
    let started = Instant::now();
    let failed = adapter.rescan_shared().await.expect_err("detection never started");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::NotRedetected), "{failed:?}");
    assert!(started.elapsed() < Duration::from_millis(100), "the call waited for a window: {:?}", started.elapsed());
    assert_eq!(server.rescan_count(), 1);
    assert!(adapter.is_connected());
}

// ---- addressing by id

/// Two sticks of one model register again in the other order. Their old ids are dead: the frames the
/// daemon keeps sending while the detection runs are dropped by the server (none lands on either
/// stick), and once the detection is over each stick has a new id and gets its own frames.
#[tokio::test(start_paused = true)]
async fn two_identical_sticks_that_register_in_the_other_order_get_no_frame_meant_for_the_other() {
    let server = server(vec![twin("a"), twin("b")]).await;
    server.rescan_timing(Duration::from_millis(100), Duration::from_millis(500));
    server.rescan_order(vec![1, 0]);
    let mut adapter = connected(&server, 2).await;
    let a = DeviceId("openrgb:a".into());
    let b = DeviceId("openrgb:b".into());
    adapter.write_frame(&a, &[RED; 8]).await.expect("a frame for a");
    until("a's frame landed", || !server.landings().is_empty()).await;
    assert_eq!(server.landings()[0].serial, "a");
    let old_ids = server.controller_ids();

    let scanner = adapter.clone();
    let rescan = tokio::spawn(async move { scanner.rescan_shared().await });
    let sender = adapter.clone();
    while !rescan.is_finished() {
        // The daemon's frame writers go on: to the ids it holds.
        sender.write_frame_shared(&a, &[RED; 8]).await.expect("a frame for a held id is sent");
        sender.write_frame_shared(&b, &[BLUE; 8]).await.expect("a frame for a held id is sent");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    rescan.await.expect("the task ends").expect("the detection ended");
    for landing in server.landings() {
        let wanted = if landing.serial == "a" { RED } else { BLUE };
        assert_eq!(landing.color, wanted, "a frame meant for the other stick landed on {landing:?}");
    }
    assert!(server.frames_to_stale_ids() > 0, "the frames never went to the held ids: they were refused by the client");

    // The sticks swapped places for good and have new ids: each frame goes to where the stick is now.
    adapter.discover().await.expect("a read after 103");
    let new_ids = server.controller_ids();
    assert!(new_ids.iter().all(|id| !old_ids.contains(id)), "a re-detected controller kept its id: {old_ids:?} -> {new_ids:?}");
    let before = server.landings().len();
    adapter.write_frame(&a, &[RED; 8]).await.expect("a frame for a after the rescan");
    adapter.write_frame(&b, &[BLUE; 8]).await.expect("a frame for b after the rescan");
    until("both frames landed", || server.landings().len() >= before + 2).await;
    let landed = server.landings();
    let (to_a, to_b) = (&landed[before], &landed[before + 1]);
    assert_eq!((to_a.serial.as_str(), to_a.color), ("a", RED), "{to_a:?}");
    assert_eq!((to_b.serial.as_str(), to_b.color), ("b", BLUE), "{to_b:?}");
    // a registered second this time, so its id is the later one.
    assert_eq!((to_a.index, to_b.index), (new_ids[1], new_ids[0]), "the frames were addressed by the new ids");
}

/// A controller that registers again is a new object in its built-in mode: its first frame says
/// SET_CUSTOM_MODE, because its id is new and no mode was ever sent to it.
#[tokio::test]
async fn a_controller_that_registered_again_gets_the_custom_mode_again() {
    let server = server(vec![stick("one")]).await;
    let mut adapter = connected(&server, 1).await;
    let id = DeviceId("openrgb:one".into());
    adapter.write_frame(&id, &[RED; 8]).await.expect("first frame");
    until("the first mode landed", || server.custom_mode_writes(0) == 1).await;

    adapter.rescan_shared().await.expect("re-detected");
    adapter.discover().await.expect("a read after 103");
    adapter.write_frame(&id, &[RED; 8]).await.expect("a frame after the rescan");
    until("the mode was set again", || server.custom_mode_writes(0) == 2).await;
}

/// A request for an id the server no longer has is acknowledged and answered with nothing. The ACK says
/// so at once: the request fails quickly and the link stays up. Without it the request waits out its
/// reply timeout (5 s) and the link is dropped.
#[tokio::test(start_paused = true)]
async fn a_request_for_a_dead_id_is_refused_at_once_and_the_link_stays_up() {
    let server = server(vec![stick("one"), stick("two")]).await;
    server.rescan_timing(Duration::from_secs(60), Duration::from_millis(100));
    let mut adapter = connected(&server, 2).await;
    server.start_detection();
    until("the clear reached the client", || server.list_cleared()).await;

    let started = Instant::now();
    let failed = adapter.read_back(&DeviceId("openrgb:one".into())).await.expect_err("the id is gone");
    assert!(
        failed.to_string().contains("no longer has that controller id"),
        "the request was not refused by the ACK: {failed}"
    );
    assert!(started.elapsed() < Duration::from_secs(1), "the request waited for its reply timeout (5 s): {:?}", started.elapsed());
    assert!(adapter.is_connected(), "a request for a dead id broke the link");
}

// ---- the reader

/// Protocol 6 sends every client an echo of every frame, the sender too, and an ACK after every packet.
/// The reader reads and drops them all, so after ten thousand frames the link is healthy, nothing is
/// stuck behind them, and a request still gets its own reply.
#[tokio::test]
async fn the_reader_stays_healthy_over_ten_thousand_frames_and_their_echoes() {
    let server = server(vec![stick("one")]).await;
    let mut adapter = connected(&server, 1).await;
    let id = DeviceId("openrgb:one".into());
    for frame in 0..10_000_u32 {
        let shade = (frame % 250) as u8 + 1;
        adapter.write_frame(&id, &[Rgb8 { r: shade, g: 0, b: 0 }; 8]).await.expect("a frame");
    }
    until("every frame was echoed", || server.echoes_sent() >= 10_000).await;
    assert!(adapter.is_connected(), "the reader lost the link");
    // The reply to this request has to find its way past ten thousand echoes and ACKs.
    let colors = adapter.read_back(&id).await.expect("a request after the echoes");
    assert_eq!(colors, vec![Rgb8 { r: (9_999 % 250) as u8 + 1, g: 0, b: 0 }; 8], "the reply was the last frame's colours");
    assert_eq!(adapter.discover().await.expect("discover after the echoes").len(), 1);
    assert!(server.echoes_sent() >= 10_000, "the server never echoed the frames: it is not modelling protocol 6");
}

// ---- a connection that joins a detection that is already running

/// A server whose detection is already running when the client connects: OpenRGB restarted, or the daemon
/// reconnected mid-detection. The client never saw the 101, and release_1.1 signals progress per detector
/// and not per controller (`DetectionManager.cpp:720`, `:747`, `:1009`; a registration is a 100 alone,
/// `:491`), so all it may have seen of the detection is list changes. The first controller has registered,
/// the other two will in 3 s and 6 s.
async fn joined_mid_detection(gap: Duration) -> (FakeOpenRgbServer, OpenRgbAdapter) {
    let server = server(vec![stick("one"), stick("two"), stick("three")]).await;
    server.quiet_progress(true);
    server.rescan_timing(Duration::from_millis(100), gap);
    server.start_detection();
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    // Paused clock: the first registration is 100 ms into the detection.
    tokio::time::sleep(Duration::from_millis(500)).await;
    (server, adapter)
}

/// Review 6, gap 1: the held-list guard needs a held list, and a connection that joined holds none. Its
/// first read is OpenRGB's part list (one controller of three). The adapter must not call that list final:
/// the daemon holds its own list until the detection's 103 has been read, and only then is the list real.
#[tokio::test(start_paused = true)]
async fn a_connection_that_joined_a_running_detection_does_not_call_a_part_list_final() {
    let (_server, mut adapter) = joined_mid_detection(Duration::from_secs(3)).await;

    let part = adapter.discover().await.expect("a read mid-detection");
    assert_eq!(part.len(), 1, "the read is OpenRGB's part list");
    assert!(!adapter.list_is_final(), "a part list from a detection that joined mid-way was called final");
    assert!(!adapter.detection_end_unread(), "no 103 has arrived");

    until("the detection's 103 arrived", || adapter.detection_end_unread()).await;
    assert!(!adapter.list_is_final(), "the list was called final before it was read after the 103");
    let whole = adapter.discover().await.expect("a read after 103");
    assert_eq!(whole.len(), 3, "the list after 103 is whole");
    assert!(adapter.list_is_final(), "the list read after 103 was not final");
    assert!(!adapter.detection_end_unread(), "the 103 was read and still counted as unread");
}

/// Review 6 (fix first): release_1.1 sends a new client no detection state (`NetworkServer.cpp:886` is a
/// broadcast; the server starts before detection, `ResourceManager.cpp:1362` then `:1377`), so a connection
/// that heard nothing cannot tell a quiet OpenRGB from one that is mid-detection. Its first read is NOT final,
/// and neither is one after a restart: the daemon's rule decides what to apply, and a list is final only
/// after a 103.
#[tokio::test(start_paused = true)]
async fn a_first_read_that_heard_nothing_is_not_final_and_a_103_makes_the_list_final() {
    let server = server(vec![stick("one"), stick("two")]).await;
    server.rescan_timing(Duration::from_millis(100), Duration::from_millis(100));
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    assert_eq!(adapter.discover().await.expect("the first read").len(), 2);
    assert!(!adapter.list_is_final(), "a read that heard nothing was called final: a join may be mid-detection");
    assert!(!adapter.detection_end_unread(), "no 103 has arrived");
    assert_eq!(adapter.detection_signals(), 0);

    adapter.rescan_shared().await.expect("a rescan on a quiet server");
    assert!(adapter.detection_signals() >= 1, "the 101 of the rescan was not counted");
    assert!(adapter.detection_end_unread(), "the 103 was not noticed");
    assert!(!adapter.list_is_final(), "the list was called final before it was read after the 103");
    assert_eq!(adapter.discover().await.expect("a read after 103").len(), 2);
    assert!(adapter.list_is_final(), "the list read after 103 was not final");
    assert!(!adapter.detection_end_unread());
}

/// A hot plug is a 100 with no 101 before it and no 103 after it: the final list is final no longer, and the
/// read after it is not final either. The daemon's quiet period, not a 103, is what ends the doubt.
#[tokio::test(start_paused = true)]
async fn a_hot_plug_after_a_final_list_makes_it_not_final_and_no_103_follows() {
    let server = server(vec![stick("one"), stick("two")]).await;
    server.rescan_timing(Duration::from_millis(100), Duration::from_millis(100));
    let mut adapter = adapter(&server);
    adapter.connect().await.expect("connect");
    adapter.rescan_shared().await.expect("a rescan");
    adapter.discover().await.expect("a read after 103");
    assert!(adapter.list_is_final());

    server.unplug_device(1);
    server.list_changed();
    until("the hot plug's 100 arrived", || !adapter.list_is_final()).await;
    assert!(!adapter.detection_end_unread(), "a 100 alone was taken for a detection that ended");
    assert_eq!(adapter.discover().await.expect("a read after the hot plug").len(), 1, "the unplug was read");
    assert!(!adapter.list_is_final(), "a read after a hot plug with no 103 was called final");
    assert!(!adapter.detection_end_unread(), "the hot plug left a read owed for ever");
}

/// Review 6, gap 2: a client that joined mid-detection never saw the 101, so its packet 140 is dropped by
/// the server (`DetectionManager.cpp:1753-1756`) and answered by an ACK alone. A detection IS running: the
/// call waits for its 103 and then succeeds, as it does for a client that saw the 101.
#[tokio::test(start_paused = true)]
async fn a_packet_140_from_a_connection_that_joined_a_running_detection_waits_for_its_103() {
    let (server, mut adapter) = joined_mid_detection(Duration::from_secs(3)).await;

    let started = Instant::now();
    adapter.rescan_shared().await.expect("the call joined the running detection and it ended");
    assert_eq!(server.rescan_dropped(), 1, "OpenRGB dropped the packet 140: a detection was running");
    assert!(
        started.elapsed() >= Duration::from_secs(8),
        "the call ended before the running detection's 103: {:?}",
        started.elapsed()
    );
    assert_eq!(adapter.discover().await.expect("a read after 103").len(), 3);
    assert!(adapter.list_is_final());
}

/// The same client, and a detection that outlasts the call's bound (15 s): the answer is "still running",
/// the one a client that saw the 101 gets, never "did not start" while a detection runs. The link stays up.
#[tokio::test(start_paused = true)]
async fn a_packet_140_from_a_joined_connection_says_still_running_at_the_bound() {
    let (_server, adapter) = joined_mid_detection(Duration::from_secs(20)).await;

    let started = Instant::now();
    let failed = adapter.rescan_shared().await.expect_err("the detection is still running at the bound");
    assert_eq!(rescan_miss(&failed), Some(RescanMiss::Unsettled), "{failed:?}");
    assert!(
        started.elapsed() >= Duration::from_secs(15) && started.elapsed() < Duration::from_secs(16),
        "the bound is 15 s: {:?}",
        started.elapsed()
    );
    assert!(adapter.is_connected(), "the bound broke the link");
}
