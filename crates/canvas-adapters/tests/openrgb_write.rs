//! A write that is given up on must not leave a half-written packet on the socket.
//!
//! The write holds the socket lock across the write, so a caller that gives up on it (the
//! rescan budget does) can leave a part of a packet on the socket. The next writer would then
//! write into the middle of it, and OpenRGB would read two packets out of one.
//!
//! The fake OpenRGB server stops reading on command (`stop_reading`), so the socket fills and
//! the next write blocks in the middle of the packet. The test gives up on that write and
//! checks that the link is broken: the next writer must not write into the middle of the
//! half-written packet.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_proto::{DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8, ZoneInfo, ZoneKind};
use std::time::Duration;

/// How many LEDs the controller has. A frame of this size fills the socket in a few frames.
const LEDS: u32 = 8192;

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

fn big_stick() -> DeviceInfo {
    DeviceInfo {
        id: DeviceId("openrgb:big".into()),
        name: "Big stick".into(),
        vendor: "fake".into(),
        kind: DeviceKind::Dram,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "big".into(),
        location: "smbus".into(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "RGB".into(),
            kind: ZoneKind::Linear,
            led_count: LEDS,
            min_leds: LEDS,
            max_leds: LEDS,
            resizable: false,
        }],
        connected: true,
    }
}

/// A write that is given up on leaves the link broken.
///
/// The socket fills, so a write blocks in the middle of its packet. The test gives up on it
/// (a caller that ran out of time does that), and the link must be broken: the socket may
/// hold a part of a packet, and the next writer would write into the middle of it.
#[tokio::test]
async fn a_write_that_is_given_up_on_leaves_the_link_broken() {
    let server = FakeOpenRgbServer::start(vec![big_stick()]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("a plain discover");

    // The server stops reading: the socket fills, and the next write blocks.
    server.stop_reading(true);
    let id = DeviceId("openrgb:big".into());
    let colors = vec![Rgb8::WHITE; LEDS as usize];
    let mut blocked = false;
    for _ in 0..200 {
        // The wait is a hang guard: it ends a test that would hang. The verdict is the
        // link's state below, not the time.
        match tokio::time::timeout(Duration::from_secs(2), adapter.write_frame_shared(&id, &colors)).await {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => panic!("the frame failed: {error}"),
            Err(_) => {
                // The write did not go out: the socket is full. The future is dropped here,
                // which is what a caller that ran out of time does.
                blocked = true;
                break;
            }
        }
    }
    assert!(blocked, "the socket never filled: the write never blocked");

    // The write was cut in the middle: the link is broken, so no other writer writes into
    // the middle of the half-written packet.
    assert!(
        !adapter.is_connected(),
        "the link stayed up after a write that was given up on"
    );
}

/// The link comes back after a write that was given up on.
///
/// A broken link is not a dead daemon: the supervisor reconnects, and the frames go out
/// again on the new connection.
#[tokio::test]
async fn the_link_comes_back_after_a_write_that_was_given_up_on() {
    let server = FakeOpenRgbServer::start(vec![big_stick()]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("a plain discover");

    server.stop_reading(true);
    let id = DeviceId("openrgb:big".into());
    let colors = vec![Rgb8::WHITE; LEDS as usize];
    for _ in 0..200 {
        // The wait is a hang guard: it ends a test that would hang.
        if tokio::time::timeout(Duration::from_secs(2), adapter.write_frame_shared(&id, &colors)).await.is_err() {
            break;
        }
    }
    assert!(!adapter.is_connected(), "the link stayed up after a write that was given up on");

    // The server reads again, and the adapter connects afresh.
    server.stop_reading(false);
    adapter.disconnect().await.expect("disconnect");
    adapter.connect().await.expect("connect again");
    assert!(adapter.is_connected());
    adapter.discover().await.expect("a plain discover");
    let before = server.update_leds_writes(0);
    adapter.write_frame_shared(&id, &colors).await.expect("the frame goes out again");
    until("the frame reached the lights", || server.update_leds_writes(0) == before + 1).await;
}
