//! Service-level tests: the core's tick, what "connected" means for the OpenRGB supervisor, and
//! the boot color while the daemon starts (#163).
//!
//! Everything runs on the fakes: a fake daemon launcher, a fake OpenRGB server, and a fan
//! controller with no chip behind it.

use crate::daemon::DaemonSupervisor;
use crate::fakes::{FakeLauncher, FakeProcesses, FakeSpawner};
use crate::fans::FanController;
use crate::openrgb::{OpenRgbSupervisor, SERVER_HOST};
use crate::service::{Core, PowerEvent};
use crate::test_logs::captured_async;
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_proto::config::FansFile;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::Rgb8;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// One OpenRGB device with `leds` LEDs in its single zone.
fn device(index: u32, leds: u32) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(format!("openrgb:test:{index}")),
        name: format!("Test device {index}"),
        vendor: "canvas-rgb tests".into(),
        kind: DeviceKind::Motherboard,
        adapter: "openrgb".into(),
        adapter_index: index,
        // The adapter's device id is `<adapter>:<serial>`, so each fake needs its own serial.
        serial: format!("TEST-{index}"),
        location: String::new(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "Zone".into(),
            kind: ZoneKind::Linear,
            led_count: leds,
            min_leds: leds,
            max_leds: leds,
            resizable: false,
        }],
        connected: true,
    }
}

/// The service core over the fakes: a fan controller with a boot color and no hardware behind it,
/// an OpenRGB supervisor pointed at the fake server, and a daemon supervisor with a fake launcher.
fn core(fake_port: u16, boot: Rgb8, home: &TempDir) -> (Core, FakeProcesses) {
    let processes = FakeProcesses::new();
    let fans = FanController::new(
        home.path().to_path_buf(),
        None,
        None,
        BTreeMap::new(),
        FansFile { boot_color: Some(boot), ..FansFile::default() },
        None,
    );
    let daemon = DaemonSupervisor::new(
        PathBuf::from(r"C:\svc\canvas-rgbd.exe"),
        Box::new(FakeLauncher(processes.clone())),
    );
    let openrgb = OpenRgbSupervisor::new(
        home.path().join("openrgb").join("OpenRGB.exe"),
        fake_port,
        home.path().join("openrgb-config"),
        home.path().to_path_buf(),
        Box::new(FakeSpawner(processes.clone())),
        Box::new(move || Box::new(OpenRgbAdapter::new(SERVER_HOST, fake_port))),
    );
    (Core::new(fans, openrgb, daemon), processes)
}

/// The adapter sends LED updates without waiting for a reply, and the fake server applies them
/// on its own task, so a test polls until the colors land (or gives up after 2 s).
async fn wait_for_colors(fake: &FakeOpenRgbServer, index: u32, expected: &[Rgb8]) -> Vec<Rgb8> {
    let mut last = Vec::new();
    for _ in 0..100 {
        last = fake.colors(index);
        if last == expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    last
}

/// A daemon that has not beaten yet is not connected, so the boot color is painted while it
/// starts (#163). On the branch before the fix the grace made it count as connected, and the
/// lights stayed dark until the daemon said it was up.
#[tokio::test]
async fn the_boot_color_is_painted_while_the_daemon_starts() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 4)]).await.unwrap();
    let home = tempfile::tempdir().unwrap();
    let boot = Rgb8::new(10, 20, 30);
    let (mut core, _processes) = core(fake.port(), boot, &home);
    let t0 = Instant::now();
    core.daemon.start(t0);
    assert!(!core.daemon.is_connected(t0), "a daemon that has not beaten is not connected");
    // The first tick connects to OpenRGB; the device count must hold still before the write.
    core.tick(t0).await;
    assert!(!core.openrgb.boot_written(), "the boot color was written before the detection settled");
    // A second tick a second later: the count has held still, so the boot color goes out.
    core.tick(t0 + Duration::from_secs(1)).await;
    assert_eq!(
        wait_for_colors(&fake, 0, &[boot; 4]).await,
        vec![boot; 4],
        "the boot color was not painted while the daemon starts"
    );
}

/// A daemon that has beaten owns the devices: the boot color is not painted over its light.
#[tokio::test]
async fn a_daemon_that_has_beaten_suppresses_the_boot_color() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 4)]).await.unwrap();
    let home = tempfile::tempdir().unwrap();
    let boot = Rgb8::new(10, 20, 30);
    let (mut core, _processes) = core(fake.port(), boot, &home);
    let t0 = Instant::now();
    core.daemon.start(t0);
    // The daemon says it is up.
    core.daemon.on_heartbeat(t0);
    assert!(core.daemon.is_connected(t0), "a daemon that has beaten is connected");
    core.tick(t0).await;
    core.tick(t0 + Duration::from_secs(1)).await;
    assert_eq!(
        fake.colors(0),
        vec![Rgb8::BLACK; 4],
        "the boot color was painted while the daemon was connected"
    );
}

/// A resume is not a heartbeat (#163): it refreshes the daemon's clock without claiming the
/// daemon has beaten, and the daemon that is still starting up is not killed for it.
#[tokio::test]
async fn a_resume_is_not_a_first_heartbeat() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 4)]).await.unwrap();
    let home = tempfile::tempdir().unwrap();
    let boot = Rgb8::new(10, 20, 30);
    let (mut core, _processes) = core(fake.port(), boot, &home);
    let t0 = Instant::now();
    core.daemon.start(t0);
    let pid = core.daemon.daemon_pid().expect("a daemon is running");
    // The machine slept and woke. The daemon has not beaten yet.
    let (_, text) = captured_async(async {
        core.on_power(PowerEvent::Resume, t0 + Duration::from_secs(10));
    })
    .await;
    assert!(
        !text.contains("first heartbeat arrived"),
        "the resume logged a first-heartbeat measurement: {text}"
    );
    assert!(
        !core.daemon.is_connected(t0 + Duration::from_secs(10)),
        "the resume claimed the daemon has beaten"
    );
    // The daemon is still starting up: the grace holds, and it is not killed 5 s after the wake.
    core.tick(t0 + Duration::from_secs(31)).await;
    assert_eq!(core.daemon.daemon_pid(), Some(pid), "the daemon was killed after the resume");
    // A real beat after the resume is measured once, from the launch.
    let (_, text) = captured_async(async {
        core.daemon.on_heartbeat(t0 + Duration::from_secs(32));
    })
    .await;
    assert!(
        text.contains("the daemon's first heartbeat arrived 32000 ms after launch"),
        "{text}"
    );
}
