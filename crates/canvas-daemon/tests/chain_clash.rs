//! Chain setup raises a header to its longest length while the owner drags (#338). The OpenRGB
//! supervisor re-applies every saved zone size each refresh, so a raise on a header that first run
//! sized to 120 is "wrong" to it. A setup holds the raised length while it works (`App::zone_holds`,
//! `canvas_daemon::state::ZoneHolds`), and the supervisor overlays the holds on the saved sizes: a
//! held zone is left alone, and an unheld one is still resized back. No test sleeps through a
//! refresh: the second, unheld header being resized back shows that one ran.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_daemon::openrgb_supervisor::supervise_with_rescans;
use canvas_daemon::state::ZoneHolds;
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::traits::DeviceAdapter;
use canvas_proto::DeviceId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// The saved size of the header, from the first run.
const SAVED: u32 = 120;
/// The length chain setup raises it to while the owner drags.
const RAISED: u32 = 300;
/// The saved size of the board's second header, which no setup holds. A refresh that resizes it
/// back is the proof that a refresh ran, so no test has to sleep through one.
const OTHER_SAVED: u32 = 60;
/// What a test raises the second header to.
const OTHER_RAISED: u32 = 200;
/// A test's reply deadline is a hang guard, not the verdict: a loaded runner must never turn a
/// slow-but-live reply from the fake into "no reply to packet ... within 5s" (#519).
const TEST_REPLY_TIMEOUT: Duration = Duration::from_secs(60);

fn jargb(index: u32, name: &str) -> ZoneInfo {
    ZoneInfo {
        index,
        name: name.into(),
        kind: ZoneKind::Linear,
        led_count: 0,
        min_leds: 0,
        max_leds: 300,
        resizable: true,
    }
}

/// A board with two JARGB headers: any size from 0 to 300.
fn board() -> Vec<DeviceInfo> {
    vec![DeviceInfo {
        id: DeviceId("openrgb:mb".into()),
        name: "MSI MPG B850 Mystic Light".into(),
        vendor: "Fake".into(),
        kind: DeviceKind::Motherboard,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "mb".into(),
        location: "usb".into(),
        zones: vec![jargb(0, "JARGB1"), jargb(1, "JARGB2")],
        connected: true,
    }]
}

async fn wait_for(server: &FakeOpenRgbServer, zone: u32, leds: u32, _within: Duration) -> bool {
    // The verdict is the size the fake holds, and the wait is woken by the fake's own change
    // (#519). The old wall-clock deadline made a loaded runner answer "false" while the resize
    // was simply still on its way.
    server
        .wait_until("the zone size", || server.zone_leds(0, zone) == Some(leds))
        .await;
    server.zone_leds(0, zone) == Some(leds)
}

/// The real supervisor over the fake server, with the saved 120 and the holds the test chooses.
struct Harness {
    server: FakeOpenRgbServer,
    adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>>,
    holds: Arc<tokio::sync::RwLock<ZoneHolds>>,
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.task.abort();
    }
}

impl Harness {
    async fn start(holds: ZoneHolds) -> Self {
        let server = FakeOpenRgbServer::start(board()).await.expect("fake server");
        // The test's raise goes through the supervisor's own adapter: the header is one device.
        let adapter: Box<dyn DeviceAdapter> = Box::new(
            OpenRgbAdapter::new("127.0.0.1", server.port()).with_reply_timeout(TEST_REPLY_TIMEOUT),
        );
        let adapter = Arc::new(tokio::sync::Mutex::new(adapter));
        let sizes = BTreeMap::from([("openrgb:mb".to_string(), BTreeMap::from([(0u32, SAVED), (1u32, OTHER_SAVED)]))]);
        let zone_sizes = Arc::new(tokio::sync::RwLock::new(sizes));
        let holds = Arc::new(tokio::sync::RwLock::new(holds));
        let (stop, stop_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(supervise_with_rescans(
            Arc::clone(&adapter),
            zone_sizes,
            Arc::clone(&holds),
            |_devices| async {},
            |_link| async {},
            None,
            stop_rx,
        ));
        Self { server, adapter, holds, stop, task }
    }

    fn leds(&self) -> Option<u32> {
        self.server.zone_leds(0, 0)
    }

    async fn wait_for(&self, leds: u32, within: Duration) -> bool {
        wait_for(&self.server, 0, leds, within).await
    }

    /// Waits for the unheld second header to reach `leds`.
    async fn wait_for_other(&self, leds: u32, within: Duration) -> bool {
        wait_for(&self.server, 1, leds, within).await
    }

    /// Raises the second header, which nothing holds.
    async fn raise_other(&self) {
        self.adapter
            .lock()
            .await
            .set_zone_size(&DeviceId("openrgb:mb".into()), 1, OTHER_RAISED)
            .await
            .expect("raise the other header");
    }

    /// The raise chain setup does when it starts.
    async fn raise(&self) {
        self.adapter
            .lock()
            .await
            .set_zone_size(&DeviceId("openrgb:mb".into()), 0, RAISED)
            .await
            .expect("raise");
    }

    /// The hold chain setup takes while the owner drags.
    async fn hold(&self) {
        self.holds.write().await.insert(("openrgb:mb".to_string(), 0), RAISED);
    }

    /// Drops the hold, as chain setup does when it is done or gives up.
    async fn release(&self) {
        self.holds.write().await.remove(&("openrgb:mb".to_string(), 0));
    }
}

/// A held raise survives the refresh that used to undo it mid-drag (#338).
#[tokio::test]
async fn refresh_does_not_undo_a_held_raise() {
    let harness = Harness::start(ZoneHolds::new()).await;
    assert!(harness.wait_for(SAVED, Duration::from_secs(5)).await, "connect applies the saved 120");

    harness.hold().await;
    harness.raise().await;
    assert_eq!(harness.leds(), Some(RAISED), "the raise reached the header");
    harness.raise_other().await;

    // The refresh that resizes the unheld header back is the one that used to undo the held
    // raise: once it has run, the held header must still be raised.
    assert!(
        harness.wait_for_other(OTHER_SAVED, Duration::from_secs(8)).await,
        "a refresh resizes the unheld header back to its saved size"
    );
    assert_eq!(harness.leds(), Some(RAISED), "the supervisor must leave a held raise alone");
}

/// OpenRGB forgets SDK resizes when it restarts; a held raise is re-applied on the reconnection.
#[tokio::test]
async fn an_openrgb_restart_during_a_hold_reapplies_the_raised_length() {
    let harness = Harness::start(ZoneHolds::new()).await;
    assert!(harness.wait_for(SAVED, Duration::from_secs(5)).await, "connect applies the saved 120");

    harness.hold().await;
    harness.raise().await;
    assert_eq!(harness.leds(), Some(RAISED), "the raise reached the header");

    harness.server.restart().await;
    assert_eq!(harness.leds(), Some(0), "OpenRGB forgot the resize when it restarted");

    assert!(
        harness.wait_for(RAISED, Duration::from_secs(20)).await,
        "the supervisor re-applies a held raise when it reconnects"
    );
}

/// The hold lasts as long as the setup keeps it: without it the saved size comes back.
#[tokio::test]
async fn dropping_the_hold_lets_the_saved_size_return() {
    let harness = Harness::start(ZoneHolds::new()).await;
    assert!(harness.wait_for(SAVED, Duration::from_secs(5)).await, "connect applies the saved 120");

    harness.hold().await;
    harness.raise().await;
    assert_eq!(harness.leds(), Some(RAISED), "the raise reached the header");

    harness.release().await;
    assert!(
        harness.wait_for(SAVED, Duration::from_secs(8)).await,
        "the next refresh applies the saved 120 once the hold is gone"
    );
}

/// The old behaviour, for a zone nothing holds: the refresh resizes the raise back (#338).
#[tokio::test]
async fn without_a_hold_the_supervisor_still_resizes_back() {
    let harness = Harness::start(ZoneHolds::new()).await;
    assert!(harness.wait_for(SAVED, Duration::from_secs(5)).await, "connect applies the saved 120");

    harness.raise().await;
    assert_eq!(harness.leds(), Some(RAISED), "the raise reached the header");

    assert!(
        harness.wait_for(SAVED, Duration::from_secs(8)).await,
        "an unheld raise is resized back to the saved size"
    );
}
