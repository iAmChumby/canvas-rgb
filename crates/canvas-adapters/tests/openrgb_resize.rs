//! A frame for a controller that is being resized must not reach OpenRGB with the old size.
//!
//! The zone resize runs under the adapter lock, but the frame writers do not take that lock
//! any more: they go out on the OpenRGB link itself, so the lights keep moving while a
//! discovery, a refresh or a rescan holds the adapter (#233). A frame that lands while
//! OpenRGB is re-configuring the controller carries the old zone size, and it may stamp the
//! custom mode back on after the resize cleared it.
//!
//! The adapter therefore skips the frames of a controller that is being resized, until the
//! re-read of the list lands.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_proto::{DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8, ZoneInfo, ZoneKind};
use std::time::Duration;

/// A controller whose zone can be resized, as an MSI JARGB header.
fn header() -> DeviceInfo {
    DeviceInfo {
        id: DeviceId("openrgb:jar".into()),
        name: "JARGB".into(),
        vendor: "fake".into(),
        kind: DeviceKind::Motherboard,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "jar".into(),
        location: "smbus".into(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "Header".into(),
            kind: ZoneKind::Linear,
            led_count: 8,
            min_leds: 4,
            max_leds: 16,
            resizable: true,
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

/// A frame for a controller that is being resized does not reach OpenRGB, and the mode is
/// set again after the resize.
#[tokio::test]
async fn a_frame_for_a_controller_being_resized_does_not_reach_openrgb() {
    let server = FakeOpenRgbServer::start(vec![header()]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("a plain discover");

    let id = DeviceId("openrgb:jar".into());
    // The first frame sets the mode and writes the colours.
    adapter.write_frame_shared(&id, &[Rgb8::WHITE; 8]).await.expect("the first frame");
    until("the first frame reached OpenRGB", || server.update_leds_writes(0) == 1).await;
    assert_eq!(server.custom_mode_writes(0), 1, "the first frame set the mode");
    assert_eq!(server.update_leds_writes(0), 1, "the first frame wrote the colours");

    // The server keeps its replies back: the re-read of the list after the resize waits, so
    // the resize is still in flight while the frame goes out.
    server.hold_replies(true);
    let mut resizing = adapter.clone();
    let resize_id = id.clone();
    let resize = tokio::spawn(async move { resizing.set_zone_size(&resize_id, 0, 12).await });
    // The resize reached OpenRGB: the zone has its new size there.
    until("the zone was resized", || server.zone_leds(0, 0) == Some(12)).await;

    // A frame for that controller while the resize is in flight: it must not go out. It was
    // built for the old size, and OpenRGB is re-configuring the controller right now.
    let result = adapter.write_frame_shared(&id, &[Rgb8::WHITE; 8]).await;
    assert!(result.is_err(), "the frame of the old size went out during the resize");
    assert_eq!(server.update_leds_writes(0), 1, "a frame reached OpenRGB during the resize");
    assert_eq!(server.custom_mode_writes(0), 1, "the mode was set during the resize");

    // The slow OpenRGB catches up: the resize finishes and the list is read again.
    server.hold_replies(false);
    server.release_replies();
    resize.await.expect("its task ends").expect("the resize works");

    // The frame after the resize goes out, and the mode is set again: the resize took the
    // controller out of custom mode.
    adapter.write_frame_shared(&id, &[Rgb8::WHITE; 12]).await.expect("the frame after the resize");
    until("the frame after the resize reached OpenRGB", || server.update_leds_writes(0) == 2).await;
    assert_eq!(server.custom_mode_writes(0), 2, "the mode was not set again after the resize");
    assert_eq!(server.update_leds_writes(0), 2, "the frame after the resize did not go out");
}

/// A resize that was cancelled does not skip the frames of that controller forever.
///
/// The client went away in the middle of the PATCH: the resize is dropped before it reads the
/// list again. The frames of that controller must go out again, not stay skipped for the rest
/// of the connection.
#[tokio::test]
async fn a_cancelled_resize_does_not_skip_the_frames_of_that_controller_forever() {
    let server = FakeOpenRgbServer::start(vec![header()]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("a plain discover");

    let id = DeviceId("openrgb:jar".into());
    // The first frame sets the mode and writes the colours.
    adapter.write_frame_shared(&id, &[Rgb8::WHITE; 8]).await.expect("the first frame");
    until("the first frame reached OpenRGB", || server.update_leds_writes(0) == 1).await;

    // The server keeps its replies back: a request holds the request gate, and the resize
    // waits for that gate when it reads the list again.
    server.hold_replies(true);
    let mut asking = adapter.clone();
    let asking_id = id.clone();
    let holding = tokio::spawn(async move { asking.read_back(&asking_id).await });
    until("the request holds the gate", || server.held_replies() == 1).await;

    // The resize starts: it writes the new zone size, then waits for the gate to read the
    // list again.
    let mut resizing = adapter.clone();
    let resize_id = id.clone();
    let resize = tokio::spawn(async move { resizing.set_zone_size(&resize_id, 0, 12).await });
    until("the zone was resized", || server.zone_leds(0, 0) == Some(12)).await;

    // The client goes away in the middle of the PATCH: the resize is dropped.
    resize.abort();
    let _ = resize.await;

    // The frames of that controller go out again: the cancelled resize does not keep them
    // skipped for the rest of the connection.
    adapter.write_frame_shared(&id, &[Rgb8::WHITE; 8]).await.expect("the frame after the cancelled resize");
    until("the frame after the cancelled resize reached OpenRGB", || server.update_leds_writes(0) == 2).await;

    // The request that held the gate is given up on too, so the test leaves nothing waiting.
    holding.abort();
    let _ = holding.await;
}
