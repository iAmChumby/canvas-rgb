//! The device path: one frame sink per device, the zone resize and the readback.
//!
//! These tests use the in-test fakes: no hardware is touched.

mod common;

use canvas_daemon::state::{App, DeviceSink};
use canvas_proto::device::{DeviceId, DeviceKind};
use canvas_proto::traits::FrameSink;
use common::{device, FakeAdapter, FakeEngine};
use std::sync::Arc;

/// Builds the app with a fake adapter that knows about one device.
fn build() -> Arc<App> {
    let adapter = FakeAdapter::default();
    {
        let mut devices = adapter.devices.lock().unwrap();
        *devices = vec![device("MSI B850 Mystic Light", DeviceKind::Motherboard, 120)];
    }
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    Arc::new(App::new(
        canvas_daemon::storage::Store::default(),
        engine,
        Box::new(adapter),
        "token".into(),
        "0.1.0-test".into(),
        tx,
    ))
}

#[tokio::test]
async fn a_frame_sink_drops_the_old_frame_when_the_device_is_slow() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let sink = DeviceSink::new(DeviceId("fake:one".into()), tx);
    sink.submit(vec![canvas_proto::Rgb8::WHITE]);
    sink.submit(vec![canvas_proto::Rgb8::BLACK]);

    // The first frame is there; the second was dropped rather than queued.
    let first = rx.try_recv().expect("the first frame is queued");
    assert_eq!(first, vec![canvas_proto::Rgb8::WHITE]);
    assert!(rx.try_recv().is_err(), "the channel holds one frame at most");
}

#[tokio::test]
async fn the_frame_sink_never_blocks_the_engine() {
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let sink = DeviceSink::new(DeviceId("fake:one".into()), tx);
    // Fill the channel, then keep submitting: this must not block.
    for _ in 0..100 {
        sink.submit(vec![canvas_proto::Rgb8::WHITE]);
    }
}

#[tokio::test]
async fn the_zone_sizes_are_re_applied_on_every_connect() {
    // The zone store: the sizes survive a reconnect.
    let mut sizes = std::collections::BTreeMap::new();
    let device_id = "fake:MSI B850 Mystic Light".to_string();
    sizes.insert(device_id.clone(), std::collections::BTreeMap::from([(0u32, 120u32)]));

    // Connect and prepare: the sizes are applied to the adapter.
    let mut adapter = FakeAdapter::default();
    {
        let mut devices = adapter.devices.lock().unwrap();
        *devices = vec![device("MSI B850 Mystic Light", DeviceKind::Motherboard, 120)];
    }
    let devices = canvas_daemon::openrgb_supervisor::connect_and_prepare(&mut adapter, &sizes)
        .await
        .expect("the connection succeeds");

    let applied = adapter.zone_sizes.lock().unwrap().clone();
    assert_eq!(applied.len(), 1, "the zone size was applied");
    assert_eq!(applied[0].0, DeviceId(device_id));
    assert_eq!(applied[0].1, 0);
    assert_eq!(applied[0].2, 120);

    // The devices come back with their LED counts.
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].led_count(), 120);
}

#[tokio::test]
async fn a_connect_without_saved_sizes_changes_nothing() {
    let mut adapter = FakeAdapter::default();
    {
        let mut devices = adapter.devices.lock().unwrap();
        *devices = vec![device("MSI B850 Mystic Light", DeviceKind::Motherboard, 120)];
    }
    let devices = canvas_daemon::openrgb_supervisor::connect_and_prepare(
        &mut adapter,
        &std::collections::BTreeMap::new(),
    )
    .await
    .expect("the connection succeeds");
    assert!(adapter.zone_sizes.lock().unwrap().is_empty());
    assert_eq!(devices.len(), 1);
}

#[tokio::test]
async fn the_readback_reports_the_colours_the_adapter_gives() {
    let app = build();
    // The device is known.
    {
        let mut list = app.devices.write().await;
        *list = vec![device("MSI B850 Mystic Light", DeviceKind::Motherboard, 120)];
    }
    // The route answers with the devices it knows.
    let response =
        canvas_daemon::api::diag::readback(axum::extract::State(app.clone())).await;
    let response = response.expect("the readback answers");
    let devices = response.0.devices;
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].device_id, "fake:MSI B850 Mystic Light");
}

/// A fake adapter whose first `failures` connects fail.
struct FlakyAdapter {
    inner: FakeAdapter,
    failures: u32,
}

#[async_trait::async_trait]
impl canvas_proto::traits::DeviceAdapter for FlakyAdapter {
    fn name(&self) -> &'static str {
        "flaky"
    }
    fn capabilities(&self) -> canvas_proto::device::AdapterCapabilities {
        self.inner.capabilities()
    }
    async fn connect(&mut self) -> Result<(), canvas_proto::traits::AdapterError> {
        if self.failures > 0 {
            self.failures -= 1;
            return Err(canvas_proto::traits::AdapterError::NotConnected);
        }
        self.inner.connect().await
    }
    async fn disconnect(&mut self) -> Result<(), canvas_proto::traits::AdapterError> {
        self.inner.disconnect().await
    }
    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }
    async fn discover(
        &mut self,
    ) -> Result<Vec<canvas_proto::device::DeviceInfo>, canvas_proto::traits::AdapterError> {
        self.inner.discover().await
    }
    async fn write_frame(
        &mut self,
        device: &DeviceId,
        colors: &[canvas_proto::Rgb8],
    ) -> Result<(), canvas_proto::traits::AdapterError> {
        self.inner.write_frame(device, colors).await
    }
    async fn set_zone_size(
        &mut self,
        device: &DeviceId,
        zone: u32,
        leds: u32,
    ) -> Result<(), canvas_proto::traits::AdapterError> {
        self.inner.set_zone_size(device, zone, leds).await
    }
    async fn identify(
        &mut self,
        device: &DeviceId,
        zone: u32,
        led: u32,
    ) -> Result<(), canvas_proto::traits::AdapterError> {
        self.inner.identify(device, zone, led).await
    }
    async fn read_back(
        &mut self,
        device: &DeviceId,
    ) -> Result<Vec<canvas_proto::Rgb8>, canvas_proto::traits::AdapterError> {
        self.inner.read_back(device).await
    }
}

#[tokio::test]
async fn the_supervisor_reports_connecting_on_every_attempt_and_down_only_after_a_failure() {
    use canvas_proto::api::LinkState;
    let adapter: Arc<tokio::sync::Mutex<Box<dyn canvas_proto::traits::DeviceAdapter>>> =
        Arc::new(tokio::sync::Mutex::new(Box::new(FlakyAdapter {
            inner: FakeAdapter::default(),
            failures: 2,
        })));
    let zones = Arc::new(tokio::sync::RwLock::new(std::collections::BTreeMap::new()));
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let events: Arc<std::sync::Mutex<Vec<&'static str>>> = Arc::default();

    let on_devices = {
        let events = events.clone();
        let stop_tx = stop_tx.clone();
        move |_devices| {
            events.lock().unwrap().push("devices");
            let _ = stop_tx.send(true);
        }
    };
    let on_link = {
        let events = events.clone();
        move |link| {
            events.lock().unwrap().push(match link {
                LinkState::Connecting => "connecting",
                LinkState::Down => "down",
                LinkState::Connected => "connected",
            });
        }
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        canvas_daemon::openrgb_supervisor::supervise(adapter, zones, on_devices, on_link, stop_rx),
    )
    .await
    .expect("the supervisor stops once connected");

    let seen = events.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec!["connecting", "down", "connecting", "down", "connecting", "devices"],
        "Connecting before each attempt, Down only after a failure"
    );
}
