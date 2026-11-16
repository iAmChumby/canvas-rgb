//! The fake server's colour wait is woken by the write, never by a clock.
//!
//! The class `cause:timing-test`: a test that polls `colors(index)` on a 20 ms sleep reads its
//! verdict off how fast the machine is, and it passes when the colours land late. The fake now
//! says when a write lands them, so a test waits on the thing its assert reads.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_proto::{DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8, ZoneInfo, ZoneKind};
use std::time::Duration;

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
