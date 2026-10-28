//! The paths that need the real wave-1 crates.
//!
//! These tests are `#[ignore]`d until `canvas-core`, `canvas-effects` and
//! `canvas-adapters` land; the spine owner un-ignores them then.
//!
//! They use the fake OpenRGB server, never real hardware.

#![cfg(target_os = "windows")]

use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::traits::{DeviceAdapter, EngineSettings};

fn device(name: &str, kind: DeviceKind, leds: u32) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(format!("fake:{name}")),
        name: name.to_string(),
        vendor: "fake".to_string(),
        kind,
        adapter: "openrgb".to_string(),
        adapter_index: 0,
        serial: name.to_string(),
        location: String::new(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "Zone 1".to_string(),
            kind: ZoneKind::Linear,
            led_count: leds,
            min_leds: 1,
            max_leds: 512,
            resizable: true,
        }],
        connected: true,
    }
}

#[tokio::test]
#[ignore = "needs wave-1 crates"]
async fn the_fake_openrgb_server_serves_the_devices() {
    let server = canvas_adapters::fake::FakeOpenRgbServer::start(vec![device(
        "MSI B850 Mystic Light",
        DeviceKind::Motherboard,
        120,
    )])
    .await
    .expect("the fake server starts");

    let mut adapter = canvas_adapters::openrgb::OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("the adapter connects");
    let devices = adapter.discover().await.expect("the devices are discovered");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].led_count(), 120);
}

#[tokio::test]
#[ignore = "needs wave-1 crates"]
async fn the_zone_sizes_survive_an_openrgb_restart() {
    let server = canvas_adapters::fake::FakeOpenRgbServer::start(vec![device(
        "MSI B850 Mystic Light",
        DeviceKind::Motherboard,
        120,
    )])
    .await
    .expect("the fake server starts");

    let mut adapter = canvas_adapters::openrgb::OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("the adapter connects");
    adapter
        .set_zone_size(&DeviceId("fake:MSI B850 Mystic Light".into()), 0, 60)
        .await
        .expect("the zone is resized");

    // OpenRGB forgets the resize when it restarts.
    server.restart().await;

    let mut sizes = std::collections::BTreeMap::new();
    sizes.insert(
        "fake:MSI B850 Mystic Light".to_string(),
        std::collections::BTreeMap::from([(0u32, 60u32)]),
    );
    let devices = canvas_daemon::openrgb_supervisor::connect_and_prepare(&mut adapter, &sizes)
        .await
        .expect("the reconnection succeeds");
    assert_eq!(devices[0].zones[0].led_count, 60);
}

#[tokio::test]
#[ignore = "needs wave-1 crates"]
async fn the_engine_parks_with_no_devices_and_no_preview() {
    let engine = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
    });
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let stats = engine.stats();
    assert!(stats.parked, "the engine parks when nothing consumes frames");
    assert_eq!(stats.frames_rendered, 0, "no frames are rendered while parked");
}

#[tokio::test]
#[ignore = "needs wave-1 crates"]
async fn the_effects_registry_has_the_eleven_native_effects() {
    let effects = canvas_effects::registry();
    assert_eq!(effects.len(), 11);
    let ids: Vec<String> = effects.iter().map(|f| f().schema().id).collect();
    for id in canvas_proto::EFFECT_IDS {
        assert!(ids.iter().any(|i| i == id), "{id} is in the registry");
    }
}

#[tokio::test]
#[ignore = "needs wave-1 crates"]
async fn a_native_effect_declares_its_default_properties() {
    let effect = canvas_effects::create("aurora").expect("aurora exists");
    let schema = effect.schema();
    assert_eq!(schema.id, "aurora");
    // The defaults come from the schema.
    let defaults = schema.defaults();
    assert!(!defaults.is_empty(), "aurora declares properties");
}
