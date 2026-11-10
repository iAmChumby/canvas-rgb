//! Controller tests over the register simulation (`FakeChip`) and fake temperatures.

use crate::chip::{FanChip, Nct6687, COMMAND_REG, MODE_REG, MSI_ALT1_MAP, SYS_MODE_REG};
use crate::fakes::{FakeChip, FakeTemp};
use crate::fans::{curve_percent, FanController, FanError, RESUME_GRACE, SUSPEND_HOLD_CAP_TICKS};
use crate::paths;
use crate::store;
use crate::temps::TempReader;
use canvas_proto::config::FansFile;
use canvas_proto::fans::{CurvePoint, FanMode, FanProfile, TempSource};
use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime};
use tempfile::TempDir;

/// MSI alt1 channel indices: cpu_fan 0, pump_fan 1, sys_fan1 2.
const SYS_FAN1: usize = 2;
const CPU_FAN: usize = 0;

struct Rig {
    dir: TempDir,
    chip: FakeChip,
    cpu: FakeTemp,
    fans: FanController,
}

fn point(temp_c: f32, percent: u8) -> CurvePoint {
    CurvePoint { temp_c, percent }
}

/// The map name `chip.toml` must name for writes to be allowed on the simulated board.
fn confirmed() -> Option<String> {
    Some(MSI_ALT1_MAP.name.to_string())
}

fn controller_with(
    dir: &TempDir,
    chip: &FakeChip,
    cpu: &FakeTemp,
    config: FansFile,
    now: Instant,
    confirmed_map: Option<String>,
) -> FanController {
    let chip_box: Box<dyn FanChip> = Box::new(Nct6687::detect(chip.clone(), MSI_ALT1_MAP).unwrap());
    let temps: BTreeMap<TempSource, Box<dyn TempReader>> =
        BTreeMap::from([(TempSource::Cpu, Box::new(cpu.clone()) as Box<dyn TempReader>)]);
    let mut fans = FanController::new(dir.path().to_path_buf(), Some(chip_box), None, temps, config, confirmed_map);
    fans.startup(now);
    fans
}

fn controller(dir: &TempDir, chip: &FakeChip, cpu: &FakeTemp, config: FansFile, now: Instant) -> FanController {
    controller_with(dir, chip, cpu, config, now, confirmed())
}

fn rig(config: FansFile, now: Instant) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let fans = controller(&dir, &chip, &cpu, config, now);
    Rig { dir, chip, cpu, fans }
}

const UNCONFIRMED_TEXT: &str = "channel map msi_alt1 not yet confirmed against LibreHardwareMonitor; fans stay on BIOS";

/// While unconfirmed, the only register writes allowed are restores to auto: every write goes to
/// the fan-mode register, every value written is the auto value (all manual bits clear), and no
/// duty register is written. On a fresh chip (all bits clear) the auto value is 0.
fn assert_only_auto_restores(chip: &FakeChip) {
    let log = chip.writes_log();
    let duty_regs: Vec<u16> = MSI_ALT1_MAP.channels.iter().filter_map(|c| c.pwm_write_reg).collect();
    assert!(log.iter().all(|(reg, _)| is_restore_reg(*reg)), "only restore registers: {log:?}");
    assert!(log.iter().all(|(reg, _)| !duty_regs.contains(reg)), "no duty register: {log:?}");
    assert!(
        log.iter().filter(|(reg, _)| *reg != COMMAND_REG).all(|(_, value)| *value == 0),
        "every mode value is the auto value: {log:?}"
    );
}

/// The registers a restore to auto may write: the two fan-mode registers, and the command
/// register for the configuration handshake around a mode change.
fn is_restore_reg(reg: u16) -> bool {
    reg == MODE_REG || reg == SYS_MODE_REG || reg == COMMAND_REG
}

#[test]
fn unconfirmed_writes_only_restores_to_auto_and_every_header_says_why() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(55.0);
    let mut fans = controller_with(&dir, &chip, &cpu, FansFile::default(), t0, None);
    assert!(!fans.writes_allowed());
    fans.tick(t0);
    // Preparing a mode persists it but applies nothing.
    fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    fans.set_profile("Default", t0).unwrap();
    // Startup's BIOS restore on a fresh chip is the only register traffic.
    assert_only_auto_restores(&chip);
    assert!(!chip.manual(SYS_FAN1));
    assert_eq!(chip.register(MODE_REG), 0);
    assert_eq!(chip.register(SYS_MODE_REG), 0);

    let status = fans.status();
    for h in &status.headers {
        assert!(h.on_bios, "{} reports BIOS", h.id);
        assert_eq!(h.override_reason.as_deref(), Some(UNCONFIRMED_TEXT), "{} carries the reason", h.id);
    }
    // Reads still work.
    assert!(status.headers.iter().any(|h| h.rpm.is_some()));
    let saved = store::load_fans(&paths::fans_path(dir.path())).unwrap().unwrap();
    assert_eq!(saved.profiles[0].headers.get("sys_fan1"), Some(&FanMode::Fixed { percent: 50 }));
}

#[test]
fn a_confirmation_for_a_different_map_is_treated_as_unconfirmed() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = controller_with(&dir, &chip, &cpu, FansFile::default(), t0, Some("default".into()));
    assert!(!fans.writes_allowed());
    fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert_only_auto_restores(&chip);
    assert!(!chip.manual(SYS_FAN1));
}

#[test]
fn a_header_left_manual_by_an_earlier_run_is_restored_to_auto_at_startup_while_unconfirmed() {
    let t0 = Instant::now();
    // An earlier confirmed run engaged sys_fan1 and then stopped without restoring it.
    let r = rig(FansFile::default(), t0);
    let mut first = r.fans;
    first.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert!(r.chip.manual(SYS_FAN1));
    drop(first);

    // The map is now unconfirmed. Startup must hand the header back to auto anyway.
    let cpu = FakeTemp::new(40.0);
    let before = r.chip.writes_log().len();
    let mut unconfirmed = controller_with(&r.dir, &r.chip, &cpu, FansFile::default(), t0, None);
    assert!(!unconfirmed.writes_allowed());
    assert!(!r.chip.manual(SYS_FAN1), "restored to auto at startup");
    assert_eq!(r.chip.register(MODE_REG), 0, "every manual bit cleared");
    assert_eq!(r.chip.register(SYS_MODE_REG), 0, "every system-fan manual bit cleared");
    // And the profile did not re-engage it.
    unconfirmed.tick(t0 + Duration::from_secs(1));
    assert!(!r.chip.manual(SYS_FAN1));
    let since = r.chip.writes_log()[before..].to_vec();
    assert!(!since.is_empty(), "the restore wrote the mode register");
    assert!(since.iter().all(|(reg, _)| is_restore_reg(*reg)), "only restore registers: {since:?}");
    // In each mode register, every write clears bits and never sets one, and it ends at auto (0).
    for mode_reg in [MODE_REG, SYS_MODE_REG] {
        let values: Vec<u8> = since.iter().filter(|(reg, _)| *reg == mode_reg).map(|(_, v)| *v).collect();
        assert!(values.windows(2).all(|w| w[1] & !w[0] == 0), "{mode_reg:#x} only clears bits: {values:?}");
        assert!(values.last().is_none_or(|v| *v == 0), "{mode_reg:#x} ends at auto: {values:?}");
    }
}

#[test]
fn restore_bios_writes_only_the_mode_register_while_unconfirmed() {
    let t0 = Instant::now();
    let r = rig(FansFile::default(), t0);
    let mut first = r.fans;
    first.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert!(r.chip.manual(SYS_FAN1));
    drop(first);

    // `--restore-bios` builds its own chip and never consults chip.toml.
    let before = r.chip.written_regs().len();
    let mut chip = Nct6687::detect(r.chip.clone(), MSI_ALT1_MAP).unwrap();
    FanController::restore_from_file(r.dir.path(), &mut chip).unwrap();
    let after = r.chip.written_regs();
    assert!(after.len() > before, "restore wrote something");
    assert!(after[before..].iter().all(|reg| is_restore_reg(*reg)), "only restore registers: {:?}", &after[before..]);
    assert!(!r.chip.manual(SYS_FAN1));
}

#[test]
fn chip_state_defaults_to_unconfirmed_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = paths::chip_path(dir.path());
    assert_eq!(store::load_chip_state(&path).unwrap(), store::ChipState::unconfirmed());
    store::save_chip_state(&path, &store::ChipState::confirmed("msi_alt1")).unwrap();
    let back = store::load_chip_state(&path).unwrap();
    assert_eq!(back.confirmed_map.as_deref(), Some("msi_alt1"));
}

fn header<'a>(status: &'a canvas_proto::fans::FansStatus, id: &str) -> &'a canvas_proto::fans::FanHeaderStatus {
    status.headers.iter().find(|h| h.id == id).unwrap()
}

#[test]
fn curve_interpolates_linearly_and_is_flat_outside() {
    let points = [point(30.0, 20), point(70.0, 100)];
    assert_eq!(curve_percent(&points, 10.0), 20);
    assert_eq!(curve_percent(&points, 50.0), 60);
    assert_eq!(curve_percent(&points, 90.0), 100);
    assert_eq!(curve_percent(&[], 50.0), 100, "an empty curve fails to full speed");
}

#[test]
fn a_curve_drives_the_header_and_reads_back() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.tick(t0);
    r.cpu.set(Some(50.0));
    r.fans.tick(t0 + Duration::from_secs(1));
    let curve = FanMode::Curve { source: TempSource::Cpu, points: vec![point(30.0, 20), point(70.0, 100)] };
    r.fans.set_header_mode("sys_fan1", curve, t0 + Duration::from_secs(1)).unwrap();
    let status = r.fans.status();
    let h = header(&status, "sys_fan1");
    assert!(!h.on_bios);
    assert_eq!(h.pwm_percent, Some(60), "50 C on a 20..100 curve over 30..70 C is 60 %");
    assert!(r.chip.manual(SYS_FAN1));
    assert_eq!(h.override_reason, None);
}

#[test]
fn the_hot_floor_overrides_a_lower_fixed_value_and_says_why() {
    let t0 = Instant::now();
    // Issue #130: the latch mechanics run here at a user-set 50 °C CPU line; the
    // automatic line is covered by the issue130 tests below.
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(55.0));
    r.fans.tick(t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 20 }, t0).unwrap();
    let status = r.fans.status();
    let h = header(&status, "sys_fan1");
    assert_eq!(h.pwm_percent, Some(40), "hot floor 35 % plus the first 5-point boost at 55 C");
    assert_eq!(h.mode, FanMode::Fixed { percent: 20 }, "the request is kept");
    assert_eq!(h.override_reason.as_deref(), Some("holding CPU under 50 °C"));

    r.fans.set_header_mode("cpu_fan", FanMode::Fixed { percent: 30 }, t0).unwrap();
    let status = r.fans.status();
    assert_eq!(header(&status, "cpu_fan").pwm_percent, Some(55), "hot floor 50 % plus the 5-point boost at 55 C");
}

#[test]
fn a_lost_curve_sensor_hands_the_header_back_within_three_seconds() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.tick(t0);
    let curve = FanMode::Curve { source: TempSource::Cpu, points: vec![point(30.0, 20), point(70.0, 100)] };
    r.fans.set_header_mode("sys_fan1", curve, t0).unwrap();
    assert!(r.chip.manual(SYS_FAN1));

    r.cpu.set(None);
    r.fans.tick(t0 + Duration::from_secs(1));
    r.fans.tick(t0 + Duration::from_secs(2));
    assert!(r.chip.manual(SYS_FAN1), "still inside the 3 s window");
    r.fans.tick(t0 + Duration::from_secs(3));

    assert!(!r.chip.manual(SYS_FAN1), "chip readback shows BIOS mode for the header");
    let status = r.fans.status();
    let h = header(&status, "sys_fan1");
    assert!(h.on_bios);
    assert!(h.override_reason.as_deref().unwrap_or_default().contains("unavailable"));
}

#[test]
fn a_failed_sensor_diagnostic_hands_back_the_same_way() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.tick(t0);
    let curve = FanMode::Curve { source: TempSource::Cpu, points: vec![point(30.0, 20), point(70.0, 100)] };
    r.fans.set_header_mode("sys_fan1", curve, t0).unwrap();
    r.fans.set_sensor_failed(TempSource::Cpu, true);
    for s in 1..=3 {
        r.fans.tick(t0 + Duration::from_secs(s));
    }
    assert!(!r.chip.manual(SYS_FAN1));
    r.fans.set_sensor_failed(TempSource::Cpu, false);
}

#[test]
fn a_readback_mismatch_hands_the_header_back_and_stays_back() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.tick(t0);
    r.chip.set_drop_duty_writes(true);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert!(!r.chip.manual(SYS_FAN1), "the mismatch hands the header back");
    let status = r.fans.status();
    // The chip reads the duty back inside its configuration window, so the mismatch is
    // reported by the write itself.
    let reason = header(&status, "sys_fan1").override_reason.clone().unwrap_or_default();
    assert!(reason.starts_with("write failed:") && reason.contains("read back"), "{reason}");
    // Sticky: later ticks do not drive it again.
    r.chip.set_drop_duty_writes(false);
    r.fans.tick(t0 + Duration::from_secs(1));
    assert!(!r.chip.manual(SYS_FAN1));
}

#[test]
fn bios_state_is_saved_before_the_first_manual_write() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    let saved = store::load_bios_state(&paths::bios_state_path(r.dir.path())).unwrap();
    let entry = saved.headers.get("sys_fan1").expect("sys_fan1 saved");
    assert!(!entry.manual, "the chip was on BIOS before the first write");
    assert!(entry.duty_percent.is_some());

    r.fans.set_header_mode("sys_fan1", FanMode::Bios, t0).unwrap();
    let saved = store::load_bios_state(&paths::bios_state_path(r.dir.path())).unwrap();
    assert!(!saved.headers.contains_key("sys_fan1"), "released headers leave the record");
}

#[test]
fn a_killed_service_restores_bios_before_the_profile_applies() {
    let t0 = Instant::now();
    let r = rig(FansFile::default(), t0);
    let mut first = r.fans;
    first.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert!(r.chip.manual(SYS_FAN1));
    // "Process killed": the controller is dropped without restore_all.
    drop(first);

    // The saved profile says BIOS for every header, so the restart must hand sys_fan1 back.
    store::save_fans(&paths::fans_path(r.dir.path()), &FansFile::default()).unwrap();
    let cpu = FakeTemp::new(40.0);
    let _second = controller(&r.dir, &r.chip, &cpu, store::load_fans(&paths::fans_path(r.dir.path())).unwrap().unwrap(), t0);
    assert!(!r.chip.manual(SYS_FAN1), "startup restored BIOS on the header");
    assert!(!paths::bios_state_path(r.dir.path()).exists(), "the crash record was consumed");
}

#[test]
fn a_restart_reapplies_the_saved_profile_after_restoring_bios() {
    let t0 = Instant::now();
    let r = rig(FansFile::default(), t0);
    let mut first = r.fans;
    first.set_header_mode("sys_fan1", FanMode::Fixed { percent: 40 }, t0).unwrap();
    drop(first);
    let cpu = FakeTemp::new(40.0);
    let mut second = controller(&r.dir, &r.chip, &cpu, store::load_fans(&paths::fans_path(r.dir.path())).unwrap().unwrap(), t0);
    assert!(r.chip.manual(SYS_FAN1), "the saved Default profile asks for 40 % again");
    let status = second.status();
    assert_eq!(header(&status, "sys_fan1").pwm_percent, Some(40));
}

#[test]
fn a_profile_below_the_floor_is_refused_and_nothing_is_saved() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let bad = FanProfile {
        name: "Loud".into(),
        headers: BTreeMap::from([("sys_fan1".into(), FanMode::Fixed { percent: 10 })]),
    };
    let err = r.fans.put_profile(bad, t0).err().unwrap();
    assert!(matches!(err, FanError::Validation(_)), "{err}");
    assert!(!paths::fans_path(r.dir.path()).exists(), "a refused profile writes nothing");
}

#[test]
fn the_tach_only_header_cannot_take_a_mode() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let err = r.fans.set_header_mode("pump_fan2", FanMode::Fixed { percent: 50 }, t0).err().unwrap();
    assert!(matches!(err, FanError::Validation(_)), "{err}");
    let status = r.fans.status();
    assert!(status.headers.iter().any(|h| h.id == "pump_fan2" && h.rpm.is_some()), "tach-only still reports RPM");
}

#[test]
fn an_unknown_profile_is_not_found() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let err = r.fans.set_profile("Nope", t0).err().unwrap();
    assert!(matches!(err, FanError::NotFound(_)), "{err}");
}

#[test]
fn switching_profiles_changes_the_modes_and_the_status_shows_it() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let quiet = FanProfile {
        name: "Quiet".into(),
        headers: BTreeMap::from([("sys_fan1".into(), FanMode::Fixed { percent: 30 })]),
    };
    r.fans.put_profile(quiet, t0).unwrap();
    r.fans.set_profile("Quiet", t0).unwrap();
    let status = r.fans.status();
    assert_eq!(status.active_profile, "Quiet");
    assert_eq!(status.profiles, vec!["Default".to_string(), "Quiet".to_string()]);
    assert_eq!(header(&status, "sys_fan1").mode, FanMode::Fixed { percent: 30 });
    assert_eq!(header(&status, "sys_fan1").pwm_percent, Some(30));
    r.fans.set_profile("Default", t0).unwrap();
    assert!(!r.chip.manual(SYS_FAN1), "Default has no header modes, so sys_fan1 goes to BIOS");
}

#[test]
fn the_status_reports_the_chip_temps_and_watchdog() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.tick(t0);
    let status = r.fans.status();
    assert_eq!(status.chip.as_deref(), Some("NCT6687D-R @ 0xA20"));
    assert_eq!(status.hardware_watchdog, Some(false));
    assert_eq!(status.headers.len(), MSI_ALT1_MAP.channels.len());
    assert_eq!(status.floor_percent, 20);
    assert!(status.temps.iter().any(|t| t.source == TempSource::Cpu && t.celsius == Some(40.0)));
}

#[test]
fn restore_all_leaves_every_header_on_bios_with_no_record() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    r.fans.set_header_mode("cpu_fan", FanMode::Fixed { percent: 40 }, t0).unwrap();
    r.fans.restore_all("test");
    assert!(!r.chip.manual(SYS_FAN1));
    assert!(!r.chip.manual(CPU_FAN));
    assert!(!paths::bios_state_path(r.dir.path()).exists());
}

#[test]
fn a_chip_that_is_absent_reports_no_headers_and_refuses_modes() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let mut fans = FanController::new(dir.path().to_path_buf(), None, Some("no chip".into()), BTreeMap::new(), FansFile::default(), None);
    fans.startup(t0);
    let status = fans.status();
    assert!(status.chip.is_none());
    assert!(status.headers.is_empty());
    assert_eq!(fans.chip_error(), Some("no chip"));
    let err = fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).err().unwrap();
    assert!(matches!(err, FanError::Unavailable(_)), "{err}");
}

#[test]
fn fans_toml_round_trips_with_curves_and_boot_color() {
    let dir = tempfile::tempdir().unwrap();
    let path = paths::fans_path(dir.path());
    let mut file = FansFile { boot_color: Some(canvas_proto::Rgb8::new(1, 2, 3)), ..FansFile::default() };
    file.profiles[0].headers.insert(
        "sys_fan1".into(),
        FanMode::Curve { source: TempSource::Board("board_temp0".into()), points: vec![point(30.0, 20), point(70.0, 100)] },
    );
    file.profiles[0].headers.insert("cpu_fan".into(), FanMode::Fixed { percent: 45 });
    file.profiles[0].headers.insert("pump_fan".into(), FanMode::Bios);
    store::save_fans(&path, &file).unwrap();
    let back = store::load_fans(&path).unwrap().unwrap();
    assert_eq!(back, file);
    // Atomic write leaves no temp file behind.
    assert!(!dir.path().join("fans.toml.tmp").exists());
}

#[test]
fn board_sensors_read_back_with_the_half_degree_bit() {
    let t0 = Instant::now();
    let r = rig(FansFile::default(), t0);
    r.chip.set_board_temp(0, 42.5);
    r.chip.set_board_temp(3, 31.0);
    let mut chip = Nct6687::detect(r.chip.clone(), MSI_ALT1_MAP).unwrap();
    assert_eq!(chip.read_board_temp("board_temp0").unwrap(), Some(42.5));
    assert_eq!(chip.read_board_temp("board_temp3").unwrap(), Some(31.0));
    // Only four board sensors exist on this chip: the ones behind them read 0.0 °C and are
    // not reported, so the editor does not draw a temperature that is not there.
    assert_eq!(chip.read_board_temp("board_temp4").unwrap(), None);
    assert_eq!(chip.read_board_temp("no_such_sensor").unwrap(), None);
}

#[test]
fn rpm_follows_a_duty_change_with_a_lag_and_settles_within_ten_seconds() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 60 }, t0).unwrap();
    // 60 % of the simulated 2000 RPM full scale is 1200 RPM.
    assert!(r.chip.rpm(SYS_FAN1) < 1200, "RPM lags the duty change");
    r.chip.advance(Duration::from_secs(1));
    let early = r.chip.rpm(SYS_FAN1);
    assert!(early < 1200, "one second in, still climbing (got {early})");
    r.chip.advance(Duration::from_secs(9));
    let settled = r.chip.rpm(SYS_FAN1);
    assert!(settled.abs_diff(1200) <= 120, "within 10 % of target after 10 s (got {settled})");
}

#[test]
fn restore_from_file_hands_every_controllable_header_back() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    r.fans.set_header_mode("cpu_fan", FanMode::Fixed { percent: 40 }, t0).unwrap();
    assert!(r.chip.manual(SYS_FAN1) && r.chip.manual(CPU_FAN));
    // Simulates the panic hook or `--restore-bios` with no controller in memory.
    let mut chip = Nct6687::detect(r.chip.clone(), MSI_ALT1_MAP).unwrap();
    let count = FanController::restore_from_file(r.dir.path(), &mut chip).unwrap();
    assert_eq!(count, MSI_ALT1_MAP.channels.iter().filter(|c| c.controllable()).count());
    assert!(!r.chip.manual(SYS_FAN1) && !r.chip.manual(CPU_FAN));
    assert!(!paths::bios_state_path(r.dir.path()).exists());
}

#[test]
fn a_fans_file_with_another_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = paths::fans_path(dir.path());
    let file = FansFile { schema_version: 99, ..FansFile::default() };
    store::save_fans(&path, &file).unwrap();
    let err = store::load_fans(&path).err().unwrap();
    assert!(matches!(err, store::StoreError::Schema { found: 99, .. }), "{err}");
}
#[test]
fn a_profile_can_be_deleted_but_not_the_active_or_last_one() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    // Default is active; Quiet is not.
    assert!(matches!(r.fans.delete_profile("Default"), Err(FanError::Conflict(_))));
    assert!(matches!(r.fans.delete_profile("Nope"), Err(FanError::NotFound(_))));
    r.fans.delete_profile("Quiet").unwrap();
    assert_eq!(r.fans.status().profiles, vec!["Default".to_string()]);
    assert!(matches!(r.fans.delete_profile("Default"), Err(FanError::Conflict(_))), "active and last");
}

#[test]
fn a_system_fan_is_driven_through_the_msi_am5_registers() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let before = r.chip.writes_log().len();
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    let log = r.chip.writes_log()[before..].to_vec();
    // LibreHardwareMonitor's NCT6687DR mapping: SYSFAN1 duty at 0x265, manual bit 7 of 0x80F.
    assert!(log.contains(&(0x265, 128)), "duty written to 0x265: {log:?}");
    assert_eq!(r.chip.register(SYS_MODE_REG) & 0x80, 0x80, "manual bit 7 of 0x80F set");
    assert_eq!(r.chip.register(MODE_REG), 0, "nothing set in 0xA00: {log:?}");
    assert!(r.chip.manual(SYS_FAN1));
    assert_eq!(r.chip.duty_raw(SYS_FAN1), 128);
}

/// Delegates to a real driver but can be told to fail every `set_manual(.., false)`.
struct FlakyHandback {
    inner: Nct6687<FakeChip>,
    fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl FanChip for FlakyHandback {
    fn chip_name(&self) -> String {
        self.inner.chip_name()
    }
    fn map_name(&self) -> &'static str {
        self.inner.map_name()
    }
    fn channels(&self) -> &'static [crate::chip::Channel] {
        self.inner.channels()
    }
    fn board_sensor_names(&self) -> &'static [&'static str] {
        self.inner.board_sensor_names()
    }
    fn read_rpm(&mut self, channel: usize) -> std::io::Result<Option<u32>> {
        self.inner.read_rpm(channel)
    }
    fn read_duty_percent(&mut self, channel: usize) -> std::io::Result<Option<u8>> {
        self.inner.read_duty_percent(channel)
    }
    fn read_manual(&mut self, channel: usize) -> std::io::Result<bool> {
        self.inner.read_manual(channel)
    }
    fn set_manual(&mut self, channel: usize, manual: bool) -> std::io::Result<()> {
        if !manual && self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(std::io::Error::other("injected hand-back failure"));
        }
        self.inner.set_manual(channel, manual)
    }
    fn write_duty_percent(&mut self, channel: usize, percent: u8) -> std::io::Result<()> {
        self.inner.write_duty_percent(channel, percent)
    }
    fn read_board_temp(&mut self, name: &str) -> std::io::Result<Option<f32>> {
        self.inner.read_board_temp(name)
    }
}

#[test]
fn a_failed_handback_keeps_bios_state_json_for_the_next_start() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let fail = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let boxed: Box<dyn FanChip> =
        Box::new(FlakyHandback { inner: Nct6687::detect(chip.clone(), MSI_ALT1_MAP).unwrap(), fail: fail.clone() });
    let temps: BTreeMap<TempSource, Box<dyn TempReader>> =
        BTreeMap::from([(TempSource::Cpu, Box::new(FakeTemp::new(40.0)) as Box<dyn TempReader>)]);
    let mut fans = FanController::new(dir.path().to_path_buf(), Some(boxed), None, temps, FansFile::default(), confirmed());
    fans.startup(t0);
    fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    let path = paths::bios_state_path(dir.path());
    assert!(path.exists(), "the record exists while sys_fan1 is manual");

    fail.store(true, std::sync::atomic::Ordering::SeqCst);
    fans.restore_all("test stop");
    assert!(chip.manual(SYS_FAN1), "the injected failure left sys_fan1 manual");
    assert!(path.exists(), "bios-state.json must survive a failed hand-back");

    // A later restore that succeeds clears it.
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    fans.restore_all("test retry");
    assert!(!chip.manual(SYS_FAN1));
    assert!(!path.exists(), "a fully successful restore removes the record");
}

fn recorded_state_file(dir: &TempDir) -> std::path::PathBuf {
    let path = paths::bios_state_path(dir.path());
    let mut state = store::BiosState::default();
    state
        .headers
        .insert("sys_fan1".into(), store::SavedHeader { manual: false, duty_percent: Some(40) });
    store::save_bios_state(&path, &state).unwrap();
    path
}

#[test]
fn with_no_chip_the_recorded_bios_state_is_kept() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let path = recorded_state_file(&dir);
    let temps: BTreeMap<TempSource, Box<dyn TempReader>> = BTreeMap::new();
    let mut fans = FanController::new(dir.path().to_path_buf(), None, None, temps, FansFile::default(), confirmed());
    fans.startup(t0);
    fans.restore_all("test stop");
    assert!(path.exists(), "nothing was handed back, so the record must stay");
}

#[test]
fn a_failed_startup_restore_does_not_apply_the_profile() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let path = recorded_state_file(&dir);
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let fail = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let boxed: Box<dyn FanChip> =
        Box::new(FlakyHandback { inner: Nct6687::detect(chip.clone(), MSI_ALT1_MAP).unwrap(), fail });
    let mut config = FansFile::default();
    config.profiles[0].headers.insert("sys_fan1".into(), FanMode::Fixed { percent: 60 });
    let temps: BTreeMap<TempSource, Box<dyn TempReader>> =
        BTreeMap::from([(TempSource::Cpu, Box::new(FakeTemp::new(40.0)) as Box<dyn TempReader>)]);
    let mut fans = FanController::new(dir.path().to_path_buf(), Some(boxed), None, temps, config, confirmed());
    fans.startup(t0);
    assert!(path.exists(), "record kept");
    assert!(!chip.manual(SYS_FAN1), "no profile may take a header while the hand-back is failing");
}

// ---- Issue #17: a hovering temperature must not make the fan hunt. ----

fn quiet_curve() -> FanMode {
    FanMode::Curve { source: TempSource::Cpu, points: vec![point(30.0, 20), point(50.0, 45), point(70.0, 70)] }
}

/// Runs one tick a second at `temp`, returns the duty register after the tick.
fn tick_at_temp(r: &mut Rig, t0: Instant, second: u64, temp: f32) -> u8 {
    r.cpu.set(Some(temp));
    r.fans.tick(t0 + Duration::from_secs(second));
    r.chip.duty_raw(SYS_FAN1)
}

#[test]
fn a_temperature_alternating_50_and_54_gives_a_steady_duty() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(50.0));
    r.fans.set_header_mode("sys_fan1", quiet_curve(), t0).unwrap();
    let mut last = r.chip.duty_raw(SYS_FAN1);
    let mut changes = 0;
    for s in 1..=300 {
        let duty = tick_at_temp(&mut r, t0, s, if s % 2 == 0 { 50.0 } else { 54.0 });
        if duty != last {
            changes += 1;
            last = duty;
        }
    }
    assert!(changes <= 2, "the duty changed {changes} times in 300 s");
}

#[test]
fn a_real_rise_raises_the_duty_within_two_seconds() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(50.0));
    r.fans.set_header_mode("sys_fan1", quiet_curve(), t0).unwrap();
    for s in 1..=5 {
        tick_at_temp(&mut r, t0, s, 50.0);
    }
    let before = r.chip.duty_raw(SYS_FAN1);
    tick_at_temp(&mut r, t0, 6, 70.0);
    let after = tick_at_temp(&mut r, t0, 7, 70.0);
    assert!(after >= 175 && after > before, "70 C asks for 70 %: {before} -> {after}");
}

#[test]
fn a_real_fall_lowers_the_duty_after_the_drop_and_then_after_the_hold() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(70.0));
    r.fans.set_header_mode("sys_fan1", quiet_curve(), t0).unwrap();
    let peak = tick_at_temp(&mut r, t0, 1, 70.0);
    // One cool reading changes nothing.
    assert_eq!(tick_at_temp(&mut r, t0, 2, 50.0), peak);
    let mut after_drop = peak;
    for s in 3..=20 {
        after_drop = tick_at_temp(&mut r, t0, s, 50.0);
    }
    assert!(after_drop < peak, "20 C lower for 18 s lowers the duty: {peak} -> {after_drop}");
    let mut settled = after_drop;
    for s in 21..=100 {
        settled = tick_at_temp(&mut r, t0, s, 50.0);
    }
    // 45 % is the curve's answer at 50 C; the hold only steps down for 3 % or more (CURVE_STEP_DOWN_MIN_PERCENT), so up to 8 raw units remain.
    assert!(settled.abs_diff(115) <= 8, "after the hold the duty follows the curve: {settled}");
}

// ---- Issue #29: a sleep must not cost the user their fan profile. ----

/// A wall-clock time `secs` after an arbitrary fixed origin.
fn wall(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + secs)
}

/// A rig with sys_fan1 held at a fixed 60 % and one tick done, so the header is under control.
fn driven_rig(t0: Instant) -> Rig {
    let mut r = rig(FansFile::default(), t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 60 }, t0).unwrap();
    r.fans.tick_at(t0 + Duration::from_secs(1), wall(1));
    assert!(r.chip.manual(SYS_FAN1), "the header is under manual control before the sleep");
    r
}

/// The header is driven at 60 % and reports no BIOS fallback.
fn assert_under_control(r: &mut Rig) {
    assert!(r.chip.manual(SYS_FAN1), "the chip's mode bit is manual");
    let status = r.fans.status();
    let h = header(&status, "sys_fan1");
    assert!(!h.on_bios, "status says the header is on BIOS: {:?}", h.override_reason);
    assert_eq!(h.override_reason, None, "no handback reason");
    assert!(h.pwm_percent.is_some_and(|p| p.abs_diff(60) <= 2), "duty {:?}", h.pwm_percent);
}

#[test]
fn a_suspend_and_resume_ends_with_the_header_back_under_manual_control_after_the_grace() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);

    r.fans.on_suspend();
    let before = r.chip.writes();
    r.fans.tick_at(t0 + Duration::from_secs(2), wall(2));
    assert_eq!(r.chip.writes(), before, "no write while suspended");

    // The wake: firmware owns the fan controller. The old loop wrote straight away, read the BIOS
    // duty back for 3 s, and handed the header back for good.
    r.chip.simulate_resume();
    let t1 = t0 + Duration::from_secs(3600);
    r.fans.on_resume(t1, "test");
    let after_reset = r.chip.writes();
    r.fans.tick_at(t1 + Duration::from_secs(1), wall(3601));
    assert_eq!(r.chip.writes(), after_reset, "no write inside the resume grace");

    r.chip.release_to_service();
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(1), wall(3607));
    assert_under_control(&mut r);
    assert!(r.chip.writes() > after_reset, "the profile was written again");
    // And the loop keeps driving afterwards.
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(2), wall(3608));
    assert_under_control(&mut r);
}

#[test]
fn a_wall_clock_gap_between_ticks_is_a_resume_with_no_power_event() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);

    // No Suspend or Resume call: only the ticks show it. `Instant` advanced 2 s (it may not count
    // the sleep), the wall clock advanced 8 h.
    r.chip.simulate_resume();
    let after_reset = r.chip.writes();
    let t1 = t0 + Duration::from_secs(2);
    r.fans.tick_at(t1, wall(8 * 3600));
    assert_eq!(r.chip.writes(), after_reset, "the gap tick holds writes for the grace");
    assert!(r.fans.take_resumed(), "the core is told a resume happened");

    r.chip.release_to_service();
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(1), wall(8 * 3600 + 6));
    assert_under_control(&mut r);
}

#[test]
fn an_ordinary_tick_gap_is_not_a_resume() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);
    r.fans.tick_at(t0 + Duration::from_secs(2), wall(2 + 20));
    assert!(!r.fans.take_resumed(), "a 20 s stall is under the gap threshold");
}

#[test]
fn an_apply_inside_the_resume_window_is_held_and_applied_after_the_grace() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);
    r.fans.on_suspend();
    r.chip.simulate_resume();
    let t1 = t0 + Duration::from_secs(60);
    r.fans.on_resume(t1, "test");

    // A user applies a second header inside the window. The write that failed in #29 was exactly
    // this kind, so nothing reaches the chip until the grace ends.
    let before = r.chip.writes();
    r.fans.set_header_mode("pump_fan", FanMode::Fixed { percent: 60 }, t1 + Duration::from_secs(1)).unwrap();
    r.fans.set_profile("Default", t1 + Duration::from_secs(2)).unwrap();
    assert_eq!(r.chip.writes(), before, "no chip write for an apply inside the window");
    assert!(!r.chip.manual(1));

    r.chip.release_to_service();
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(1), wall(70));
    assert!(r.chip.manual(1), "the held apply landed after the grace");
    assert!(header(&r.fans.status(), "pump_fan").override_reason.is_none());
    assert_under_control(&mut r);
}

#[test]
fn an_apply_while_suspended_is_held() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);
    r.fans.on_suspend();
    let before = r.chip.writes();
    r.fans.set_header_mode("pump_fan", FanMode::Fixed { percent: 60 }, t0 + Duration::from_secs(2)).unwrap();
    assert_eq!(r.chip.writes(), before, "no chip write while suspended");
    r.fans.on_resume(t0 + Duration::from_secs(3), "test");
    r.fans.tick_at(t0 + Duration::from_secs(3) + RESUME_GRACE, wall(10));
    assert!(r.chip.manual(1), "applied after the resume grace");
}

#[test]
fn a_real_persistent_failure_still_hands_back_and_stays_back_after_the_grace() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);
    r.fans.on_suspend();
    // The chip reset in the sleep (duty back at the BIOS value) and then ignores duty writes for good.
    r.chip.simulate_resume();
    r.chip.release_to_service();
    r.chip.set_drop_duty_writes(true);
    let t1 = t0 + Duration::from_secs(60);
    r.fans.on_resume(t1, "test");
    r.fans.set_header_mode("pump_fan", FanMode::Fixed { percent: 60 }, t1 + Duration::from_secs(1)).unwrap();

    // Inside the grace the loop leaves the chip alone.
    let in_grace = r.chip.writes();
    r.fans.tick_at(t1 + Duration::from_secs(2), wall(62));
    assert_eq!(r.chip.writes(), in_grace, "no write inside the grace");

    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(1), wall(70));
    assert!(r.chip.writes() > in_grace, "the re-apply after the grace wrote to the chip");
    assert!(!r.chip.manual(1), "pump_fan stays on BIOS");
    assert!(!r.chip.manual(SYS_FAN1), "sys_fan1 stays on BIOS");
    let status = r.fans.status();
    assert!(header(&status, "pump_fan").override_reason.as_deref().is_some_and(|m| m.starts_with("write failed")));
    assert!(header(&status, "sys_fan1").override_reason.as_deref().is_some_and(|m| m.starts_with("write failed")));

    // Sticky: a later tick makes no further attempt, even if the chip recovers.
    r.chip.set_drop_duty_writes(false);
    let settled = r.chip.writes();
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(2), wall(71));
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(3), wall(72));
    assert_eq!(r.chip.writes(), settled, "no write after the handback");
    assert!(!r.chip.manual(1) && !r.chip.manual(SYS_FAN1));
}

#[test]
fn a_suspend_with_no_resume_event_ends_its_hold_instead_of_freezing_the_fans() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);
    r.fans.on_suspend();
    // The machine never slept, or the resume notification was lost: ticks keep coming, one a
    // second, with a steady wall clock. Firmware has the chip reset but does not hold it.
    r.chip.simulate_resume();
    r.chip.release_to_service();
    let mut held_until_cap = true;
    for s in 2..=(u64::from(SUSPEND_HOLD_CAP_TICKS) + RESUME_GRACE.as_secs() + 10) {
        r.fans.tick_at(t0 + Duration::from_secs(s), wall(s));
        if s <= u64::from(SUSPEND_HOLD_CAP_TICKS) / 2 {
            held_until_cap &= !r.chip.manual(SYS_FAN1);
        }
    }
    assert!(held_until_cap, "the hold is real before the cap");
    assert!(r.fans.take_resumed(), "the cap ended the hold through the resume path");
    assert_under_control(&mut r);
}

#[test]
fn a_wall_clock_gap_while_suspended_is_a_resume_with_no_event() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);
    r.fans.on_suspend();
    r.chip.simulate_resume();
    r.chip.release_to_service();
    // Slept, woke, and the resume event never arrived: the first tick sees an 8 h gap.
    r.fans.tick_at(t0 + Duration::from_secs(2), wall(8 * 3600));
    for s in 1..=RESUME_GRACE.as_secs() + 1 {
        r.fans.tick_at(t0 + Duration::from_secs(2 + s), wall(8 * 3600 + s));
    }
    assert_under_control(&mut r);
}

#[test]
fn the_grace_constant_is_what_decides_when_writes_resume() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);
    r.fans.on_suspend();
    // Firmware lets go of the controller 3 s after the wake, on the chip's own clock. Nothing in
    // the test releases it: only RESUME_GRACE decides whether the first write lands too early.
    let firmware_busy = Duration::from_secs(3);
    r.chip.set_clock(Duration::ZERO);
    r.chip.simulate_resume_for(firmware_busy);
    let t1 = t0 + Duration::from_secs(3600);
    r.fans.on_resume(t1, "test");
    for s in 1..=RESUME_GRACE.as_secs() + 1 {
        r.chip.set_clock(Duration::from_secs(s));
        r.fans.tick_at(t1 + Duration::from_secs(s), wall(5000 + s));
    }
    assert_under_control(&mut r);
}

#[test]
fn a_curve_sensor_that_is_slow_after_the_wake_gets_its_full_timeout() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let curve = FanMode::Curve { source: TempSource::Cpu, points: vec![point(30.0, 20), point(70.0, 100)] };
    r.fans.set_header_mode("sys_fan1", curve, t0).unwrap();
    r.fans.tick_at(t0 + Duration::from_secs(1), wall(1));
    assert!(r.chip.manual(SYS_FAN1));

    // The wake, with `Instant` counting the sleep: the last good reading looks 8 h old, and the
    // sensor is down at the first write.
    r.fans.on_suspend();
    r.chip.simulate_resume();
    r.cpu.set(None);
    let t1 = t0 + Duration::from_secs(8 * 3600);
    r.fans.on_resume(t1, "test");
    r.chip.release_to_service();
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(1), wall(8 * 3600 + 6));
    assert!(header(&r.fans.status(), "sys_fan1").override_reason.is_none(), "no sensor-loss handback yet");

    r.cpu.set(Some(60.0));
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(2), wall(8 * 3600 + 7));
    assert!(r.chip.manual(SYS_FAN1), "the curve drives the header once the sensor is back");
}

#[test]
fn applying_the_profile_again_keeps_the_held_duty_of_a_curve_that_did_not_change() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(70.0));
    r.fans.set_header_mode("sys_fan1", quiet_curve(), t0).unwrap();
    let peak = tick_at_temp(&mut r, t0, 1, 70.0);
    // 62 C is within the drop threshold of 70 C, so the held duty stays.
    assert_eq!(tick_at_temp(&mut r, t0, 2, 62.0), peak);
    // Another header changes; sys_fan1's curve is the same, so its hysteresis must survive.
    r.fans.set_header_mode("pump_fan", FanMode::Fixed { percent: 60 }, t0 + Duration::from_secs(3)).unwrap();
    r.fans.set_profile("Default", t0 + Duration::from_secs(3)).unwrap();
    assert_eq!(tick_at_temp(&mut r, t0, 4, 62.0), peak, "one apply dropped another header's held duty");
    // A changed curve does start afresh.
    let other = FanMode::Curve { source: TempSource::Cpu, points: vec![point(30.0, 20), point(50.0, 40), point(70.0, 60)] };
    r.fans.set_header_mode("sys_fan1", other, t0 + Duration::from_secs(5)).unwrap();
    // Fresh state answers the new curve at 62 C exactly: 52 %, raw 133. A kept hold would sit at 140.
    assert_eq!(tick_at_temp(&mut r, t0, 6, 62.0), 133, "a changed curve starts with no held duty");
}

#[test]
fn a_small_steady_fall_steps_down_only_after_the_hold() {
    // 66 C after a 70 C peak is 4 C lower: under the drop threshold, so only the hold can lower the
    // duty. The Quiet curve asks for 65 % at 66 C, 5 % under the held 70 %.
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(70.0));
    r.fans.set_header_mode("sys_fan1", quiet_curve(), t0).unwrap();
    let peak = tick_at_temp(&mut r, t0, 1, 70.0);
    let mut duty = peak;
    for s in 2..=30 {
        duty = tick_at_temp(&mut r, t0, s, 66.0);
    }
    assert_eq!(duty, peak, "inside the hold the duty stays");
    for s in 31..=90 {
        duty = tick_at_temp(&mut r, t0, s, 66.0);
    }
    assert!(duty < peak, "after the hold the duty steps down: {peak} -> {duty}");
}

// ---- Issue #17 follow-up: the hot floor must latch (engage instantly, release after the hold). ----

/// One tick a second at `temp`, returns the cpu_fan duty register after the tick.
fn tick_cpu_at_temp(r: &mut Rig, t0: Instant, second: u64, temp: f32) -> u8 {
    r.cpu.set(Some(temp));
    r.fans.tick(t0 + Duration::from_secs(second));
    r.chip.duty_raw(CPU_FAN)
}

/// A curve whose lowest point clears the cpu_fan floor of 30 %.
fn cpu_curve() -> FanMode {
    FanMode::Curve {
        source: TempSource::Cpu,
        points: vec![point(30.0, 30), point(50.0, 44), point(70.0, 70)],
    }
}

#[test]
fn a_cpu_temp_alternating_49_and_51_gives_a_steady_duty() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(49.0));
    r.fans.set_header_mode("cpu_fan", cpu_curve(), t0).unwrap();
    let mut last = r.chip.duty_raw(CPU_FAN);
    let mut changes = 0;
    for s in 1..=300 {
        let duty = tick_cpu_at_temp(&mut r, t0, s, if s % 2 == 0 { 49.0 } else { 51.0 });
        if duty != last {
            changes += 1;
            last = duty;
        }
    }
    assert!(changes <= 2, "the duty changed {changes} times in 300 s");
}

#[test]
fn a_spike_train_on_cpu_fan_does_not_staircase() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(39.0));
    r.fans.set_header_mode("cpu_fan", cpu_curve(), t0).unwrap();
    // The logged sequence from issue #17 (23:26:56-23:27:40). Rises follow the temperature at
    // once (that is the safe direction); a staircase is the duty stepping DOWN again and again
    // inside the hold. At most one fall over the whole train.
    let mut duties = vec![tick_cpu_at_temp(&mut r, t0, 1, 39.0)];
    for (i, &temp) in [60.0f32, 66.0, 60.0, 50.0, 42.0, 52.0, 62.0, 65.0, 57.0, 50.0, 41.0].iter().enumerate() {
        duties.push(tick_cpu_at_temp(&mut r, t0, (i + 2) as u64, temp));
    }
    let falls = duties.windows(2).filter(|w| w[1] < w[0]).count();
    assert!(falls <= 1, "the duty fell {falls} times over the spike train: {duties:?}");
}

#[test]
fn a_step_from_40_to_51_raises_cpu_fan_to_the_hot_floor_on_the_same_tick() {
    let t0 = Instant::now();
    // Issue #130: at a user-set 50 °C CPU line (see the note above).
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(40.0));
    r.fans.set_header_mode("cpu_fan", cpu_curve(), t0).unwrap();
    for s in 1..=3 {
        tick_cpu_at_temp(&mut r, t0, s, 40.0);
    }
    let before = r.chip.duty_raw(CPU_FAN);
    assert!(before < 128, "40 C is below the hot floor: {before}");
    let after = tick_cpu_at_temp(&mut r, t0, 4, 51.0);
    // Issue #255: the floor still engages on the same tick, and the closed loop's first step
    // (1 point) lands with it: 50 % + 1 = 51 %, raw 130.
    assert_eq!(after, 130, "51 C engages the 50 % hot floor and the first boost step on the same tick: {before} -> {after}");
}

#[test]
fn the_hot_floor_releases_after_a_hold_continuously_below_50() {
    let t0 = Instant::now();
    // Issue #130: at a user-set 50 °C CPU line (see the note above).
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(51.0));
    r.fans.set_header_mode("cpu_fan", cpu_curve(), t0).unwrap();
    for s in 1..=3 {
        tick_cpu_at_temp(&mut r, t0, s, 51.0);
    }
    assert_eq!(r.chip.duty_raw(CPU_FAN), 135, "the floor is engaged at 51 C, with the boost at 3 points (53 %)");
    let mut duty = 135;
    for s in 4..=20 {
        duty = tick_cpu_at_temp(&mut r, t0, s, 40.0);
    }
    assert_eq!(duty, 128, "16 s below 50 C the latched floor still holds: {duty}");
    for s in 21..=70 {
        duty = tick_cpu_at_temp(&mut r, t0, s, 40.0);
    }
    assert!(duty < 128, "after 60 s below 50 C the floor released: {duty}");
}

#[test]
fn a_latched_hot_floor_never_caps_a_rising_curve() {
    // Review of PR #142: the latch must be a floor, not a cap. Latch at 51 C, then heat to
    // 85 C: the curve's 100 % must be written on that same tick. (Issue #130: at a user-set
    // 50 °C CPU line with a 9800X3D's 95 °C rated maximum, so the 90 °C ceiling stays above
    // the 85 °C top temperature and only the curve answers.)
    let t0 = Instant::now();
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.fans.set_cpu_brand(Some("AMD Ryzen 9 9800X3D 8-Core Processor"));
    let curve = FanMode::Curve {
        source: TempSource::Cpu,
        points: vec![point(30.0, 30), point(50.0, 44), point(85.0, 100)],
    };
    r.cpu.set(Some(51.0));
    r.fans.set_header_mode("cpu_fan", curve, t0).unwrap();
    for s in 1..=3 {
        tick_cpu_at_temp(&mut r, t0, s, 51.0);
    }
    assert_eq!(r.chip.duty_raw(CPU_FAN), 135, "latched at the 50 % hot floor at 51 C, with the boost at 3 points (53 %)");
    let duty = tick_cpu_at_temp(&mut r, t0, 4, 85.0);
    assert_eq!(duty, 255, "85 C takes the curve's 100 % on the same tick: {duty}");
}

#[test]
fn an_oscillating_hot_temperature_still_raises_the_curve_at_once() {
    // Review of PR #142: a temperature alternating 90/54 C must not hold the fan at a low duty.
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let curve = FanMode::Curve {
        source: TempSource::Cpu,
        points: vec![point(30.0, 30), point(50.0, 44), point(90.0, 100)],
    };
    r.cpu.set(Some(40.0));
    r.fans.set_header_mode("cpu_fan", curve, t0).unwrap();
    tick_cpu_at_temp(&mut r, t0, 1, 40.0);
    let first_hot = tick_cpu_at_temp(&mut r, t0, 2, 90.0);
    assert_eq!(first_hot, 255, "the first 90 C tick takes 100 %: {first_hot}");
    for s in 3..=20 {
        let duty = tick_cpu_at_temp(&mut r, t0, s, if s % 2 == 0 { 90.0 } else { 54.0 });
        assert!(duty >= 128, "never below the hot floor while it oscillates hot: {duty} at {s} s");
    }
}

const MS7E56: &str = "PRO B850-P WIFI (MS-7E56)";

fn board_controller(dir: &tempfile::TempDir, chip: &FakeChip, cpu: &FakeTemp, config: FansFile, now: Instant, board: Option<&str>) -> FanController {
    let chip_box: Box<dyn FanChip> = Box::new(Nct6687::detect(chip.clone(), MSI_ALT1_MAP).unwrap());
    let temps: BTreeMap<TempSource, Box<dyn TempReader>> =
        BTreeMap::from([(TempSource::Cpu, Box::new(cpu.clone()) as Box<dyn TempReader>)]);
    let mut fans = FanController::new(dir.path().to_path_buf(), Some(chip_box), None, temps, config, confirmed());
    fans.set_board_product(board.map(str::to_string));
    fans.startup(now);
    fans
}

#[test]
fn ms7e56_sys_fan5_and_6_are_unavailable_and_refuse_writes_without_a_chip_write() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some(MS7E56));
    // The issue's real symptom: the read register of an unrouted channel stays at 0.
    chip.set_stuck_zero_duty(6, true);
    chip.set_stuck_zero_duty(7, true);
    let status = fans.status();
    for id in ["sys_fan5", "sys_fan6"] {
        let h = header(&status, id);
        assert!(!h.available, "{id} reports available");
        assert!(h.on_bios, "{id} stays on BIOS");
        assert_eq!(h.override_reason, None, "{id} carries no other reason");
    }
    for id in ["cpu_fan", "sys_fan1", "sys_fan3"] {
        assert!(header(&status, id).available, "{id} reports available");
    }
    let writes_before = chip.writes();
    let err = fans.set_header_mode("sys_fan5", FanMode::Fixed { percent: 45 }, t0).unwrap_err();
    assert!(matches!(err, FanError::Unavailable(_)), "set_header_mode on an absent header is Unavailable: {err}");
    assert_eq!(chip.writes(), writes_before, "a refused write writes nothing");
}

#[test]
fn ms7e56_profile_apply_skips_the_absent_headers_and_drives_the_rest() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut profile = FansFile::default().profiles[0].clone();
    profile.headers.insert("cpu_fan".into(), FanMode::Fixed { percent: 50 });
    profile.headers.insert("sys_fan5".into(), FanMode::Fixed { percent: 50 });
    profile.headers.insert("sys_fan6".into(), FanMode::Fixed { percent: 50 });
    let config = FansFile { profiles: vec![profile], ..Default::default() };
    let mut fans = board_controller(&dir, &chip, &cpu, config, t0, Some(MS7E56));
    fans.apply_profile(t0);
    assert!(chip.manual(CPU_FAN), "cpu_fan was driven");
    assert!(!chip.manual(6), "sys_fan5 was skipped");
    assert!(!chip.manual(7), "sys_fan6 was skipped");
    assert!(!chip.written_regs().contains(&0x261), "no write to the sys_fan5 duty register");
    assert!(!chip.written_regs().contains(&0x260), "no write to the sys_fan6 duty register");
}

#[test]
fn an_unknown_board_reports_every_header_available() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some("Some other board"));
    let status = fans.status();
    assert!(status.headers.iter().all(|h| h.available), "every header is available on an unknown board");
    let writes_before = chip.writes();
    fans.set_header_mode("sys_fan5", FanMode::Fixed { percent: 45 }, t0).unwrap();
    assert!(chip.writes() > writes_before, "a present header is written as before");
}

#[test]
fn the_header_override_shows_sys_fan5_anyway_and_hides_sys_fan3() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some(MS7E56));
    fans.set_header_override("sys_fan5", true).unwrap();
    fans.set_header_override("sys_fan3", false).unwrap();
    let status = fans.status();
    assert!(header(&status, "sys_fan5").available, "show anyway makes sys_fan5 available again");
    assert!(!header(&status, "sys_fan6").available, "sys_fan6 stays absent");
    assert!(!header(&status, "sys_fan3").available, "not on my board hides sys_fan3");
    fans.set_header_mode("sys_fan5", FanMode::Fixed { percent: 45 }, t0).unwrap();
    assert!(chip.manual(6), "an overridden-available header drives again");
    let err = fans.set_header_mode("sys_fan3", FanMode::Fixed { percent: 45 }, t0).unwrap_err();
    assert!(matches!(err, FanError::Unavailable(_)), "a hidden header refuses: {err}");
    assert!(!chip.manual(CPU_FAN + 4), "the hidden header was never taken manual");
    // Overrides persist in chip.toml.
    let saved = store::load_chip_state(&paths::chip_path(dir.path())).unwrap();
    assert_eq!(saved.header_overrides.get("sys_fan5"), Some(&true));
    assert_eq!(saved.header_overrides.get("sys_fan3"), Some(&false));
}

#[test]
fn a_stuck_zero_header_is_handed_back_on_an_unknown_board_and_left_alone_on_ms7e56() {
    // The #128 symptom on the fakes: the unrouted channel's read register stays at 0, so a
    // write to it fails readback and the header is handed back to BIOS. On MS-7E56 the table
    // marks it absent: the write is refused and nothing is handed back.
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some("Some other board"));
    chip.set_stuck_zero_duty(6, true);
    fans.set_header_mode("sys_fan5", FanMode::Fixed { percent: 39 }, t0).unwrap();
    let reason = header(&fans.status(), "sys_fan5").override_reason.clone().unwrap_or_default();
    assert!(reason.starts_with("write failed:"), "unknown board: the stuck header is handed back: {reason:?}");

    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some(MS7E56));
    chip.set_stuck_zero_duty(6, true);
    let writes_before = chip.writes();
    assert!(fans.set_header_mode("sys_fan5", FanMode::Fixed { percent: 39 }, t0).is_err());
    let h = header(&fans.status(), "sys_fan5").clone();
    assert_eq!(h.override_reason, None, "MS-7E56: no handback, no write failure");
    assert_eq!(chip.writes(), writes_before, "MS-7E56: nothing written");
}

#[test]
fn issue146_clearing_a_shown_anyway_header_on_a_board_hidden_header_releases_it_to_bios() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some(MS7E56));
    // MS-7E56 hides sys_fan5; show anyway, then give it a mode so it is manual.
    fans.set_header_override("sys_fan5", true).unwrap();
    fans.set_header_mode("sys_fan5", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert!(chip.manual(6), "sys_fan5 is manual after the mode is set");
    let saved = store::load_bios_state(&paths::bios_state_path(dir.path())).unwrap();
    assert!(saved.headers.contains_key("sys_fan5"), "bios-state.json names sys_fan5 while it is manual");
    fans.clear_header_override("sys_fan5", t0).unwrap();
    fans.tick(t0 + Duration::from_secs(1));
    assert!(!chip.manual(6), "clearing to a board-hidden header releases it to BIOS");
    let saved = store::load_bios_state(&paths::bios_state_path(dir.path())).unwrap();
    assert!(!saved.headers.contains_key("sys_fan5"), "bios-state.json no longer names sys_fan5");
}

#[test]
fn issue146_clearing_not_on_my_board_resumes_the_profile_mode() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some(MS7E56));
    fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert!(chip.manual(SYS_FAN1), "sys_fan1 is running Fixed 50%");
    fans.set_header_override("sys_fan1", false).unwrap();
    assert!(!chip.manual(SYS_FAN1), "not on my board releases sys_fan1");
    fans.clear_header_override("sys_fan1", t0).unwrap();
    assert!(chip.manual(SYS_FAN1), "clearing resumes the profile mode: sys_fan1 is manual again");
    assert_eq!(header(&fans.status(), "sys_fan1").pwm_percent, Some(50), "sys_fan1 drives Fixed 50% again");
}

#[test]
fn issue146_a_shown_anyway_header_gets_the_safety_ceiling() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some(MS7E56));
    cpu.set(Some(90.0));
    fans.tick(t0);
    fans.set_header_override("sys_fan5", true).unwrap();
    fans.set_header_mode("sys_fan5", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert_eq!(chip.duty_raw(6), 255, "90 C is above the ceiling: the shown-anyway header runs at 100%");
    assert_eq!(header(&fans.status(), "sys_fan5").pwm_percent, Some(100));
}

#[test]
fn issue146_a_failed_save_on_set_false_changes_nothing() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some(MS7E56));
    fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert!(chip.manual(SYS_FAN1));
    std::fs::create_dir(paths::chip_path(dir.path())).unwrap();
    let writes_before = chip.writes();
    let err = fans.set_header_override("sys_fan1", false).unwrap_err();
    assert!(matches!(err, FanError::Store(_)), "the save failure is the error: {err}");
    assert_eq!(chip.writes(), writes_before, "a failed save writes nothing to the chip");
    assert!(chip.manual(SYS_FAN1), "the header is still manual: the release never ran");
    assert!(fans.header_available("sys_fan1"), "the in-memory word is unchanged after a failed save");
    assert_eq!(header(&fans.status(), "sys_fan1").override_available, None, "no word is in memory");
}

#[test]
fn issue146_a_failed_save_on_clear_changes_nothing() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let cpu = FakeTemp::new(40.0);
    let mut fans = board_controller(&dir, &chip, &cpu, FansFile::default(), t0, Some(MS7E56));
    fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    fans.set_header_override("sys_fan1", false).unwrap();
    assert!(!chip.manual(SYS_FAN1));
    std::fs::remove_file(paths::chip_path(dir.path())).unwrap();
    std::fs::create_dir(paths::chip_path(dir.path())).unwrap();
    let writes_before = chip.writes();
    let err = fans.clear_header_override("sys_fan1", t0).unwrap_err();
    assert!(matches!(err, FanError::Store(_)), "the save failure is the error: {err}");
    assert_eq!(chip.writes(), writes_before, "a failed save writes nothing to the chip");
    assert!(!fans.header_available("sys_fan1"), "the in-memory word is unchanged after a failed clear");
    assert_eq!(header(&fans.status(), "sys_fan1").override_available, Some(false), "the word is still in memory");
}

#[test]
fn issue146_with_no_chip_every_header_is_refused() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let cpu = FakeTemp::new(40.0);
    let temps: BTreeMap<TempSource, Box<dyn TempReader>> =
        BTreeMap::from([(TempSource::Cpu, Box::new(cpu.clone()) as Box<dyn TempReader>)]);
    let mut fans = FanController::new(dir.path().to_path_buf(), None, None, temps, FansFile::default(), None);
    fans.startup(t0);
    for available in [Some(false), Some(true)] {
        let err = match available {
            Some(v) => fans.set_header_override("sys_fan1", v).unwrap_err(),
            None => fans.clear_header_override("sys_fan1", t0).unwrap_err(),
        };
        assert!(matches!(err, FanError::Validation(_)), "no chip: every header is refused: {err}");
        assert!(err.to_string().contains("sys_fan1"), "the error names the header: {err}");
    }
    let err = fans.clear_header_override("sys_fan1", t0).unwrap_err();
    assert!(matches!(err, FanError::Validation(_)), "clear with no chip is refused: {err}");
    assert!(err.to_string().contains("sys_fan1"), "the error names the header: {err}");
}

#[test]
fn reconfirming_the_channel_map_keeps_the_header_overrides() {
    let mut old = store::ChipState::confirmed("msi_alt1");
    old.header_overrides.insert("sys_fan5".into(), true);
    old.header_overrides.insert("sys_fan3".into(), false);
    let state = crate::app::confirmed_state(Some(old), "msi_alt1");
    assert_eq!(state.confirmed_map.as_deref(), Some("msi_alt1"));
    assert_eq!(state.header_overrides.get("sys_fan5"), Some(&true));
    assert_eq!(state.header_overrides.get("sys_fan3"), Some(&false));
    assert!(crate::app::confirmed_state(None, "msi_alt1").header_overrides.is_empty());
}

// ---- Issue #130: movable hot limits and the fixed safety ceiling ----

use canvas_proto::fans::{HotKind, HotLimits};

fn hot_of(r: &mut Rig, kind: HotKind) -> canvas_proto::fans::HotLimitStatus {
    let status = r.fans.status();
    status
        .hot
        .iter()
        .find(|s| s.kind == kind)
        .copied()
        .expect("status reports a hot line for every kind")
}

#[test]
fn issue130_cpu_at_60_with_the_automatic_74_has_no_hot_floor() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(60.0));
    r.fans.tick(t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 20 }, t0).unwrap();
    let status = r.fans.status();
    let h = header(&status, "sys_fan1");
    assert_eq!(h.pwm_percent, Some(20), "60 C is below the 74 C automatic line: nothing raises the duty");
    assert_eq!(h.override_reason, None, "no override is said: the hot latched floor (latch) must stay off");
}

#[test]
fn issue130_a_user_limit_of_55_engages_at_55_and_not_at_52() {
    let t0 = Instant::now();
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(55.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(52.0));
    r.fans.tick(t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 20 }, t0).unwrap();
    let status = r.fans.status();
    assert_eq!(header(&status, "sys_fan1").pwm_percent, Some(20), "52 C is below the user line of 55 C: no floor");
    assert_eq!(header(&status, "sys_fan1").override_reason, None);

    r.cpu.set(Some(56.0));
    r.fans.tick(t0 + Duration::from_secs(1));
    let status = r.fans.status();
    assert_eq!(header(&status, "sys_fan1").pwm_percent, Some(36), "56 C is at the user line: the hot floor applies, plus the boost's first point");
    assert_eq!(header(&status, "sys_fan1").override_reason.as_deref(), Some("holding CPU under 55 °C"));
}

#[test]
fn issue130_the_safety_ceiling_takes_100_and_releases_after_the_hold() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(84.0));
    r.fans.tick(t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 40 }, t0).unwrap();
    let status = r.fans.status();
    let h = header(&status, "sys_fan1");
    assert_eq!(h.pwm_percent, Some(100), "84 C is the 7800X3D ceiling: the safety ceiling (rule) writes 100 %");
    assert!(
        h.override_reason.as_deref().unwrap_or_default().starts_with("at the safety ceiling"),
        "the ceiling is said out loud: {:?}",
        h.override_reason
    );

    // The release carries the same hold-time rule as the hot latch (CURS hold).
    r.cpu.set(Some(60.0));
    for s in 1..=65 {
        r.fans.tick(t0 + Duration::from_secs(s));
    }
    let status = r.fans.status();
    let h = header(&status, "sys_fan1");
    assert_eq!(h.pwm_percent, Some(40), "after the hold below the ceiling the Fixed duty returns: {:?}", h.pwm_percent);
    assert!(h.override_reason.is_none(), "the ceiling reason clears with the latch: {:?}", h.override_reason);
}

#[test]
fn issue130_a_board_curve_still_reads_its_own_board_line() {
    let t0 = Instant::now();
    // Board at 55 C with a user board line of 60 C: no floor, where the old 50 C would have engaged one.
    let config = FansFile { hot_limits: HotLimits { cpu_c: None, gpu_c: None, board_c: Some(60.0) }, ..Default::default() };
    let mut r = rig(config, t0);
    r.chip.set_board_temp(0, 55.0);
    r.fans.tick(t0);
    let flat = FanMode::Curve {
        source: TempSource::Board("board_temp0".into()),
        points: vec![point(30.0, 20), point(70.0, 20)],
    };
    r.fans.set_header_mode("sys_fan1", flat, t0).unwrap();
    let status = r.fans.status();
    assert_eq!(header(&status, "sys_fan1").pwm_percent, Some(20), "55 C is below the board user line of 60 C: no floor");
    assert_eq!(header(&status, "sys_fan1").override_reason, None);
}

#[test]
fn issue130_board_sensors_keep_the_50_degree_line_by_default() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.chip.set_board_temp(0, 55.0);
    r.fans.tick(t0);
    let flat = FanMode::Curve {
        source: TempSource::Board("board_temp0".into()),
        points: vec![point(30.0, 20), point(70.0, 20)],
    };
    r.fans.set_header_mode("sys_fan1", flat, t0).unwrap();
    let status = r.fans.status();
    assert_eq!(header(&status, "sys_fan1").pwm_percent, Some(40), "the board automatic line stays 50 C: the hot floor applies at 55 C, plus the 5-point boost");
    let board = hot_of(&mut r, HotKind::Board);
    assert_eq!(board.limit_c, 50.0);
    assert_eq!(board.ceiling_c, 90.0);
}

// ---- Issue #255: "Don't exceed" is a closed loop, not a flat floor. ----

/// The duty the chip reports for `ch`, in percent (the driver's own readback conversion).
fn duty_percent(r: &Rig, ch: usize) -> u8 {
    ((u32::from(r.chip.duty_raw(ch)) * 100 + 127) / 255) as u8
}

/// A flat 50 % CPU curve, so only the hot-limit boost moves the duty.
fn flat_fifty_curve() -> FanMode {
    FanMode::Curve { source: TempSource::Cpu, points: vec![point(30.0, 50), point(70.0, 50)] }
}

#[test]
fn issue255_a_curve_above_the_hot_limit_speeds_up_to_full_speed() {
    // Issue #255: a curve sitting at 50 % with the reference held at 70 C and a 50 C line
    // climbs tick over tick and reaches 100 %.
    let t0 = Instant::now();
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(40.0));
    r.fans.set_header_mode("cpu_fan", flat_fifty_curve(), t0).unwrap();
    for s in 1..=3 {
        tick_cpu_at_temp(&mut r, t0, s, 40.0);
    }
    let mut last = duty_percent(&r, CPU_FAN);
    assert!(last < 100, "40 C is below the line: {last}%");
    let mut duties = Vec::new();
    for s in 4..=10 {
        r.cpu.set(Some(70.0));
        r.fans.tick(t0 + Duration::from_secs(s));
        let duty = duty_percent(&r, CPU_FAN);
        if last < 100 {
            assert!(duty > last, "the duty did not rise on tick {s}: {duties:?} then {duty}%");
        }
        last = duty;
        duties.push(duty);
    }
    assert_eq!(*duties.last().unwrap(), 100, "the boost reaches full speed within 10 ticks: {duties:?}");
}

#[test]
fn issue255_one_degree_over_the_line_rises_at_least_one_point_a_tick() {
    let t0 = Instant::now();
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(40.0));
    r.fans.set_header_mode("cpu_fan", flat_fifty_curve(), t0).unwrap();
    for s in 1..=3 {
        tick_cpu_at_temp(&mut r, t0, s, 40.0);
    }
    let mut last = duty_percent(&r, CPU_FAN);
    for s in 4..=10 {
        let duty = tick_cpu_at_temp(&mut r, t0, s, 51.0);
        let duty = ((u32::from(duty) * 100 + 127) / 255) as u8;
        assert!(
            duty > last,
            "51 C (the line plus one) rose only {last} -> {duty} at tick {s}: 1 point a tick is the minimum"
        );
        last = duty;
    }
}

#[test]
fn issue255_the_boost_decays_below_the_deadband_and_holds_inside_it() {
    let t0 = Instant::now();
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(40.0));
    r.fans.set_header_mode("cpu_fan", flat_fifty_curve(), t0).unwrap();
    for s in 1..=3 {
        tick_cpu_at_temp(&mut r, t0, s, 60.0);
    }
    let hot = duty_percent(&r, CPU_FAN);
    assert!(hot > 50, "60 C raised the duty: {hot}%");
    // Inside the deadband (49 C, one degree below the 50 C line) the boost holds.
    let mut held = Vec::new();
    for s in 4..=8 {
        tick_cpu_at_temp(&mut r, t0, s, 49.0);
        held.push(duty_percent(&r, CPU_FAN));
    }
    assert!(held.iter().all(|&d| d == hot), "the deadband held the boost at {hot}%: {held:?}");
    // Below the deadband (45 C, five degrees under) the boost decays to 0 and the duty returns
    // to the curve's 50 %.
    let mut last = held[0];
    for s in 9..=200 {
        last = tick_cpu_at_temp(&mut r, t0, s, 45.0);
        last = ((u32::from(last) * 100 + 127) / 255) as u8;
    }
    assert_eq!(last, 50, "after the boost decays the curve's 50 % returns: {last}%");
}

#[test]
fn issue255_an_oscillating_temperature_around_the_line_never_steps_the_duty_down() {
    let t0 = Instant::now();
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(40.0));
    r.fans.set_header_mode("cpu_fan", flat_fifty_curve(), t0).unwrap();
    let mut duties = Vec::new();
    for s in 1..=120 {
        let temp = if s % 2 == 0 { 50.5 } else { 49.5 };
        duties.push(tick_cpu_at_temp(&mut r, t0, s, temp));
    }
    let falls = duties.windows(2).filter(|w| w[1] < w[0]).count();
    assert_eq!(falls, 0, "the duty stepped down {falls} times over 120 ticks: {duties:?}");
    // And the closed loop did engage: the duty ended above where it started (a flat floor
    // alone leaves it where it was, which is what the old code did).
    assert!(duties.last() > duties.first(), "the boost never engaged: {duties:?}");
}

#[test]
fn issue255_fixed_mode_speeds_up_above_the_line() {
    let t0 = Instant::now();
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(40.0));
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 20 }, t0).unwrap();
    for s in 1..=3 {
        tick_at_temp(&mut r, t0, s, 40.0);
    }
    let start = duty_percent(&r, SYS_FAN1);
    assert!(start < 100, "40 C is below the line: {start}%");
    let mut duties = Vec::new();
    for s in 4..=10 {
        r.cpu.set(Some(60.0));
        r.fans.tick(t0 + Duration::from_secs(s));
        duties.push(duty_percent(&r, SYS_FAN1));
    }
    assert!(duties.last().unwrap() >= &90, "Fixed 20 % climbs with the boost: {duties:?}");
    assert!(duties.windows(2).all(|w| w[1] >= w[0]), "the Fixed duty never steps down while hot: {duties:?}");
}

#[test]
fn issue255_the_gpu_kind_is_covered() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let chip = FakeChip::new(MSI_ALT1_MAP);
    let gpu = crate::fakes::FakeGpuFans::new(1);
    let nct: Box<dyn FanChip> = Box::new(Nct6687::detect(chip.clone(), MSI_ALT1_MAP).unwrap());
    let composite = crate::gpu_fans::CompositeChip::new(Some(nct), Some(Box::new(gpu.clone())));
    let gpu_temp = FakeTemp::new(40.0);
    let temps: BTreeMap<TempSource, Box<dyn TempReader>> = BTreeMap::from([
        (TempSource::Cpu, Box::new(FakeTemp::new(40.0)) as Box<dyn TempReader>),
        (TempSource::Gpu, Box::new(gpu_temp.clone()) as Box<dyn TempReader>),
    ]);
    let config = FansFile { hot_limits: HotLimits { cpu_c: None, gpu_c: Some(50.0), board_c: None }, ..Default::default() };
    let mut fans = FanController::new(dir.path().to_path_buf(), Some(Box::new(composite)), None, temps, config, confirmed());
    fans.startup(t0);
    let curve = FanMode::Curve { source: TempSource::Gpu, points: vec![point(30.0, 30), point(70.0, 30)] };
    fans.set_header_mode("gpu_fan1", curve, t0).unwrap();
    assert_eq!(gpu.target_of(0), Some(30), "40 C is below the 50 C GPU line: the curve, not the boost");
    let mut targets = Vec::new();
    for s in 1..=5 {
        gpu_temp.set(Some(60.0));
        fans.tick(t0 + Duration::from_secs(s));
        targets.push(gpu.target_of(0).unwrap());
    }
    assert!(targets.last().unwrap() >= &60, "the GPU fan speeds up with the boost: {targets:?}");
    assert!(targets.windows(2).all(|w| w[1] >= w[0]), "the GPU duty never steps down while hot: {targets:?}");
}

#[test]
fn issue255_the_reason_is_the_stable_holding_sentence() {
    let t0 = Instant::now();
    let config = FansFile { hot_limits: HotLimits { cpu_c: Some(50.0), gpu_c: None, board_c: None }, ..Default::default() };
    let mut r = rig(config, t0);
    r.cpu.set(Some(40.0));
    r.fans.set_header_mode("cpu_fan", flat_fifty_curve(), t0).unwrap();
    for s in 1..=3 {
        tick_cpu_at_temp(&mut r, t0, s, 60.0);
    }
    let status = r.fans.status();
    assert_eq!(header(&status, "cpu_fan").override_reason.as_deref(), Some("holding CPU under 50 °C"));
    // The reason names the limit, never the live temperature: it is the same at 61 C.
    r.cpu.set(Some(61.0));
    r.fans.tick(t0 + Duration::from_secs(4));
    let status = r.fans.status();
    assert_eq!(header(&status, "cpu_fan").override_reason.as_deref(), Some("holding CPU under 50 °C"));
}

#[test]
fn issue130_an_invalid_limit_in_a_received_config_is_refused() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let bad = FansFile { hot_limits: HotLimits { cpu_c: Some(99.0), gpu_c: None, board_c: None }, ..Default::default() };
    let err = r.fans.put_config(bad, t0).err().unwrap();
    assert!(matches!(err, FanError::Validation(_)), "a limit past the ceiling gap is refused: {err}");
    // The running config is unchanged.
    let status = r.fans.status();
    assert_eq!(status.active_profile, "Default");
    let cpu = hot_of(&mut r, HotKind::Cpu);
    assert_eq!(cpu.limit_c, 74.0, "the refused write left the automatic line in force");
}

#[test]
fn issue130_an_unknown_cpu_brand_gets_the_74_degree_automatic_line() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let cpu = hot_of(&mut r, HotKind::Cpu);
    assert_eq!(cpu.limit_c, 74.0, "89 fallback minus the 15 C margin");
    assert_eq!(cpu.auto_c, 74.0);
    assert_eq!(cpu.ceiling_c, 84.0, "89 minus the 5 C ceiling margin");
    assert_eq!(cpu.max_settable_c, 82.0);
    assert_eq!(cpu.rated_max_c, None, "an unknown CPU reports no rated maximum");
    assert!(!cpu.user_set);
}

#[test]
fn issue130_the_gpu_slowdown_threshold_from_the_fake_nvml_sets_the_gpu_line() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    // The fake NVML reports a 91 C slowdown threshold: that becomes the GPU's rated
    // maximum, so the automatic line is 76 C and the ceiling 86 C.
    let gpu_fake = crate::fakes::FakeGpuFans::new(1);
    gpu_fake.set_slowdown_threshold_c(Some(91.0));
    r.fans.set_gpu_rated_max_c(gpu_fake.slowdown_threshold_c());
    let gpu = hot_of(&mut r, HotKind::Gpu);
    assert_eq!(gpu.limit_c, 76.0, "91 rated minus the 15 C margin");
    assert_eq!(gpu.auto_c, 76.0);
    assert_eq!(gpu.ceiling_c, 86.0, "91 minus the 5 C ceiling margin");
    assert_eq!(gpu.max_settable_c, 84.0);
    assert_eq!(gpu.rated_max_c, Some(91.0));
    assert!(!gpu.user_set);
}

#[test]
fn issue130_the_gpu_defaults_to_the_88_fallback_when_no_slowdown_is_reported() {
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let gpu = hot_of(&mut r, HotKind::Gpu);
    assert_eq!(gpu.limit_c, 73.0, "88 fallback minus the 15 C margin");
    assert_eq!(gpu.ceiling_c, 83.0);
    assert_eq!(gpu.rated_max_c, None);
    assert_eq!(r.fans.status().hot.len(), 3, "one status row per kind");
}

#[test]
fn the_brand_string_assembles_eax_ebx_ecx_edx_per_leaf() {
    use canvas_proto::fans::rated_cpu_max_c;
    use crate::fans::brand_string_from_leaves;
    // The registers of a known brand string: the string's bytes, NUL-padded to 48,
    // split into twelve little-endian u32s and grouped into three leaves as
    // (EAX, EBX, ECX, EDX) — the order the CPU documents for leaves 0x80000002..4.
    let mut bytes = [0u8; 48];
    let brand = "AMD Ryzen 7 7800X3D 8-Core Processor";
    bytes[..brand.len()].copy_from_slice(brand.as_bytes());
    let mut words = [0u32; 12];
    for (i, word) in words.iter_mut().enumerate() {
        *word = u32::from_le_bytes([bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2], bytes[i * 4 + 3]]);
    }
    let leaves = [
        (words[0], words[1], words[2], words[3]),
        (words[4], words[5], words[6], words[7]),
        (words[8], words[9], words[10], words[11]),
    ];
    let assembled = brand_string_from_leaves(leaves).expect("the brand string assembles");
    assert_eq!(assembled, brand, "the round trip through EAX EBX ECX EDX returns the brand");
    assert_eq!(rated_cpu_max_c(&assembled), Some(89.0), "the 7800X3D's rated maximum is 89 °C");
    assert_eq!(brand_string_from_leaves([(0, 0, 0, 0); 3]), None, "no string is None");
}

#[test]
fn the_startup_hot_limits_log_names_the_cpu_brand() {
    // The brand is set through the same path the service uses at startup.
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.fans.set_cpu_brand(Some("AMD Ryzen 7 7800X3D 8-Core Processor"));
    let cpu = hot_of(&mut r, HotKind::Cpu);
    assert_eq!(cpu.rated_max_c, Some(89.0), "the brand from the setter rates the CPU at 89 °C");
}

#[test]
fn a_fixed_header_holds_the_ceiling_duty_through_a_missing_reading_then_hands_back() {
    // Fixed 20 % with the CPU at 90 °C (above the 84 °C ceiling) writes 100 %. The
    // next tick has no CPU reading: while the ceiling is latched the service must hold
    // 100 % and never write the 20 % Fixed duty. After the 3 s sensor-loss timeout it
    // hands the header back to BIOS.
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(90.0));
    r.fans.tick(t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 20 }, t0).unwrap();
    assert_eq!(r.chip.duty_raw(SYS_FAN1), 255, "90 °C takes the ceiling's 100 %");
    r.cpu.set(None);
    r.fans.tick(t0 + Duration::from_secs(1));
    assert_eq!(r.chip.duty_raw(SYS_FAN1), 255, "no reading while latched: 100 % is held, not the 20 % Fixed duty");
    assert!(r.chip.manual(SYS_FAN1), "inside the sensor-loss window the header stays manual");
    r.fans.tick(t0 + Duration::from_secs(3));
    assert!(!r.chip.manual(SYS_FAN1), "after the sensor-loss timeout the header hands back to BIOS");
}

#[test]
fn an_edited_curve_keeps_the_ceiling_latch_until_the_hold_ends() {
    // Latched at 90 °C (ceiling 84 °C), cooled to 80 °C, then the points are edited:
    // the latch must survive the edit and the header stays at 100 % for the hold,
    // instead of dropping to the edited curve's lower duty.
    let t0 = Instant::now();
    let mut r = rig(FansFile::default(), t0);
    let curve = FanMode::Curve {
        source: TempSource::Cpu,
        points: vec![point(30.0, 20), point(70.0, 40)],
    };
    r.cpu.set(Some(90.0));
    r.fans.tick(t0);
    r.fans.set_header_mode("sys_fan1", curve, t0).unwrap();
    assert_eq!(r.chip.duty_raw(SYS_FAN1), 255, "90 °C latches the ceiling at 100 %");
    for s in 1..=3 {
        r.cpu.set(Some(90.0));
        r.fans.tick(t0 + Duration::from_secs(s));
    }
    r.cpu.set(Some(80.0));
    r.fans.tick(t0 + Duration::from_secs(4));
    assert_eq!(r.chip.duty_raw(SYS_FAN1), 255, "80 °C inside the hold still holds 100 %");
    // Edit the points for the same header and source.
    let edited = FanMode::Curve {
        source: TempSource::Cpu,
        points: vec![point(30.0, 20), point(80.0, 30)],
    };
    r.fans.set_header_mode("sys_fan1", edited, t0 + Duration::from_secs(4)).unwrap();
    r.cpu.set(Some(80.0));
    r.fans.tick(t0 + Duration::from_secs(5));
    let duty = r.chip.duty_raw(SYS_FAN1);
    assert_eq!(duty, 255, "the points edit kept the ceiling latch: {duty}");
    // After the hold below the ceiling the latch releases.
    let mut last = duty;
    for s in 6..=70 {
        r.cpu.set(Some(80.0));
        r.fans.tick(t0 + Duration::from_secs(s));
        last = r.chip.duty_raw(SYS_FAN1);
    }
    assert_ne!(last, 255, "after 60 s below the ceiling the latch released: {last}");
}
