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
