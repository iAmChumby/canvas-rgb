//! #308: the newest frame goes out when the device is behind.
//!
//! The device is often slower than the render loop: the writer is busy with a frame while
//! the engine sends more. The frame that matters is the newest one, so a frame that arrives
//! while the device is behind replaces the older one instead of being dropped. A queue of
//! one keeps the oldest frame and drops the newest, and the lights then show colours the
//! scene has left behind.
//!
//! The fakes: the in-test adapter that waits for a permit, and the daemon's own path to a
//! frame sink (the one the daemon builds). No hardware is touched.

mod common;

use canvas_daemon::state::App;
use canvas_proto::device::{DeviceId, DeviceKind};
use common::{device, FakeAdapter, FakeEngine};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Waits until `ready` is true. The wait is on the thing the assert reads; the long limit
/// only ends a test that would hang.
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while !ready() {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never happened: {what}"));
}

/// An adapter whose write waits for a permit: the device is behind until the test lets it go.
///
/// It records the frames it wrote in a log the test reads, and how many writes have started.
struct SlowAdapter {
    inner: FakeAdapter,
    /// The permit every write waits for.
    gate: Arc<tokio::sync::Semaphore>,
    /// How many writes have started.
    started: Arc<AtomicU32>,
    /// The frames that went out.
    written: Arc<Mutex<Vec<Vec<canvas_proto::Rgb8>>>>,
}

impl canvas_adapters::resume::ResumeBehavior for SlowAdapter {
    fn on_resume(&self) -> canvas_adapters::resume::OnResume {
        // A test fake: it holds no mode.
        canvas_adapters::resume::OnResume::NothingToRestore
    }
}

impl canvas_adapters::frame_lane::FrameLane for SlowAdapter {
    fn frame_lane(&self) -> Option<canvas_adapters::openrgb::OpenRgbAdapter> {
        // A test fake: it has no OpenRGB link, so its frames go through the adapter.
        None
    }
}

#[async_trait::async_trait]
impl canvas_proto::traits::DeviceAdapter for SlowAdapter {
    fn name(&self) -> &'static str {
        "slow"
    }
    fn capabilities(&self) -> canvas_proto::device::AdapterCapabilities {
        self.inner.capabilities()
    }
    async fn connect(&mut self) -> Result<(), canvas_proto::traits::AdapterError> {
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
        _device: &DeviceId,
        colors: &[canvas_proto::Rgb8],
    ) -> Result<(), canvas_proto::traits::AdapterError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        // The device is behind: the write waits until the test lets it go.
        let permit = self.gate.acquire().await.expect("the gate stays open");
        permit.forget();
        self.written.lock().unwrap().push(colors.to_vec());
        Ok(())
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

/// The newest frame goes out when the device is behind (#308).
///
/// The device is slower than the render loop, so the writer is busy with a frame while the
/// engine sends more. The frame that goes out must be the newest one, not the oldest one
/// that was still waiting: a frame that arrives while the device is behind replaces the
/// older one instead of being dropped.
#[tokio::test]
async fn the_newest_frame_goes_out_when_the_device_is_behind() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let started = Arc::new(AtomicU32::new(0));
    let written = Arc::new(Mutex::new(Vec::new()));
    let adapter = SlowAdapter {
        inner: FakeAdapter::default(),
        gate: Arc::clone(&gate),
        started: Arc::clone(&started),
        written: Arc::clone(&written),
    };
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        canvas_daemon::storage::Store::default(),
        engine.clone(),
        Box::new(adapter),
        "token".into(),
        "0.1.0-test".into(),
        tx,
    ));
    let device = device("MSI B850 Mystic Light", DeviceKind::Motherboard, 120);
    // The daemon's own path: a frame sink and a writer for the device.
    canvas_daemon::device_sync::sync_device_sinks(&app, std::slice::from_ref(&device)).await;
    let sink = engine
        .sinks
        .lock()
        .unwrap()
        .get(&device.id)
        .cloned()
        .expect("the device has a frame sink");

    // The first frame goes out, and the device is busy with it.
    sink.submit(vec![canvas_proto::Rgb8::WHITE; 120]);
    until("the first write starts", || started.load(Ordering::SeqCst) == 1).await;

    // Two more frames arrive while the device is busy: the newest one is the red frame.
    sink.submit(vec![canvas_proto::Rgb8::BLACK; 120]);
    sink.submit(vec![canvas_proto::Rgb8::new(255, 0, 0); 120]);

    // The device catches up, one frame at a time.
    for _ in 0..2 {
        gate.add_permits(1);
    }
    until("both writes went out", || written.lock().unwrap().len() == 2).await;
    // The second frame the device got is the newest one, not the black frame that was
    // waiting behind it.
    let frames = written.lock().unwrap().clone();
    assert_eq!(frames[0], vec![canvas_proto::Rgb8::WHITE; 120], "the first frame went out");
    assert_eq!(
        frames[1],
        vec![canvas_proto::Rgb8::new(255, 0, 0); 120],
        "the newest frame did not go out"
    );
}
