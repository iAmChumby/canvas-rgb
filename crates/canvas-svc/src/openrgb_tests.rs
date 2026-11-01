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

/// A sleep inside the watch window must not end the window: `Instant` may have counted the
/// sleep, and a controller OpenRGB announces after the wake still gets the color (#29, class
/// `cause:sleep-state`).
#[tokio::test]
async fn a_resume_starts_the_boot_color_watch_window_again() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 4)]).await.unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);
    let boot = Rgb8::new(10, 20, 30);
    let now = Instant::now();
    sup.tick(now, false, Some(boot)).await;
    sup.tick(now + Duration::from_secs(1), false, Some(boot)).await;
    assert_eq!(wait_for_colors(&fake, 0, &[boot; 4]).await, vec![boot; 4]);
    assert!(sup.boot_pending(), "the watch window runs after the first write");

    // The machine sleeps for an hour. The clock may have counted it.
    let woke = now + Duration::from_secs(3600);
    sup.on_resume(woke);
    sup.tick(woke, false, Some(boot)).await;
    assert!(sup.boot_pending(), "the watch window ended at the wake");

    // A controller OpenRGB announces after the wake still gets the boot color.
    fake.announce(device(1, 6));
    sup.tick(woke + Duration::from_secs(1), false, Some(boot)).await;
    assert_eq!(
        wait_for_colors(&fake, 1, &[boot; 6]).await,
        vec![boot; 6],
        "the controller announced after the wake did not get the boot color"
    );
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

// ---- #18 review: late controllers, the unsettled cap, and the restart-limit black ----

/// Polls `fake.custom_mode_writes(index)` until it is above zero (the adapter writes a device's
/// colors only after switching it to custom mode), or gives up after 2 s.
async fn wait_for_write(fake: &FakeOpenRgbServer, index: u32) -> u32 {
    let mut writes = 0;
    for _ in 0..100 {
        writes = fake.custom_mode_writes(index);
        if writes > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    writes
}

/// Real OpenRGB announces controllers over seconds. Here devices 0 and 1 settle and get the boot
/// color, and device 2 is announced about 1.6 s after that, at the service's real 1 s tick. Before
/// the fix the supervisor locked `boot_written` after the first stable tick and never saw it.
#[tokio::test]
async fn a_controller_announced_after_the_boot_write_is_painted_too() {
    let fake = FakeOpenRgbServer::start_detecting(
        vec![device(0, 4), device(1, 6), device(2, 3)],
        Duration::from_millis(200),
        Duration::from_millis(1650),
    )
    .await
    .unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);
    let boot = Rgb8::new(10, 20, 30);

    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    let mut written_with_two = false;
    for _ in 0..10 {
        ticker.tick().await;
        sup.tick(Instant::now(), false, Some(boot)).await;
        if sup.boot_written() && fake.colors(2) != vec![boot; 3] {
            written_with_two = true;
        }
        if fake.colors(2) == vec![boot; 3] {
            break;
        }
    }
    assert!(written_with_two, "the boot color was written while device 2 was still unannounced");
    assert_eq!(wait_for_colors(&fake, 0, &[boot; 4]).await, vec![boot; 4]);
    assert_eq!(wait_for_colors(&fake, 1, &[boot; 6]).await, vec![boot; 6]);
    assert_eq!(wait_for_colors(&fake, 2, &[boot; 3]).await, vec![boot; 3], "the late controller got the boot color");
}

/// The 5 s cap used to warn only for a count of zero. A count that is above zero but still
/// changing at the cap now logs how many devices OpenRGB reports.
#[tokio::test]
async fn an_unsettled_nonzero_count_at_the_cap_is_logged_with_the_count() {
    let logs = LogBuf::default();
    let _default = tracing::subscriber::set_default(logs.subscriber());
    // A device is announced every second, so the count differs at every tick and never settles.
    let devices: Vec<_> = (0..9).map(|i| device(i, 2)).collect();
    let fake = FakeOpenRgbServer::start_detecting(devices, Duration::from_millis(500), Duration::from_secs(1))
        .await
        .unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);

    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    for _ in 0..7 {
        ticker.tick().await;
        sup.tick(Instant::now(), false, Some(Rgb8::new(1, 2, 3))).await;
    }
    assert!(!sup.boot_written(), "the count never held still, so nothing was painted");
    let text = logs.text();
    let line = text
        .lines()
        .find(|l| l.contains("not written: OpenRGB has found"))
        .unwrap_or_else(|| panic!("no warning at the cap for a nonzero, unsettled count:\n{text}"));
    assert!(line.contains("WARN") && line.contains("nothing was written"), "{line}");
    assert!(!line.contains("found 0 devices"), "the count is above zero: {line}");
    assert_eq!(text.matches("not written: OpenRGB has found").count(), 1, "the cap warns once:\n{text}");
}

/// The restart-limit black used to discover once and paint whoever was listed (nobody, while
/// OpenRGB was still detecting), then forget. It now waits for detection like the boot color.
#[tokio::test]
async fn the_restart_limit_black_waits_for_detection_and_paints_late_devices() {
    let logs = LogBuf::default();
    let _default = tracing::subscriber::set_default(logs.subscriber());
    let fake = FakeOpenRgbServer::start_detecting(
        vec![device(0, 4), device(1, 6)],
        Duration::from_millis(250),
        Duration::from_millis(300),
    )
    .await
    .unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);

    sup.request_black(Instant::now());
    assert!(sup.black_pending());
    // The first tick connects while the server lists no device: nothing may be written, and the
    // old "wrote black ... to 0 devices" outcome must not be logged.
    sup.tick(Instant::now(), false, Some(Rgb8::new(9, 9, 9))).await;
    assert_eq!(fake.custom_mode_writes(0), 0);
    assert!(!logs.text().contains("0 devices"), "{}", logs.text());

    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    for _ in 0..40 {
        ticker.tick().await;
        sup.tick(Instant::now(), false, Some(Rgb8::new(9, 9, 9))).await;
        if fake.custom_mode_writes(0) > 0 && fake.custom_mode_writes(1) > 0 {
            break;
        }
    }
    assert!(wait_for_write(&fake, 0).await > 0, "device 0 got the black write");
    assert!(wait_for_write(&fake, 1).await > 0, "device 1 got the black write");
    let text = logs.text();
    assert!(text.contains("black after the daemon restart limit written to"), "{text}");
    // The boot color (9,9,9) never went out: black replaced it.
    assert_eq!(fake.colors(0), vec![Rgb8::BLACK; 4]);
}

// ---- #18 review: the end of the 15 s watch window is logged ----

/// Drives a `PaintJob` on synthetic clocks against a live fake: settles, writes, runs `between`,
/// then jumps `WATCH_WINDOW` past the first write. Returns the log text.
async fn run_job_to_window_end(fake: &FakeOpenRgbServer, between: Duration) -> String {
    use crate::openrgb::{PaintJob, WATCH_WINDOW};
    use canvas_proto::traits::DeviceAdapter;
    let logs = LogBuf::default();
    let _default = tracing::subscriber::set_default(logs.subscriber());
    let mut session = OpenRgbAdapter::new(SERVER_HOST, fake.port());
    session.connect().await.unwrap();
    let mut job = PaintJob::new("boot color", Rgb8::new(1, 2, 3), None);
    let t0 = Instant::now();
    assert!(!job.step(t0, &mut session).await);
    let t1 = t0 + Duration::from_secs(1);
    assert!(!job.step(t1, &mut session).await);
    assert!(!job.painted.is_empty(), "the first write was made");
    tokio::time::sleep(between).await;
    assert!(job.step(t1 + WATCH_WINDOW, &mut session).await, "the window has ended");
    logs.text()
}

#[tokio::test]
async fn the_watch_window_ending_with_all_devices_painted_says_so() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 4), device(1, 2)]).await.unwrap();
    let text = run_job_to_window_end(&fake, Duration::ZERO).await;
    let line = text.lines().find(|l| l.contains("watch ended")).unwrap_or_else(|| panic!("no window-end line:
{text}"));
    assert!(line.contains("INFO") && line.contains("15 s") && line.contains("all 2 devices were painted"), "{line}");
}

#[tokio::test]
async fn the_watch_window_ending_with_an_unpainted_device_warns_with_the_count() {
    // Device 0 is listed at once; device 1 is announced 300 ms later, after the first write and
    // before the window's last pass, which only counts and does not paint.
    let fake = FakeOpenRgbServer::start_detecting(
        vec![device(0, 4), device(1, 2)],
        Duration::from_millis(50),
        Duration::from_millis(300),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let text = run_job_to_window_end(&fake, Duration::from_millis(500)).await;
    let line = text.lines().find(|l| l.contains("watch ended")).unwrap_or_else(|| panic!("no window-end line:
{text}"));
    assert!(line.contains("WARN") && line.contains("1 devices are still unpainted") && line.contains("15 s"), "{line}");
}
