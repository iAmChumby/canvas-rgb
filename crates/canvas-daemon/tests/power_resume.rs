//! #5: a resume event must make every OpenRGB controller re-enter custom mode, because a sleep
//! drops the ENE DRAM sticks back to their built-in rainbow.

mod common;

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_daemon::power::{self, PowerEvent, PowerWatcher};
use canvas_daemon::state::App;
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::{DeviceAdapter, DeviceId, Rgb8};
use common::{FakeAdapter, FakeEngine};
use std::sync::Arc;

const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };

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

#[tokio::test]
async fn the_power_dispatcher_turns_a_resume_into_the_adapter_reset() {
    let home = std::env::temp_dir().join(format!("canvas-rgb-power-resume-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        canvas_daemon::storage::Store::at(&home),
        Arc::new(FakeEngine::default()),
        Box::new(FakeAdapter::default()),
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
        "0.0.0-test".into(),
        tx,
    ));

    let server = FakeOpenRgbServer::start(vec![stick()]).await.expect("fake server");
    let mut adapter = OpenRgbAdapter::new("127.0.0.1", server.port());
    adapter.connect().await.expect("connect");
    let id = DeviceId("openrgb:dram".into());
    adapter.write_frame(&id, &[RED; 8]).await.expect("frame");
    adapter.write_frame(&id, &[RED; 8]).await.expect("frame");
    let _ = adapter.read_back(&id).await.expect("readback orders the frames");
    assert_eq!(server.custom_mode_writes(0), 1);

    let mut watcher = PowerWatcher::new();
    power::handle(&app, &mut watcher, PowerEvent::Suspend).await;
    assert_eq!(server.custom_mode_writes(0), 1, "a suspend resets nothing");
    power::handle(&app, &mut watcher, PowerEvent::Resume).await;

    adapter.write_frame(&id, &[RED; 8]).await.expect("frame after resume");
    let _ = adapter.read_back(&id).await.expect("readback orders the frame");
    assert_eq!(server.custom_mode_writes(0), 2, "the resume made the next frame re-send SET_CUSTOM_MODE");
    let _ = std::fs::remove_dir_all(&home);
}
