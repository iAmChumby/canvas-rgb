//! Service-level tests: the core's tick, what "connected" means for the OpenRGB supervisor, and
//! the boot color while the daemon starts (#163).
//!
//! Everything runs on the fakes: a fake daemon launcher, a fake OpenRGB server, and a fan
//! controller with no chip behind it.

use crate::chip::{FanChip, Nct6687, MSI_ALT1_MAP};
use crate::daemon::DaemonSupervisor;
use crate::fakes::{FakeChip, FakeLauncher, FakeProcesses, FakeSpawner, FakeTemp};
use crate::fans::FanController;
use crate::openrgb::{OpenRgbSupervisor, SERVER_HOST};
use crate::paths;
use crate::service::{Core, PowerEvent};
use crate::store;
use crate::temps::TempReader;
use crate::test_logs::captured_async;
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_proto::api::error_codes;
use canvas_proto::config::FansFile;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::fans::{FanMode, TempSource};
use canvas_proto::pipe::{PipeRequest, PipeResponse};
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

// ---- Issue #146: the owner's word on whether a header is on this board ----

/// The MS-7E56 board: its per-board table says System fan 5 and 6 are not routed.
const MS7E56: &str = "PRO B850-P WIFI (MS-7E56)";

/// The MSI alt1 channel indices: cpu_fan 0, pump_fan 1, sys_fan1 2, sys_fan5 6.
const SYS_FAN1: usize = 2;
const SYS_FAN5: usize = 6;

/// The service core over the fakes, with the simulated NCT chip behind it and the
/// per-board table loaded. The chip is confirmed, so the headers really are written.
fn core_with_chip(home: &TempDir, chip: &FakeChip, board: Option<&str>, now: Instant) -> Core {
    let chip_box: Box<dyn FanChip> = Box::new(Nct6687::detect(chip.clone(), MSI_ALT1_MAP).unwrap());
    let temps: BTreeMap<TempSource, Box<dyn TempReader>> =
        BTreeMap::from([(TempSource::Cpu, Box::new(FakeTemp::new(40.0)) as Box<dyn TempReader>)]);
    let mut fans = FanController::new(
        home.path().to_path_buf(),
        Some(chip_box),
        None,
        temps,
        FansFile::default(),
        Some(MSI_ALT1_MAP.name.to_string()),
    );
    fans.set_board_product(board.map(str::to_string));
    fans.startup(now);
    core_with_fans(fans, home)
}

/// The service core over the fakes, with a fan controller the caller built.
fn core_with_fans(fans: FanController, home: &TempDir) -> Core {
    let processes = FakeProcesses::new();
    let daemon = DaemonSupervisor::new(
        PathBuf::from(r"C:\svc\canvas-rgbd.exe"),
        Box::new(FakeLauncher(processes.clone())),
    );
    let openrgb = OpenRgbSupervisor::new(
        home.path().join("openrgb").join("OpenRGB.exe"),
        1,
        home.path().join("openrgb-config"),
        home.path().to_path_buf(),
        Box::new(FakeSpawner(processes.clone())),
        Box::new(move || Box::new(OpenRgbAdapter::new(SERVER_HOST, 1))),
    );
    Core::new(fans, openrgb, daemon)
}

/// The header's row in the last status the core reported.
fn header_of(core: &mut Core, id: &str) -> canvas_proto::fans::FanHeaderStatus {
    core.fans
        .status()
        .headers
        .into_iter()
        .find(|h| h.id == id)
        .expect("the status reports the header")
}

/// The overrides saved in `chip.toml`.
fn saved_overrides(home: &TempDir) -> BTreeMap<String, bool> {
    store::load_chip_state(&paths::chip_path(home.path())).unwrap().header_overrides
}

/// Marking a header "not on my board" releases it to BIOS and saves the word in `chip.toml`.
#[test]
fn issue146_marking_a_header_not_on_my_board_releases_it_and_saves_chip_toml() {
    let t0 = Instant::now();
    let home = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let mut core = core_with_chip(&home, &chip, Some(MS7E56), t0);
    // The header is under canvas-rgb's hand first, so the release has something to release.
    core.handle(
        PipeRequest::SetHeaderMode { header: "sys_fan1".into(), mode: FanMode::Fixed { percent: 50 } },
        t0,
    );
    assert!(chip.manual(SYS_FAN1), "the header is under canvas-rgb's hand");

    let answer = core.handle(
        PipeRequest::SetHeaderAvailability { header: "sys_fan1".into(), available: Some(false) },
        t0,
    );
    assert!(matches!(answer, PipeResponse::Ok), "the service took the word: {answer:?}");
    assert!(!chip.manual(SYS_FAN1), "the header was released to BIOS");
    assert!(!header_of(&mut core, "sys_fan1").available, "the header reports not available");
    assert_eq!(
        header_of(&mut core, "sys_fan1").override_available,
        Some(false),
        "the status carries the owner's word"
    );
    assert_eq!(
        saved_overrides(&home).get("sys_fan1"),
        Some(&false),
        "the word is saved in chip.toml: {:?}",
        saved_overrides(&home)
    );
}

/// "Show anyway" says the header is on this board; it does not write the header.
#[test]
fn issue146_show_anyway_makes_a_hidden_header_available_and_writes_nothing() {
    let t0 = Instant::now();
    let home = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let mut core = core_with_chip(&home, &chip, Some(MS7E56), t0);
    // The board table says System fan 5 is not routed on this board.
    assert!(!header_of(&mut core, "sys_fan5").available, "the table hides System fan 5");

    let answer = core.handle(
        PipeRequest::SetHeaderAvailability { header: "sys_fan5".into(), available: Some(true) },
        t0,
    );
    assert!(matches!(answer, PipeResponse::Ok), "the service took the word: {answer:?}");
    assert!(header_of(&mut core, "sys_fan5").available, "show anyway makes it available");
    assert_eq!(
        header_of(&mut core, "sys_fan5").override_available,
        Some(true),
        "the status carries the owner's word"
    );
    assert_eq!(saved_overrides(&home).get("sys_fan5"), Some(&true), "the word is saved in chip.toml");
    // The header is not written until the owner gives it a mode.
    assert!(!chip.manual(SYS_FAN5), "show anyway never takes the header manual");
}

/// Clearing the word takes the override off, so the board table decides again.
#[test]
fn issue146_clearing_the_word_gives_the_board_table_the_decision_back() {
    let t0 = Instant::now();
    let home = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let mut core = core_with_chip(&home, &chip, Some(MS7E56), t0);
    core.handle(
        PipeRequest::SetHeaderAvailability { header: "sys_fan5".into(), available: Some(false) },
        t0,
    );
    assert!(!header_of(&mut core, "sys_fan5").available, "the word hides the header");
    assert_eq!(saved_overrides(&home).get("sys_fan5"), Some(&false));

    let answer = core.handle(
        PipeRequest::SetHeaderAvailability { header: "sys_fan5".into(), available: None },
        t0,
    );
    assert!(matches!(answer, PipeResponse::Ok), "the service took the word off: {answer:?}");
    assert_eq!(
        saved_overrides(&home).get("sys_fan5"),
        None,
        "the key is gone from chip.toml: {:?}",
        saved_overrides(&home)
    );
    assert_eq!(
        header_of(&mut core, "sys_fan5").override_available,
        None,
        "the status carries no word"
    );
    // The board table decides again: System fan 5 is not routed on MS-7E56.
    assert!(!header_of(&mut core, "sys_fan5").available, "the table hides it again");
}

/// A header the chip does not have is refused: the word is about a header, not a free text.
#[test]
fn issue146_an_unknown_header_is_refused() {
    let t0 = Instant::now();
    let home = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let mut core = core_with_chip(&home, &chip, Some(MS7E56), t0);

    for available in [Some(false), Some(true), None] {
        let answer = core.handle(
            PipeRequest::SetHeaderAvailability { header: "not_a_header".into(), available },
            t0,
        );
        match answer {
            PipeResponse::Error { code, message } => {
                assert_eq!(code, error_codes::VALIDATION, "an unknown header is a validation error");
                assert!(message.contains("not_a_header"), "the message names the header: {message}");
            }
            other => panic!("expected a validation error, got {other:?}"),
        }
    }
    assert!(saved_overrides(&home).is_empty(), "a refused word is not saved");
}
