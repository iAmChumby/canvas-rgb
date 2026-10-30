//! OpenRGB supervision: process lifecycle over fake spawns, the install-root check, and the
//! boot color over the real adapter talking to `FakeOpenRgbServer` (127.0.0.1 only).

use crate::fakes::{FakeProcesses, FakeSpawner};
use crate::openrgb::{server_args, OpenRgbSupervisor, SERVER_HOST};
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::pipe::OpenRgbServerState;
use canvas_proto::Rgb8;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tempfile::TempDir;

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

/// An install root that exists, and the OpenRGB exe path inside it (the exe itself is never
/// created; the fake spawner does not need it).
fn install() -> (TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let exe = root.path().join("openrgb").join("OpenRGB.exe");
    (root, exe)
}

fn supervisor(processes: &FakeProcesses, root: &TempDir, exe: &Path, port: u16, factory_port: Option<u16>) -> OpenRgbSupervisor {
    let factory_port = factory_port.unwrap_or(port);
    OpenRgbSupervisor::new(
        exe.to_path_buf(),
        port,
        root.path().join("openrgb-config"),
        root.path().to_path_buf(),
        Box::new(FakeSpawner(processes.clone())),
        Box::new(move || Box::new(OpenRgbAdapter::new(SERVER_HOST, factory_port))),
    )
}

#[test]
fn the_server_binds_loopback_with_no_autoconnect_and_its_own_config() {
    let args = server_args(6742, Path::new(r"C:\ProgramData\canvas-rgb\openrgb"));
    let joined = args.join(" ");
    assert!(joined.contains("--server-host 127.0.0.1"), "{joined}");
    assert!(joined.contains("--server-port 6742"));
    assert!(joined.contains("--noautoconnect"));
    assert!(joined.contains(r"--config C:\ProgramData\canvas-rgb\openrgb"));
}

#[tokio::test]
async fn the_server_restarts_when_it_exits() {
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    // Port 1 has no server, so the connect fails and only the process lifecycle is exercised.
    let mut sup = supervisor(&processes, &root, &exe, 6742, Some(1));
    sup.tick(Instant::now(), false, None).await;
    assert_eq!(sup.starts(), 1);
    assert_eq!(sup.state(), OpenRgbServerState::Starting);

    processes.exit_last();
    sup.tick(Instant::now(), false, None).await;
    assert_eq!(sup.starts(), 2, "an exited server is started again on the next tick");
    let spawns = processes.spawns();
    assert_eq!(spawns[1].0, exe);
    assert!(spawns[1].1.contains(&"--noautoconnect".to_string()));
}

#[tokio::test]
async fn a_restart_request_stops_and_starts_the_server() {
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, 6742, Some(1));
    sup.tick(Instant::now(), false, None).await;
    sup.restart("test");
    assert_eq!(sup.state(), OpenRgbServerState::Crashed);
    sup.tick(Instant::now(), false, None).await;
    assert_eq!(sup.starts(), 2);
}

#[tokio::test]
async fn an_exe_outside_the_install_directory_is_never_launched() {
    let processes = FakeProcesses::new();
    let (root, _exe) = install();
    let elsewhere = tempfile::tempdir().unwrap();
    let rogue = elsewhere.path().join("OpenRGB.exe");
    let mut sup = supervisor(&processes, &root, &rogue, 6742, Some(1));
    sup.tick(Instant::now(), false, None).await;
    assert!(processes.spawns().is_empty(), "nothing was spawned");
    assert_eq!(sup.state(), OpenRgbServerState::NotStarted);
}

#[tokio::test]
async fn a_disabled_supervisor_never_spawns() {
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, 6742, Some(1));
    sup.disable("machine home not secured");
    sup.tick(Instant::now(), false, None).await;
    assert!(processes.spawns().is_empty());
}

#[tokio::test]
async fn the_boot_color_is_written_to_every_device_when_no_daemon_is_connected() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 4), device(1, 6)]).await.unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);
    let boot = Rgb8::new(10, 20, 30);
    let now = Instant::now();
    sup.tick(now, false, Some(boot)).await;
    assert_eq!(sup.state(), OpenRgbServerState::Running);
    assert!(!sup.boot_written(), "the count has been seen once; it must hold still before the write");
    // The service ticks once a second: the second tick sees the same count and writes.
    sup.tick(now + Duration::from_secs(1), false, Some(boot)).await;
    assert_eq!(wait_for_colors(&fake, 0, &[boot; 4]).await, vec![boot; 4]);
    assert_eq!(wait_for_colors(&fake, 1, &[boot; 6]).await, vec![boot; 6]);
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
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    last
}

#[tokio::test]
async fn an_absent_boot_color_means_black() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 3)]).await.unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);
    let now = Instant::now();
    sup.tick(now, false, None).await;
    sup.tick(now + Duration::from_secs(1), false, None).await;
    assert_eq!(wait_for_colors(&fake, 0, &[Rgb8::BLACK; 3]).await, vec![Rgb8::BLACK; 3]);
}

#[tokio::test]
async fn the_boot_color_is_not_written_while_a_daemon_is_connected() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 3)]).await.unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);
    sup.tick(Instant::now(), true, Some(Rgb8::new(9, 9, 9))).await;
    assert_eq!(sup.state(), OpenRgbServerState::Running);
    assert_eq!(fake.colors(0), vec![Rgb8::BLACK; 3], "the daemon owns the devices");
}

// ---- #18: the boot color waits for OpenRGB to finish detecting ----

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A log sink the tests read back.
#[derive(Clone, Default)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl LogBuf {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
    fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync {
        let buf = self.clone();
        tracing_subscriber::fmt().with_ansi(false).with_writer(move || buf.clone()).finish()
    }
}

#[tokio::test]
async fn the_boot_color_waits_until_openrgb_has_found_its_devices() {
    // Real OpenRGB announced its first controller ~1.2 s after start and finished at ~2.1 s.
    // The fake compresses that: nothing for 250 ms, then one device every 250 ms.
    let fake = FakeOpenRgbServer::start_detecting(
        vec![device(0, 4), device(1, 6), device(2, 3)],
        Duration::from_millis(250),
        Duration::from_millis(300),
    )
    .await
    .unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);
    let boot = Rgb8::new(10, 20, 30);

    // The first tick connects at once, while the server still lists no device. Before the fix
    // it wrote to nobody here and never wrote again.
    sup.tick(Instant::now(), false, Some(boot)).await;
    assert_eq!(sup.state(), OpenRgbServerState::Running);
    assert!(!sup.boot_written(), "no device is detected yet, so nothing was painted");

    // Real time passes between ticks, so the fake's devices appear one by one (the count is
    // 0, 1, 2, then 3) and the write must wait for the count to hold still.
    for _ in 0..40 {
        if sup.boot_written() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        sup.tick(Instant::now(), false, Some(boot)).await;
    }
    assert!(sup.boot_written(), "the boot color was written once detection settled");
    assert_eq!(wait_for_colors(&fake, 0, &[boot; 4]).await, vec![boot; 4]);
    assert_eq!(wait_for_colors(&fake, 1, &[boot; 6]).await, vec![boot; 6]);
    assert_eq!(wait_for_colors(&fake, 2, &[boot; 3]).await, vec![boot; 3], "the last device to appear is painted too");
}

#[tokio::test]
async fn a_pc_with_no_devices_logs_that_nothing_was_painted() {
    let logs = LogBuf::default();
    let _default = tracing::subscriber::set_default(logs.subscriber());
    let fake = FakeOpenRgbServer::start(Vec::new()).await.unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);
    let mut now = Instant::now();
    for _ in 0..8 {
        sup.tick(now, false, Some(Rgb8::new(1, 2, 3))).await;
        now += Duration::from_secs(1);
    }
    assert!(!sup.boot_written(), "nothing was painted, so the boot color is not marked written");
    let text = logs.text();
    assert!(text.contains("WARN"), "{text}");
    assert!(text.contains("0 devices") && text.contains("nothing was written"), "{text}");
}

#[test]
fn a_write_to_zero_devices_is_a_warning_and_a_write_to_some_is_info() {
    let logs = LogBuf::default();
    tracing::subscriber::with_default(logs.subscriber(), || {
        crate::openrgb::log_device_write("painted the test color", 0);
        crate::openrgb::log_device_write("painted the test color", 3);
    });
    let text = logs.text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].contains("WARN") && lines[0].contains("0 devices"), "{}", lines[0]);
    assert!(lines[1].contains("INFO") && lines[1].contains("3 devices"), "{}", lines[1]);
}
