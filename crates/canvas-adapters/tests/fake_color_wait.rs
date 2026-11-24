//! The fake server's colour wait is woken by the write, never by a clock.
//!
//! The class `cause:timing-test`: a test that polls `colors(index)` on a 20 ms sleep reads its
//! verdict off how fast the machine is, and it passes when the colours land late. The fake now
//! says when a write lands them, so a test waits on the thing its assert reads.

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

fn stick() -> DeviceInfo {
    DeviceInfo {
        id: DeviceId("openrgb:wait".into()),
        name: "Waiting stick".into(),
        vendor: "fake".into(),
        kind: DeviceKind::Dram,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "wait".into(),
        location: "smbus".into(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "RGB".into(),
            kind: ZoneKind::Linear,
            led_count: 4,
            min_leds: 4,
            max_leds: 4,
            resizable: false,
        }],
        connected: true,
    }
}

/// A waiter that started before the write is woken by the write, and then the wait for the
/// colours ends with them.
///
/// The waiter subscribes first, so only the server's notification can wake it: the cap is a hang
/// guard, never the verdict. The assertion reads the colours the wait returned.
#[tokio::test]
async fn a_colour_write_wakes_a_wait_for_the_colours() {
    let server = FakeOpenRgbServer::start(vec![stick()]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("a plain discover");
    let id = DeviceId("openrgb:wait".into());
    let colours = vec![Rgb8::new(1, 2, 3); 4];

    let mut ticks = server.write_ticks();
    let wait = server.wait_for_colors(0, &colours);
    let write = async {
        // The waiter reads the controller before the write lands, so this checks the wake.
        tokio::task::yield_now().await;
        adapter.write_frame_shared(&id, &colours).await
    };
    let (woken, sent, waited) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(10), ticks.changed()),
        write,
        wait,
    );
    sent.expect("the frame goes out");
    woken.expect("the write wakes the waiter; this cap only ends a test that would hang").expect("the wait is open");
    assert_eq!(waited, colours, "the wait ended with the colours the write landed");
}

/// A resize blanks the controller's colours, so it wakes a wait for them like any other write.
///
/// The waiter subscribes before the resize lands. The outer 1 s timeout is the verdict: the
/// colour wait's own cap is 10 s, so a resize that never bumps the signal leaves this test
/// failing at 1 s instead of passing after the cap.
#[tokio::test]
async fn a_resize_wakes_a_wait_for_the_colours() {
    let server = FakeOpenRgbServer::start(vec![header()]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    adapter.discover().await.expect("a plain discover");
    let id = DeviceId("openrgb:jar".into());
    // A colour on the controller, so the resize has something to blank.
    adapter.write_frame_shared(&id, &[Rgb8::WHITE; 8]).await.expect("the frame");
    let landed = server.wait_for_colors(0, &[Rgb8::WHITE; 8]).await;
    assert_eq!(landed, vec![Rgb8::WHITE; 8], "the frame did not land before the resize");

    // The resize takes the zone to 12 LEDs and blanks its colours to that new length.
    let blanked = vec![Rgb8::BLACK; 12];
    let wait = server.wait_for_colors(0, &blanked);
    let resize = async {
        // The waiter is polled first, so it is subscribed before the resize lands.
        tokio::task::yield_now().await;
        adapter.set_zone_size(&id, 0, 12).await
    };
    let (woken, resized) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(1), wait),
        resize,
    );
    resized.expect("the resize works");
    assert_eq!(
        woken.expect("the resize wakes the wait for the colours, well under its 10 s cap"),
        blanked,
        "the wait ended with the colours the resize blanked"
    );
}
