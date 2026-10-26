//! Controller tests over the register simulation (`FakeChip`) and fake temperatures.

use crate::chip::{FanChip, Nct6687, MODE_REG, MSI_ALT1_MAP};
use crate::fakes::{FakeChip, FakeTemp};
use crate::fans::{curve_percent, FanController, FanError};
use crate::paths;
use crate::store;
use crate::temps::TempReader;
use canvas_proto::config::FansFile;
use canvas_proto::fans::{CurvePoint, FanMode, FanProfile, TempSource};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
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
    assert!(log.iter().all(|(reg, _)| *reg == MODE_REG), "only the fan-mode register: {log:?}");
    assert!(log.iter().all(|(reg, _)| !duty_regs.contains(reg)), "no duty register: {log:?}");
    assert!(log.iter().all(|(_, value)| *value == 0), "every value is the auto value: {log:?}");
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
    // And the profile did not re-engage it.
    unconfirmed.tick(t0 + Duration::from_secs(1));
    assert!(!r.chip.manual(SYS_FAN1));
    let since = r.chip.writes_log()[before..].to_vec();
    assert!(!since.is_empty(), "the restore wrote the mode register");
    assert!(since.iter().all(|(reg, _)| *reg == MODE_REG), "only the fan-mode register: {since:?}");
    // Each write clears bits and never sets one, and the register ends at the auto value (0).
    let values: Vec<u8> = since.iter().map(|(_, v)| *v).collect();
    assert!(values.windows(2).all(|w| w[1] & !w[0] == 0), "writes only clear manual bits: {values:?}");
    assert_eq!(values.last().copied(), Some(0), "ends at auto: {values:?}");
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
    assert!(after[before..].iter().all(|reg| *reg == MODE_REG), "only the fan-mode register: {:?}", &after[before..]);
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
    assert!(header(&status, "sys_fan1").override_reason.as_deref().unwrap_or_default().starts_with("write failed: readback"));
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
    assert_eq!(chip.read_board_temp("board_temp6").unwrap(), Some(0.0));
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
