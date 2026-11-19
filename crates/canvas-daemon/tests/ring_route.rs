//! The controller rings through the daemon's composite adapter (#376).
//!
//! A `ring:` device is routed to the ring adapter, never to OpenRGB. The ring adapter shares the
//! pad's vendor channel with the peripheral adapter and obeys the peripheral write gate and the
//! idle rule, so with the gate off a scene change sends the pad nothing. Everything here runs on
//! the fake pad and the fake OpenRGB server: no hardware is touched.

use std::sync::Arc;
use std::time::Duration;

use canvas_adapters::eightbitdo_ultimate2::ring::{
    ring_device_id, RingLightAdapter, IDLE_BEFORE_WRITE, RING_LEDS,
};
use canvas_adapters::eightbitdo_ultimate2::{Opener, Personality, VendorHandle};
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::fake_pad::{self, FakePad};
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_daemon::composite::CompositeAdapter;
use canvas_daemon::peripherals::PERIPHERAL_IDLE_READ_S;
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::{DeviceAdapter, DeviceId, Rgb8};

fn stick() -> DeviceInfo {
    DeviceInfo {
        id: DeviceId("openrgb:dram".into()),
        name: "DDR5".into(),
        vendor: "fake".into(),
        kind: DeviceKind::Dram,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "dram".into(),
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

struct Rig {
    composite: CompositeAdapter,
    pad: FakePad,
    handle: VendorHandle,
    server: FakeOpenRgbServer,
}

/// A composite over the fake OpenRGB server and a ring adapter on an open fake pad. The gate is
/// off, as the daemon starts it; the person has been away for a minute.
async fn rig() -> Rig {
    let server = FakeOpenRgbServer::start(vec![stick()]).await.expect("the fake OpenRGB server");
    let pad = FakePad::with_image(fake_pad::ring_fixture_image(), Personality::XInput);
    let handle = VendorHandle::new();
    handle.open_with(pad.open_vendor(Personality::XInput).unwrap().unwrap());
    handle.set_pad_present(true);
    handle.set_idle_source(Arc::new(|| Duration::from_secs(60)));
    let rings = RingLightAdapter::without_store(handle.clone()).without_pauses();
    let composite =
        CompositeAdapter::openrgb_only(OpenRgbAdapter::new("127.0.0.1", server.port())).with_rings(rings);
    Rig { composite, pad, handle, server }
}

#[tokio::test]
async fn the_ring_device_is_listed_beside_the_openrgb_devices() {
    let mut r = rig().await;
    r.composite.connect().await.expect("connect");
    let devices = r.composite.discover().await.expect("discover");
    let ids: Vec<&str> = devices.iter().map(|d| d.id.0.as_str()).collect();
    assert!(ids.contains(&"openrgb:dram"), "{ids:?}");
    let ring = devices.iter().find(|d| d.id == ring_device_id()).expect("the ring device");
    assert_eq!(ring.led_count(), 24);
    assert_eq!(ring.zones.len(), 2);
    // Without a pad there is no ring device.
    r.handle.close();
    r.handle.set_pad_present(false);
    let devices = r.composite.discover().await.expect("discover");
    assert!(devices.iter().all(|d| d.id != ring_device_id()));
}

#[tokio::test]
async fn a_ring_frame_goes_to_the_pad_and_never_to_openrgb() {
    let mut r = rig().await;
    r.composite.connect().await.expect("connect");
    assert!(!CompositeAdapter::is_openrgb_device(&ring_device_id()), "the writer must not use the OpenRGB lane");
    assert!(CompositeAdapter::is_openrgb_device(&DeviceId("openrgb:dram".into())));
    r.handle.set_writes_enabled(true);
    let frame = vec![Rgb8::new(255, 0, 0); RING_LEDS];
    r.composite.write_frame(&ring_device_id(), &frame).await.expect("ring frame");
    let (_, writes, commits, _) = r.pad.report_counts();
    assert_eq!((writes, commits), (3, 1), "one still: the record in three chunks and a commit");
    assert_eq!(r.server.custom_mode_writes(0), 0, "OpenRGB heard nothing of the ring frame");
    // The same colours again send nothing more.
    r.composite.write_frame(&ring_device_id(), &frame).await.expect("ring frame again");
    assert_eq!(r.pad.report_counts().2, 1);
}

#[tokio::test]
async fn with_the_write_gate_off_a_scene_change_through_the_composite_sends_the_pad_nothing() {
    let mut r = rig().await;
    r.composite.connect().await.expect("connect");
    assert!(!r.handle.writes_enabled(), "the daemon starts with the gate off");
    for n in 0..4u8 {
        let frame = vec![Rgb8::new(n, 9, 9); RING_LEDS];
        r.composite.write_frame(&ring_device_id(), &frame).await.expect("a held frame is not an error");
    }
    assert!(r.pad.sent_reports().is_empty(), "gate off: the pad was sent no report at all");
}

#[tokio::test]
async fn a_ring_device_without_the_ring_adapter_is_not_connected_and_the_rings_cannot_be_resized() {
    let server = FakeOpenRgbServer::start(vec![stick()]).await.expect("the fake OpenRGB server");
    let mut bare = CompositeAdapter::openrgb_only(OpenRgbAdapter::new("127.0.0.1", server.port()));
    let err = bare.write_frame(&ring_device_id(), &[Rgb8::BLACK; RING_LEDS]).await.unwrap_err();
    assert!(matches!(err, canvas_proto::traits::AdapterError::NotConnected));
    let mut r = rig().await;
    let err = r.composite.set_zone_size(&ring_device_id(), 0, 8).await.unwrap_err();
    assert!(matches!(err, canvas_proto::traits::AdapterError::Unsupported(_)));
}

#[test]
fn the_ring_idle_rule_is_the_daemons_own_peripheral_idle_rule() {
    assert_eq!(IDLE_BEFORE_WRITE, Duration::from_secs(PERIPHERAL_IDLE_READ_S));
}
