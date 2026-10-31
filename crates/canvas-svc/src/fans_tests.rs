//! Controller tests over the register simulation (`FakeChip`) and fake temperatures.

use crate::chip::{FanChip, Nct6687, COMMAND_REG, MODE_REG, MSI_ALT1_MAP, SYS_MODE_REG};
use crate::fakes::{FakeChip, FakeTemp};
use crate::fans::{curve_percent, FanController, FanError, RESUME_GRACE};
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
    let mut r = rig(FansFile::default(), t0);
    r.cpu.set(Some(55.0));
    r.fans.tick(t0);
    r.fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 20 }, t0).unwrap();
    let status = r.fans.status();
    let h = header(&status, "sys_fan1");
    assert_eq!(h.pwm_percent, Some(35), "hot floor for non-CPU headers is 35 %");
    assert_eq!(h.mode, FanMode::Fixed { percent: 20 }, "the request is kept");
    assert!(h.override_reason.as_deref().unwrap_or_default().starts_with("hot floor 35%"));

    r.fans.set_header_mode("cpu_fan", FanMode::Fixed { percent: 30 }, t0).unwrap();
    let status = r.fans.status();
    assert_eq!(header(&status, "cpu_fan").pwm_percent, Some(50), "hot floor for CPU_FAN is 50 %");
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
fn a_handback_inside_the_resume_window_is_retried_once_after_the_grace() {
    let t0 = Instant::now();
    let mut r = driven_rig(t0);
    r.fans.on_suspend();
    r.chip.simulate_resume();
    let t1 = t0 + Duration::from_secs(60);
    r.fans.on_resume(t1, "test");

    // A user applies a second header inside the window; firmware owns the controller, so it fails.
    r.fans.set_header_mode("pump_fan", FanMode::Fixed { percent: 60 }, t1 + Duration::from_secs(1)).unwrap();
    assert!(!r.chip.manual(1), "the failed write handed pump_fan back");
    assert!(header(&r.fans.status(), "pump_fan").override_reason.is_some());

    r.chip.release_to_service();
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(1), wall(70));
    assert!(r.chip.manual(1), "the retry after the grace took pump_fan back");
    assert!(header(&r.fans.status(), "pump_fan").override_reason.is_none());
    assert_under_control(&mut r);
}

#[test]
fn a_real_persistent_failure_still_hands_back_and_stays_back_after_the_one_retry() {
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
    assert!(!r.chip.manual(1), "handed back inside the window");

    // Inside the grace the loop leaves the chip alone, so nothing gets handed back by a tick.
    let in_grace = r.chip.writes();
    r.fans.tick_at(t1 + Duration::from_secs(2), wall(62));
    assert_eq!(r.chip.writes(), in_grace, "no tick write inside the grace");

    let before_retry = r.chip.writes();
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(1), wall(70));
    assert!(r.chip.writes() > before_retry, "the one retry wrote to the chip");
    assert!(!r.chip.manual(1), "pump_fan stays on BIOS");
    assert!(!r.chip.manual(SYS_FAN1), "sys_fan1 stays on BIOS");
    let status = r.fans.status();
    assert!(header(&status, "pump_fan").override_reason.as_deref().is_some_and(|m| m.starts_with("write failed")));
    assert!(header(&status, "sys_fan1").override_reason.as_deref().is_some_and(|m| m.starts_with("write failed")));

    // Sticky: a later tick makes no third attempt, even if the chip recovers.
    r.chip.set_drop_duty_writes(false);
    let settled = r.chip.writes();
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(2), wall(71));
    r.fans.tick_at(t1 + RESUME_GRACE + Duration::from_secs(3), wall(72));
    assert_eq!(r.chip.writes(), settled, "no write after the retry failed");
    assert!(!r.chip.manual(1) && !r.chip.manual(SYS_FAN1));
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
