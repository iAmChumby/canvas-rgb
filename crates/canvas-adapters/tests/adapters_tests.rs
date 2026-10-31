use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_adapters::zones::ZoneStore;
use canvas_proto::{AdapterError, DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8, ZoneInfo, ZoneKind};
use std::time::Duration;

const WHITE: Rgb8 = Rgb8 { r: 255, g: 255, b: 255 };
const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };
const BLACK: Rgb8 = Rgb8 { r: 0, g: 0, b: 0 };

fn device(name: &str, serial: &str, kind: DeviceKind, zones: Vec<ZoneInfo>) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(format!("openrgb:{serial}")),
        name: name.into(), vendor: "Fake".into(), kind, adapter: "openrgb".into(),
        adapter_index: 0, serial: serial.into(), location: "usb".into(), zones,
        connected: true,
    }
}

fn fixed(name: &str, count: u32, kind: ZoneKind) -> ZoneInfo {
    ZoneInfo { index: 0, name: name.into(), kind, led_count: count, min_leds: count, max_leds: count, resizable: false }
}

fn linear(name: &str, count: u32) -> ZoneInfo {
    fixed(name, count, ZoneKind::Linear)
}

/// A JARGB header: starts at 0 LEDs and accepts any size up to 300.
fn jargb(name: &str) -> ZoneInfo {
    ZoneInfo { index: 0, name: name.into(), kind: ZoneKind::Linear, led_count: 0,
        min_leds: 0, max_leds: 300, resizable: true }
}

/// The five devices a typical test PC reports through OpenRGB.
fn test_pc_devices() -> Vec<DeviceInfo> {
    vec![
        device("MSI MPG B850 Mystic Light", "mb", DeviceKind::Motherboard, vec![
            jargb("JARGB1"), jargb("JARGB2"), jargb("JARGB3"), linear("JRGB1", 3)]),
        device("Epomaker ROYUAN", "kb", DeviceKind::Keyboard, vec![ZoneInfo {
            index: 0, name: "Keys".into(),
            kind: ZoneKind::Matrix { width: 2, height: 2, map: vec![Some(0), Some(1), None, Some(2)] },
            led_count: 3, min_leds: 3, max_leds: 3, resizable: false }]),
        device("HyperX QuadCast S", "mic", DeviceKind::Microphone, vec![fixed("Logo", 1, ZoneKind::Single)]),
        device("DDR5 A", "dram-a", DeviceKind::Dram, vec![linear("RGB", 8)]),
        device("DDR5 B", "dram-b", DeviceKind::Dram, vec![linear("RGB", 8)]),
    ]
}

async fn connected(server: &FakeOpenRgbServer) -> OpenRgbAdapter {
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect to fake server");
    adapter
}

fn temp_zones_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("canvas-rgb-adapters-{}-{name}.json", std::process::id()))
}

#[tokio::test]
async fn discover_reports_all_five_devices_with_zone_detail() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let mut adapter = connected(&server).await;
    let found = adapter.discover().await.expect("discover");

    assert_eq!(found.len(), 5);
    assert_eq!(found[0].id, DeviceId("openrgb:mb".into()));
    assert_eq!(found[0].kind, DeviceKind::Motherboard);
    assert_eq!(found[1].kind, DeviceKind::Keyboard);
    assert_eq!(found[2].kind, DeviceKind::Microphone);
    assert_eq!(found[3].kind, DeviceKind::Dram);
    assert_eq!(found[0].serial, "mb");
    assert_eq!(found[0].location, "usb");
    assert_eq!(found[0].zones[0].name, "JARGB1");
    assert!(found[0].zones[0].resizable && found[0].zones[0].led_count == 0);
    assert!(!found[0].zones[3].resizable && found[0].zones[3].led_count == 3);
    assert_eq!(found[1].zones[0].kind, ZoneKind::Matrix {
        width: 2, height: 2, map: vec![Some(0), Some(1), None, Some(2)] });
    assert_eq!(found[2].zones[0].kind, ZoneKind::Single);
    assert_eq!(found[3].led_count(), 8);
}

#[tokio::test]
async fn device_id_falls_back_to_location_without_serial() {
    let mut no_serial = device("Strip", "", DeviceKind::LedStrip, vec![linear("S", 4)]);
    no_serial.location = "HID: /dev/strip".into();
    let server = FakeOpenRgbServer::start(vec![no_serial]).await.expect("fake server");
    let mut adapter = connected(&server).await;
    let found = adapter.discover().await.expect("discover");
    assert_eq!(found[0].id, DeviceId("openrgb:HID: /dev/strip".into()));
}

#[tokio::test]
async fn jargb_sizes_show_after_resize_and_frames_reach_hardware() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let mut adapter = connected(&server).await;
    let mb = DeviceId("openrgb:mb".into());
    adapter.set_zone_size(&mb, 0, 30).await.expect("JARGB1");
    adapter.set_zone_size(&mb, 1, 60).await.expect("JARGB2");
    adapter.set_zone_size(&mb, 2, 12).await.expect("JARGB3");

    let found = adapter.discover().await.expect("discover");
    assert!(found.iter().all(|device| device.led_count() > 0), "every device has LEDs after sizing");
    assert_eq!(found[0].zones[0].led_count, 30);
    assert_eq!(found[0].zones[1].led_count, 60);
    assert_eq!(found[0].zones[2].led_count, 12);

    let dram = &found[3];
    adapter.write_frame(&dram.id, &[RED; 8]).await.expect("write red");
    adapter.write_frame(&dram.id, &[RED; 8]).await.expect("write red again");
    let readback = adapter.read_back(&dram.id).await.expect("readback");
    assert_eq!(readback, vec![RED; 8]);
    assert_eq!(server.custom_mode_writes(3), 1, "SETCUSTOMMODE sent once, not per frame");
}

#[tokio::test]
async fn resize_is_clamped_to_the_zone_range_and_fixed_zones_refuse() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let mut adapter = connected(&server).await;
    let mb = DeviceId("openrgb:mb".into());
    adapter.set_zone_size(&mb, 0, 9_999).await.expect("clamped to max");
    assert_eq!(server.zone_leds(0, 0), Some(300));

    let fixed_zone = adapter.set_zone_size(&mb, 3, 5).await.expect_err("JRGB1 is fixed");
    assert!(matches!(fixed_zone, AdapterError::Zone { zone: 3, .. }));
    let unknown = adapter.set_zone_size(&DeviceId("openrgb:nope".into()), 0, 5).await.expect_err("unknown device");
    assert!(matches!(unknown, AdapterError::UnknownDevice(_)));
}

#[tokio::test]
async fn saved_zone_sizes_are_reapplied_after_restart() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let mut adapter = connected(&server).await;
    let path = temp_zones_path("reapply");
    let mut store = ZoneStore::load(&path).expect("load missing file as empty");
    let mb = DeviceId("openrgb:mb".into());

    adapter.set_zone_size(&mb, 0, 24).await.expect("resize");
    store.set(&mb, 0, 24);
    store.save().expect("save zones.json");

    server.restart().await;
    wait_until_dropped(&adapter).await;
    adapter.connect().await.expect("reconnect");
    let devices = adapter.discover().await.expect("discover after restart");
    assert_eq!(devices[0].zones[0].led_count, 0, "OpenRGB forgot the resize");

    let reloaded = ZoneStore::load(&path).expect("reload zones.json");
    let results = reloaded.apply(&mut adapter, &devices).await;
    assert_eq!(results.len(), 1);
    assert!(results.iter().all(|(_, _, result)| result.is_ok()));

    let devices = adapter.discover().await.expect("rediscover");
    assert_eq!(devices[0].zones[0].led_count, 24, "saved size re-applied on connect");
    let _ = std::fs::remove_file(&path);
}

/// After `restart()` the socket is gone; waits (up to 2s) for the adapter to notice.
async fn wait_until_dropped(adapter: &OpenRgbAdapter) {
    for _ in 0..100 {
        if !adapter.is_connected() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("adapter did not notice the dropped socket");
}

#[tokio::test]
async fn dropped_socket_flips_is_connected_and_calls_fail_until_reconnect() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let mut adapter = connected(&server).await;
    assert!(adapter.is_connected());
    server.restart().await;
    wait_until_dropped(&adapter).await;
    let error = adapter.write_frame(&DeviceId("openrgb:dram-a".into()), &[RED; 8]).await
        .expect_err("write fails while disconnected");
    assert!(matches!(error, AdapterError::NotConnected));
    let error = adapter.discover().await.expect_err("discover fails while disconnected");
    assert!(matches!(error, AdapterError::NotConnected));

    adapter.connect().await.expect("reconnect");
    assert!(adapter.is_connected());
    assert_eq!(adapter.discover().await.expect("discover").len(), 5);
}

#[tokio::test]
async fn discover_before_connect_is_not_connected() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    assert!(!adapter.is_connected());
    let error = adapter.discover().await.expect_err("not connected");
    assert!(matches!(error, AdapterError::NotConnected));
}

#[tokio::test]
async fn identify_turns_only_one_led_white() {
    let server = FakeOpenRgbServer::start(vec![
        device("MSI", "mb", DeviceKind::Motherboard, vec![linear("RGB", 3)]),
    ]).await.expect("fake server");
    let mut adapter = connected(&server).await;
    let devices = adapter.discover().await.expect("discover");
    adapter.identify(&devices[0].id, 0, 1).await.expect("identify");
    // UPDATELEDS has no reply; a READ on the same socket is answered after it was processed.
    let readback = adapter.read_back(&devices[0].id).await.expect("readback");
    assert_eq!(readback, vec![BLACK, WHITE, BLACK]);
    assert_eq!(server.colors(0), readback);

    let out_of_range = adapter.identify(&devices[0].id, 0, 3).await.expect_err("LED 3 of 3");
    assert!(matches!(out_of_range, AdapterError::Zone { .. }));
}

#[tokio::test]
async fn identify_offsets_into_the_second_zone() {
    let server = FakeOpenRgbServer::start(vec![
        device("MSI", "mb", DeviceKind::Motherboard, vec![linear("A", 2), linear("B", 3)]),
    ]).await.expect("fake server");
    let mut adapter = connected(&server).await;
    let devices = adapter.discover().await.expect("discover");
    adapter.identify(&devices[0].id, 1, 2).await.expect("identify");
    let readback = adapter.read_back(&devices[0].id).await.expect("readback");
    assert_eq!(readback, vec![BLACK, BLACK, BLACK, BLACK, WHITE]);
}

#[tokio::test]
async fn two_devices_written_from_two_tasks_both_land() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let adapter = connected(&server).await;
    let mut discovered = adapter.clone();
    let devices = discovered.discover().await.expect("discover");
    let dram_a = devices[3].id.clone();
    let dram_b = devices[4].id.clone();

    let mut task_a = adapter.clone();
    let mut task_b = adapter.clone();
    let frames_a = vec![RED; 8];
    let frames_b = vec![Rgb8 { r: 0, g: 0, b: 255 }; 8];
    let (a, b) = tokio::join!(
        async { for _ in 0..50 { task_a.write_frame(&dram_a, &frames_a).await.expect("A"); } },
        async { for _ in 0..50 { task_b.write_frame(&dram_b, &frames_b).await.expect("B"); } },
    );
    let _ = (a, b);
    assert_eq!(discovered.read_back(&dram_a).await.expect("readback A"), frames_a);
    assert_eq!(discovered.read_back(&dram_b).await.expect("readback B"), frames_b);
    assert_eq!(server.colors(3), frames_a);
    assert_eq!(server.colors(4), frames_b);
}

#[tokio::test]
async fn frame_length_must_match_the_device() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let mut adapter = connected(&server).await;
    let devices = adapter.discover().await.expect("discover");
    let error = adapter.write_frame(&devices[3].id, &[RED; 7]).await.expect_err("7 colors for 8 LEDs");
    assert!(matches!(error, AdapterError::Protocol(_)));
}

#[tokio::test]
async fn capabilities_match_the_brief() {
    let adapter = OpenRgbAdapter::new("127.0.0.1", 1);
    let caps = adapter.capabilities();
    assert!(caps.resizable_zones && caps.identify);
    assert!(!caps.hardware_brightness && !caps.needs_elevation);
    assert_eq!(adapter.name(), "openrgb");
}

#[tokio::test]
async fn zone_store_round_trips_and_rejects_newer_schema() {
    let path = temp_zones_path("roundtrip");
    let mut store = ZoneStore::load(&path).expect("missing file loads empty");
    assert_eq!(store.get(&DeviceId("openrgb:mb".into()), 0), None);
    store.set(&DeviceId("openrgb:mb".into()), 0, 24);
    store.set(&DeviceId("openrgb:mb".into()), 2, 12);
    store.save().expect("save");
    assert!(!path.with_file_name(format!("{}.tmp", path.file_name().unwrap().to_string_lossy())).exists(),
        "temp file is renamed away");

    let loaded = ZoneStore::load(&path).expect("load");
    assert_eq!(loaded.get(&DeviceId("openrgb:mb".into()), 0), Some(24));
    assert_eq!(loaded.get(&DeviceId("openrgb:mb".into()), 2), Some(12));
    assert_eq!(loaded.file.schema_version, canvas_proto::config::ZONES_SCHEMA);

    std::fs::write(&path, br#"{"schema_version": 99, "sizes": {}}"#).expect("write future file");
    assert!(ZoneStore::load(&path).is_err(), "future schema refused");
    let _ = std::fs::remove_file(&path);
}

// ---- #5: a sleep resets the ENE DRAM sticks to their built-in mode

const GREEN: Rgb8 = Rgb8 { r: 0, g: 255, b: 0 };

#[tokio::test]
async fn after_a_resume_the_next_frame_to_each_controller_is_preceded_by_set_custom_mode() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let mut adapter = connected(&server).await;
    let dram_a = DeviceId("openrgb:dram-a".into());
    let dram_b = DeviceId("openrgb:dram-b".into());
    adapter.write_frame(&dram_a, &[RED; 8]).await.expect("frame a");
    adapter.write_frame(&dram_b, &[RED; 8]).await.expect("frame b");
    adapter.write_frame(&dram_a, &[RED; 8]).await.expect("frame a again");
    // Writes have no reply; a read_back on the same socket is ordered after them.
    let _ = adapter.read_back(&dram_b).await.expect("settle");
    assert_eq!((server.custom_mode_writes(3), server.custom_mode_writes(4)), (1, 1));

    // The PC sleeps and wakes: the daemon tells the adapter.
    adapter.reset_custom_mode();
    assert_eq!((server.custom_mode_writes(3), server.custom_mode_writes(4)), (1, 1), "a reset sends nothing by itself");

    adapter.write_frame(&dram_a, &[GREEN; 8]).await.expect("frame a after resume");
    adapter.write_frame(&dram_b, &[GREEN; 8]).await.expect("frame b after resume");
    // read_back is a request on the same socket, so it is ordered after both frames.
    assert_eq!(adapter.read_back(&dram_a).await.expect("readback a"), vec![GREEN; 8]);
    assert_eq!(server.custom_mode_writes(3), 2, "stick A re-entered custom mode once");
    assert_eq!(server.custom_mode_writes(4), 2, "stick B re-entered custom mode once");

    adapter.write_frame(&dram_a, &[GREEN; 8]).await.expect("steady frame");
    let _ = adapter.read_back(&dram_a).await.expect("readback");
    assert_eq!(server.custom_mode_writes(3), 2, "no further SET_CUSTOM_MODE while streaming");
}

#[tokio::test]
async fn the_keepalive_sends_set_custom_mode_on_schedule_without_stopping_frames() {
    let server = FakeOpenRgbServer::start(test_pc_devices()).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::with_keepalive("127.0.0.1", server.port(), Duration::from_millis(300));
    adapter.connect().await.expect("connect");
    let dram = DeviceId("openrgb:dram-a".into());

    adapter.write_frame(&dram, &[RED; 8]).await.expect("first frame");
    adapter.write_frame(&dram, &[RED; 8]).await.expect("second frame");
    let _ = adapter.read_back(&dram).await.expect("settle");
    assert_eq!(server.custom_mode_writes(3), 1, "inside the interval: no repeat");

    tokio::time::sleep(Duration::from_millis(400)).await;
    adapter.write_frame(&dram, &[GREEN; 8]).await.expect("frame after the interval");
    assert_eq!(adapter.read_back(&dram).await.expect("readback"), vec![GREEN; 8], "the frame still lands");
    assert_eq!(server.custom_mode_writes(3), 2, "the interval passed: the mode was sent again");

    adapter.write_frame(&dram, &[GREEN; 8]).await.expect("frame");
    let _ = adapter.read_back(&dram).await.expect("readback");
    assert_eq!(server.custom_mode_writes(3), 2, "the interval restarts after each send");
}

#[test]
fn the_default_keepalive_is_ten_seconds() {
    assert_eq!(canvas_adapters::openrgb::CUSTOM_MODE_KEEPALIVE, Duration::from_secs(10));
}
