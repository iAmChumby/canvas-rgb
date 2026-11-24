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
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// A test's reply deadline is a hang guard, not the verdict: a loaded runner must never turn a
/// slow-but-live reply from the fake into a failed connect and so a dark device (#519).
const TEST_REPLY_TIMEOUT: Duration = Duration::from_secs(60);

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
        Box::new(move || {
            Box::new(OpenRgbAdapter::new(SERVER_HOST, factory_port).with_reply_timeout(TEST_REPLY_TIMEOUT))
        }),
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
    sup.restart("test", crate::daemon::DaemonBeforeStop::NotRunning);
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
    assert_eq!(fake.wait_for_colors(0, &[boot; 4]).await, vec![boot; 4]);
    assert_eq!(fake.wait_for_colors(1, &[boot; 6]).await, vec![boot; 6]);
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
    assert_eq!(fake.wait_for_colors(0, &[Rgb8::BLACK; 3]).await, vec![Rgb8::BLACK; 3]);
}

/// OpenRGB's copy of the ROYUAN MonsGeek keyboard, whose id carries the HID path
/// `VID_3151&PID_4015`: the daemon never drives it (#393), and neither may the service (#413).
fn keyboard_copy(index: u32, leds: u32) -> DeviceInfo {
    let mut info = device(index, leds);
    info.serial = "HID: hid#VID_3151&PID_4015&MI_02#7&abc".into();
    info.id = DeviceId(format!("openrgb:{}", info.serial));
    info.kind = DeviceKind::Keyboard;
    info.name = "ROYUAN MonsGeek keyboard (OpenRGB copy)".into();
    info
}

/// The boot colour and the black paint go to the ordinary device and never to the keyboard's
/// OpenRGB copy, and the end of the watch window does not count the copy as unpainted (#413).
#[tokio::test]
async fn neither_the_boot_color_nor_the_black_paint_writes_to_the_keyboards_openrgb_copy() {
    let (_, text) = crate::test_logs::captured_async(async {
        let fake = FakeOpenRgbServer::start(vec![device(0, 4), keyboard_copy(1, 6)]).await.unwrap();
        let processes = FakeProcesses::new();
        let (root, exe) = install();
        let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);
        let boot = Rgb8::new(10, 20, 30);
        let start = Instant::now();
        // The first tick connects and reads the count; the second sees it hold still and paints.
        sup.tick(start, false, Some(boot)).await;
        sup.tick(start + Duration::from_secs(1), false, Some(boot)).await;

        assert_eq!(
            fake.wait_for_colors(0, &[boot; 4]).await,
            vec![boot; 4],
            "the ordinary device got the boot colour"
        );
        assert_eq!(fake.update_leds_writes(1), 0, "the boot colour was written to the keyboard's OpenRGB copy");
        assert_eq!(fake.custom_mode_writes(1), 0, "the copy was switched to custom mode by the boot colour");
        assert_eq!(fake.colors(1), vec![Rgb8::BLACK; 6], "the copy's lights were written");
        assert!(sup.boot_pending(), "the watch window runs after the first write");

        // The watch window ends: the copy is skipped, so it is not "still unpainted".
        sup.tick(start + Duration::from_secs(1) + crate::openrgb::WATCH_WINDOW, false, Some(boot)).await;
        assert!(!sup.boot_pending(), "the watch window ended");

        // The black paint after the daemon's restart limit follows the same rule.
        sup.request_black(Instant::now());
        let black = Instant::now();
        sup.tick(black, false, None).await;
        sup.tick(black + Duration::from_secs(1), false, None).await;

        assert_eq!(
            fake.wait_for_colors(0, &[Rgb8::BLACK; 4]).await,
            vec![Rgb8::BLACK; 4],
            "the ordinary device got the black paint"
        );
        assert_eq!(fake.update_leds_writes(1), 0, "the black paint was written to the keyboard's OpenRGB copy");
        assert_eq!(fake.custom_mode_writes(1), 0, "the copy was switched to custom mode by the black paint");
        assert_eq!(fake.colors(1), vec![Rgb8::BLACK; 6], "the copy's lights were written black");
    })
    .await;

    assert!(text.contains("boot color written to 1 devices"), "the boot colour reached one device: {text}");
    assert!(
        text.contains("all 1 devices were painted"),
        "the copy is not counted as unpainted at the end of the watch window: {text}"
    );
    assert!(
        text.contains("black after the daemon restart limit written to 1 devices"),
        "the black paint reached one device: {text}"
    );
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
    assert_eq!(fake.wait_for_colors(0, &[boot; 4]).await, vec![boot; 4]);
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
        fake.wait_for_colors(1, &[boot; 6]).await,
        vec![boot; 6],
        "the controller announced after the wake did not get the boot color"
    );
}

// ---- #18: the boot color waits for OpenRGB to finish detecting ----

use crate::test_logs::captured;

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
    assert_eq!(fake.wait_for_colors(0, &[boot; 4]).await, vec![boot; 4]);
    assert_eq!(fake.wait_for_colors(1, &[boot; 6]).await, vec![boot; 6]);
    assert_eq!(fake.wait_for_colors(2, &[boot; 3]).await, vec![boot; 3], "the last device to appear is painted too");
}

#[tokio::test]
async fn a_pc_with_no_devices_logs_that_nothing_was_painted() {
    let (_, text) = crate::test_logs::captured_async(async {
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
    })
    .await;
    assert!(text.contains("WARN"), "{text}");
    assert!(text.contains("0 devices") && text.contains("nothing was written"), "{text}");
}

#[test]
fn a_write_to_zero_devices_is_a_warning_and_a_write_to_some_is_info() {
    let (_, text) = captured(|| {
        crate::openrgb::log_device_write("painted the test color", 0);
        crate::openrgb::log_device_write("painted the test color", 3);
    });
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].contains("WARN") && lines[0].contains("0 devices"), "{}", lines[0]);
    assert!(lines[1].contains("INFO") && lines[1].contains("3 devices"), "{}", lines[1]);
}

// ---- #18 review: late controllers, the unsettled cap, and the restart-limit black ----

/// Waits until `fake.custom_mode_writes(index)` is above zero (the adapter writes a device's
/// colors only after switching it to custom mode), then returns the count. The wait is woken by
/// the fake's own changes, not by a clock, so its cap only ends a write that never lands.
async fn wait_for_write(fake: &FakeOpenRgbServer, index: u32) -> u32 {
    fake.wait_until("the custom-mode write landed", || fake.custom_mode_writes(index) > 0).await;
    fake.custom_mode_writes(index)
}

/// Real OpenRGB announces controllers over seconds. Devices 0 and 1 settle and get the boot
/// color; device 2 is announced only after that write. Before the fix the supervisor locked
/// `boot_written` after the first stable tick and never painted a controller found later.
/// The test announces it itself and steps the clock the supervisor reads, so the order does not
/// depend on how fast a shared runner gets through its 1 s ticks (class `cause:timing-test`, #178).
#[tokio::test]
async fn a_controller_announced_after_the_boot_write_is_painted_too() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 4), device(1, 6)]).await.unwrap();
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);
    let boot = Rgb8::new(10, 20, 30);

    // Connect first, as the #178 test does: the handshake is real I/O and a busy machine can
    // make it take a tick or two. The synthetic clock starts once the supervisor is up.
    let mut now = Instant::now();
    let mut connected = false;
    for _ in 0..30 {
        sup.tick(now, false, Some(boot)).await;
        if sup.state() == OpenRgbServerState::Running {
            connected = true;
            break;
        }
        now += Duration::from_secs(1);
    }
    assert!(connected, "the supervisor never reached the fake server");

    // Step the supervisor's own clock until the two settled devices have the boot color.
    for _ in 0..30 {
        now += Duration::from_secs(1);
        sup.tick(now, false, Some(boot)).await;
        if sup.boot_written() {
            break;
        }
    }
    assert!(sup.boot_written(), "the boot color was never written to the first two devices");
    assert_eq!(fake.wait_for_colors(0, &[boot; 4]).await, vec![boot; 4]);
    assert_eq!(fake.wait_for_colors(1, &[boot; 6]).await, vec![boot; 6]);

    // OpenRGB finds the controller now, after the boot write. One more tick paints it.
    fake.announce(device(2, 3));
    now += Duration::from_secs(1);
    sup.tick(now, false, Some(boot)).await;
    assert_eq!(
        fake.wait_for_colors(2, &[boot; 3]).await,
        vec![boot; 3],
        "the late controller got the boot color"
    );
}

/// The 5 s cap used to warn only for a count of zero. A count that is above zero but still
/// changing at the cap now logs how many devices OpenRGB reports.
#[tokio::test]
async fn an_unsettled_nonzero_count_at_the_cap_is_logged_with_the_count() {
    let (_, text) = crate::test_logs::captured_async(async {
        // The test announces the devices and steps the clock the supervisor reads, so the count
        // differs at every tick and the 5 s cap falls on the same tick whatever the machine's
        // speed (class `cause:timing-test`, #178). The old test let the fake server announce a
        // device a second and the supervisor read `Instant::now()`, so the verdict depended on
        // how long the ticks were late.
        let fake = FakeOpenRgbServer::start(Vec::new()).await.unwrap();
        let processes = FakeProcesses::new();
        let (root, exe) = install();
        let mut sup = supervisor(&processes, &root, &exe, fake.port(), None);

        // The supervisor connects on a tick of its own, and a busy machine can make that
        // handshake miss the adapter's 5 s reply timeout and take a tick or two. The job counts
        // its 5 s cap from the tick it starts on, so the connection is made first and the ticks
        // under test follow it: the cap then falls on the same tick every time.
        let mut now = Instant::now();
        let mut connected = false;
        for _ in 0..30 {
            sup.tick(now, false, Some(Rgb8::new(1, 2, 3))).await;
            if sup.state() == OpenRgbServerState::Running {
                connected = true;
                break;
            }
            now += Duration::from_secs(1);
        }
        assert!(connected, "the supervisor never reached the fake server");

        for index in 0..8u32 {
            // A device is announced every tick: the count differs at every tick and never settles.
            fake.announce(device(index, 2));
            now += Duration::from_secs(1);
            sup.tick(now, false, Some(Rgb8::new(1, 2, 3))).await;
        }
        assert!(!sup.boot_written(), "the count never held still, so nothing was painted");
    })
    .await;
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
    let (colors, text) = crate::test_logs::captured_async(async {
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
        // old "wrote black ... to 0 devices" outcome must not be logged (asserted on the text below).
        sup.tick(Instant::now(), false, Some(Rgb8::new(9, 9, 9))).await;
        assert_eq!(fake.custom_mode_writes(0), 0);

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
        fake.colors(0)
    })
    .await;
    assert!(!text.contains("0 devices"), "{text}");
    assert!(text.contains("black after the daemon restart limit written to"), "{text}");
    // The boot color (9,9,9) never went out: black replaced it.
    assert_eq!(colors, vec![Rgb8::BLACK; 4]);
}

// ---- #18 review: the end of the 15 s watch window is logged ----

/// Drives a `PaintJob` on synthetic clocks against a live fake: settles, writes, announces
/// `late` (when given), runs `between`, then jumps `WATCH_WINDOW` past the first write.
/// Returns the log text.
async fn run_job_to_window_end(fake: &FakeOpenRgbServer, late: Option<DeviceInfo>, between: Duration) -> String {
    use crate::openrgb::{PaintJob, WATCH_WINDOW};
    use canvas_proto::traits::DeviceAdapter;
    let (_, text) = crate::test_logs::captured_async(async {
        let mut session = OpenRgbAdapter::new(SERVER_HOST, fake.port()).with_reply_timeout(TEST_REPLY_TIMEOUT);
        session.connect().await.unwrap();
        let mut job = PaintJob::new("boot color", Rgb8::new(1, 2, 3), None);
        let t0 = Instant::now();
        assert!(!job.step(t0, &mut session).await);
        let t1 = t0 + Duration::from_secs(1);
        assert!(!job.step(t1, &mut session).await);
        assert!(!job.painted.is_empty(), "the first write was made");
        if let Some(info) = late {
            fake.announce(info);
        }
        tokio::time::sleep(between).await;
        assert!(job.step(t1 + WATCH_WINDOW, &mut session).await, "the window has ended");
    })
    .await;
    text
}

#[tokio::test]
async fn the_watch_window_ending_with_all_devices_painted_says_so() {
    let fake = FakeOpenRgbServer::start(vec![device(0, 4), device(1, 2)]).await.unwrap();
    let text = run_job_to_window_end(&fake, None, Duration::ZERO).await;
    let line = text.lines().find(|l| l.contains("watch ended")).unwrap_or_else(|| panic!("no window-end line:
{text}"));
    assert!(line.contains("INFO") && line.contains("15 s") && line.contains("all 2 devices were painted"), "{line}");
}

#[tokio::test]
async fn the_watch_window_ending_with_an_unpainted_device_warns_with_the_count() {
    // Device 0 is listed at once; device 1 is announced after the first write (event, no
    // clock) and before the window's last pass, which only counts and does not paint.
    // A time-based announce raced the first write under load (`cause:timing-test`).
    let fake = FakeOpenRgbServer::start(vec![device(0, 4)]).await.unwrap();
    let text = run_job_to_window_end(&fake, Some(device(1, 2)), Duration::from_millis(500)).await;
    let line = text.lines().find(|l| l.contains("watch ended")).unwrap_or_else(|| panic!("no window-end line:
{text}"));
    assert!(line.contains("WARN") && line.contains("1 devices are still unpainted") && line.contains("15 s"), "{line}");
}

// ---- #81: every stop of the OpenRGB server leaves a line, so the next dark stick is diagnosable ----

/// A daemon started first (pid 4000), then an OpenRGB server (pid 4001), both on one set of fakes.
async fn running_pair(root: &TempDir, exe: &Path) -> (crate::daemon::DaemonSupervisor, OpenRgbSupervisor, FakeProcesses) {
    let processes = FakeProcesses::new();
    let mut daemon = crate::daemon::DaemonSupervisor::new(
        PathBuf::from(r"C:\svc\canvas-rgbd.exe"),
        Box::new(crate::fakes::FakeLauncher(processes.clone())),
    );
    daemon.start(Instant::now());
    // Port 1 has no server: only the process lifecycle matters here.
    let mut sup = supervisor(&processes, root, exe, 6742, Some(1));
    sup.tick(Instant::now(), true, None).await;
    assert_eq!(sup.starts(), 1);
    (daemon, sup, processes)
}

/// The line that announces the end of OpenRGB `pid`, if the text has one.
fn openrgb_stop_line(text: &str, pid: u32) -> Option<&str> {
    let announcement = format!("stopping the OpenRGB server (pid {pid})");
    text.lines().find(|line| line.contains(&announcement))
}

/// A service stop ends the daemon, then OpenRGB: the OpenRGB line names its pid, the reason and
/// how, says the daemon went first and how long before, and comes after the daemon's own line.
#[tokio::test]
async fn a_service_stop_logs_the_openrgb_kill_with_its_pid_reason_and_the_gap_after_the_daemon() {
    let (root, exe) = install();
    let (daemon, sup, _processes) = running_pair(&root, &exe).await;
    let home = tempfile::tempdir().unwrap();
    let mut core = crate::pipe_server::tests::test_core(home.path().to_path_buf());
    core.daemon = daemon;
    core.openrgb = sup;

    let ((), text) = crate::test_logs::captured(|| core.shutdown("service stop"));

    let line = openrgb_stop_line(&text, 4001).unwrap_or_else(|| panic!("no OpenRGB stop line:\n{text}"));
    assert!(line.contains("service stop"), "the reason: {line}");
    assert!(line.contains("how: kill"), "the how: {line}");
    assert!(line.contains("the daemon (pid 4000) was told to stop") && line.contains(" ms earlier"), "the gap: {line}");
    assert!(line.contains("its exit was not waited on"), "the kill is a request, not the exit: {line}");
    let daemon_at = text.find("terminating the daemon (pid 4000)").expect("the daemon's own line");
    let openrgb_at = text.find("stopping the OpenRGB server (pid 4001)").expect("the OpenRGB line");
    assert!(daemon_at < openrgb_at, "the daemon stop comes first: {text}");
    assert_eq!(core.openrgb.state(), OpenRgbServerState::NotStarted);
}

/// A restart on request kills OpenRGB under a daemon that keeps streaming: the line says so.
#[tokio::test]
async fn a_restart_logs_the_openrgb_kill_with_its_pid_and_that_the_daemon_still_runs() {
    let (root, exe) = install();
    let (mut daemon, mut sup, _processes) = running_pair(&root, &exe).await;

    let ((), text) = crate::test_logs::captured(|| sup.restart("requested over the pipe", daemon.before_openrgb_stop()));

    let line = openrgb_stop_line(&text, 4001).unwrap_or_else(|| panic!("no OpenRGB stop line:\n{text}"));
    assert!(line.contains("restart: requested over the pipe"), "the reason: {line}");
    assert!(line.contains("how: kill"), "the how: {line}");
    assert!(line.contains("the daemon (pid 4000) is still running and was not told"), "the daemon: {line}");
}

/// A stop with nothing running says that, instead of leaving the log silent.
#[tokio::test]
async fn a_stop_with_no_server_process_says_so() {
    let processes = FakeProcesses::new();
    let (root, exe) = install();
    let mut sup = supervisor(&processes, &root, &exe, 6742, Some(1));
    let ((), text) = crate::test_logs::captured(|| sup.stop("service stop", crate::daemon::DaemonBeforeStop::NotRunning));
    assert!(text.contains("no OpenRGB server process to stop") && text.contains("service stop"), "{text}");
}

/// An OpenRGB that exits by itself is logged with its pid and as an exit, not as a service stop.
#[tokio::test]
async fn an_openrgb_that_exits_on_its_own_is_logged_with_its_pid_as_an_exit() {
    let (root, exe) = install();
    let (_daemon, mut sup, processes) = running_pair(&root, &exe).await;
    let (_, text) = crate::test_logs::captured_async(async {
        processes.exit_last();
        sup.tick(Instant::now(), true, None).await;
    })
    .await;
    let line = text
        .lines()
        .find(|line| line.contains("OpenRGB server (pid 4001) exited on its own"))
        .unwrap_or_else(|| panic!("no exit line:\n{text}"));
    assert!(line.contains("not stopped by the service"), "{line}");
    assert!(openrgb_stop_line(&text, 4001).is_none(), "an exit is not a kill: {text}");
}

/// A daemon the service could not end was not terminated, so the OpenRGB line must not say it
/// was and must not report a gap (#81).
#[tokio::test]
async fn a_daemon_the_service_could_not_end_is_not_logged_as_terminated() {
    let (root, exe) = install();
    let (daemon, sup, processes) = running_pair(&root, &exe).await;
    let home = tempfile::tempdir().unwrap();
    let mut core = crate::pipe_server::tests::test_core(home.path().to_path_buf());
    core.daemon = daemon;
    core.openrgb = sup;

    // The next kill fails: the daemon's, not OpenRGB's.
    processes.fail_kills(1);
    let ((), text) = crate::test_logs::captured(|| core.shutdown("service stop"));

    let line = openrgb_stop_line(&text, 4001).unwrap_or_else(|| panic!("no OpenRGB stop line:\n{text}"));
    assert!(!line.contains("was told to stop"), "the daemon was not told to stop: {line}");
    assert!(!line.contains("ms earlier"), "there is no gap to report: {line}");
    assert!(line.contains("the daemon (pid 4000) could not be ended"), "the line says what happened: {line}");
    assert!(text.contains("could not end the daemon"), "the failed kill is logged: {text}");
}

/// A stop that worked is routine: the daemon's line and the OpenRGB line are INFO ones, so a
/// warning in the log means something is wrong (#81).
#[tokio::test]
async fn a_routine_stop_is_an_info_line() {
    let (root, exe) = install();
    let (mut daemon, mut sup, _processes) = running_pair(&root, &exe).await;

    let ((), text) = crate::test_logs::captured(|| {
        daemon.stop();
        sup.stop("service stop", daemon.before_openrgb_stop());
    });

    for what in ["terminating the daemon (pid 4000)", "stopping the OpenRGB server (pid 4001)"] {
        let line = text
            .lines()
            .find(|line| line.contains(what))
            .unwrap_or_else(|| panic!("no line for {what}:\n{text}"));
        assert!(line.contains("INFO"), "{what} is routine, not a warning: {line}");
        assert!(!line.contains("WARN"), "{what} is routine, not a warning: {line}");
    }
    assert!(!text.contains("could not end the daemon"), "nothing failed: {text}");
}

/// The warning is the failure: a kill that did not work is the only WARN in a stop (#81).
#[tokio::test]
async fn a_failed_kill_is_the_warning() {
    let (root, exe) = install();
    let (mut daemon, mut sup, processes) = running_pair(&root, &exe).await;
    // The next kill fails: the daemon's, not OpenRGB's.
    processes.fail_kills(1);

    let ((), text) = crate::test_logs::captured(|| {
        daemon.stop();
        sup.stop("service stop", daemon.before_openrgb_stop());
    });

    let line = text
        .lines()
        .find(|line| line.contains("could not end the daemon"))
        .unwrap_or_else(|| panic!("no failure line:\n{text}"));
    assert!(line.contains("WARN"), "{line}");
}
