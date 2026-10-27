//! GPU fan headers through the composite chip, over the NCT register simulation and
//! `FakeGpuFans`. Nothing here loads NVML.

use crate::chip::{FanChip, Nct6687, MSI_ALT1_MAP};
use crate::fakes::{FakeChip, FakeGpuFans, FakeTemp};
use crate::fans::FanController;
use crate::gpu_fans::{CompositeChip, NVML_ONLY_MAP};
use crate::temps::TempReader;
use canvas_proto::config::FansFile;
use canvas_proto::fans::{FanMode, TempSource};
use std::collections::BTreeMap;
use std::time::Instant;
use tempfile::TempDir;

/// MSI alt1 has nine channels, so the GPU headers follow at 9 and 10.
const GPU_FAN1: usize = 9;
const GPU_FAN2: usize = 10;
const SYS_FAN1: usize = 2;

fn nct_box(chip: &FakeChip) -> Box<dyn FanChip> {
    Box::new(Nct6687::detect(chip.clone(), MSI_ALT1_MAP).unwrap())
}

fn composite(chip: &FakeChip, gpu: &FakeGpuFans) -> CompositeChip {
    CompositeChip::new(Some(nct_box(chip)), Some(Box::new(gpu.clone())))
}

fn temps() -> BTreeMap<TempSource, Box<dyn TempReader>> {
    BTreeMap::from([(TempSource::Cpu, Box::new(FakeTemp::new(40.0)) as Box<dyn TempReader>)])
}

fn controller(dir: &TempDir, chip: Box<dyn FanChip>, confirmed: Option<String>, t0: Instant) -> FanController {
    let mut fans =
        FanController::new(dir.path().to_path_buf(), Some(chip), None, temps(), FansFile::default(), confirmed);
    fans.startup(t0);
    fans
}

fn confirmed() -> Option<String> {
    Some(MSI_ALT1_MAP.name.to_string())
}

fn header<'a>(status: &'a canvas_proto::fans::FansStatus, id: &str) -> &'a canvas_proto::fans::FanHeaderStatus {
    status.headers.iter().find(|h| h.id == id).unwrap()
}

#[test]
fn nct_indices_go_to_the_nct_chip_and_gpu_indices_to_the_gpu() {
    let nct = FakeChip::new(MSI_ALT1_MAP);
    let gpu = FakeGpuFans::new(2);
    let mut chip = composite(&nct, &gpu);

    let ids: Vec<&str> = chip.channels().iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), 11);
    assert_eq!(&ids[..9], MSI_ALT1_MAP.channels.iter().map(|c| c.id).collect::<Vec<_>>().as_slice());
    assert_eq!(&ids[9..], ["gpu_fan1", "gpu_fan2"]);
    assert_eq!(chip.channels()[GPU_FAN1].label, "GPU fan 1");
    assert!(chip.channels()[GPU_FAN1].controllable() && chip.channels()[GPU_FAN2].controllable());

    // An NCT write lands in the NCT simulation and never reaches the GPU.
    chip.write_duty_percent(SYS_FAN1, 60).unwrap();
    assert!(nct.manual(SYS_FAN1));
    assert!(gpu.speed_calls().is_empty() && gpu.default_calls().is_empty());

    // A GPU write lands on the GPU fake with the NVML index and never reaches the NCT chip.
    let nct_writes = nct.writes();
    chip.write_duty_percent(GPU_FAN2, 55).unwrap();
    assert_eq!(gpu.speed_calls(), [(1, 55)]);
    assert_eq!(nct.writes(), nct_writes);
    assert!(chip.read_manual(GPU_FAN2).unwrap());
    assert!(!chip.read_manual(GPU_FAN1).unwrap());
    assert_eq!(chip.read_duty_percent(GPU_FAN2).unwrap(), Some(55));
    assert_eq!(chip.read_rpm(GPU_FAN1).unwrap(), None);
    assert!(chip.read_rpm(SYS_FAN1).unwrap().is_some());
    assert!(chip.read_duty_percent(11).is_err());
    assert_eq!(chip.map_name(), "msi_alt1");
    assert_eq!(chip.chip_name(), "NCT6687D-R @ 0xA20 + NVML 2 fans");
    assert_eq!(chip.board_sensor_names().len(), 4);
}

#[test]
fn a_fixed_gpu_fan_writes_the_speed_reads_the_target_back_and_reports_manual() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let nct = FakeChip::new(MSI_ALT1_MAP);
    let gpu = FakeGpuFans::new(2);
    let mut fans = controller(&dir, Box::new(composite(&nct, &gpu)), confirmed(), t0);
    gpu.clear_calls();

    fans.set_header_mode("gpu_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert_eq!(gpu.speed_calls(), [(0, 50)]);
    assert!(gpu.default_calls().is_empty(), "entering manual makes no default call");
    assert_eq!(gpu.target_of(0), Some(50));
    let status = fans.status();
    let h = header(&status, "gpu_fan1");
    assert!(!h.on_bios, "{h:?}");
    assert_eq!(h.pwm_percent, Some(50));
    assert_eq!(h.rpm, None);
    assert!(header(&status, "gpu_fan2").on_bios);
    assert_eq!(status.chip.as_deref(), Some("NCT6687D-R @ 0xA20 + NVML 2 fans"));

    // Back to BIOS: the driver's auto control, and the status says so.
    fans.set_header_mode("gpu_fan1", FanMode::Bios, t0).unwrap();
    assert_eq!(gpu.default_calls(), [0]);
    assert!(!gpu.manual_of(0));
    assert!(header(&fans.status(), "gpu_fan1").on_bios);
}

#[test]
fn a_value_below_the_nvml_minimum_is_clamped_to_30_at_the_chip() {
    let nct = FakeChip::new(MSI_ALT1_MAP);
    let gpu = FakeGpuFans::new(2);
    let mut chip = composite(&nct, &gpu);
    chip.write_duty_percent(GPU_FAN1, 10).unwrap();
    chip.write_duty_percent(GPU_FAN2, 0).unwrap();
    chip.write_duty_percent(GPU_FAN2, 250).unwrap();
    assert_eq!(gpu.speed_calls(), [(0, 30), (1, 30), (1, 100)]);
}

#[test]
fn service_stop_and_startup_hand_both_gpu_fans_back() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let nct = FakeChip::new(MSI_ALT1_MAP);
    let gpu = FakeGpuFans::new(2);
    let mut fans = controller(&dir, Box::new(composite(&nct, &gpu)), confirmed(), t0);
    fans.set_header_mode("gpu_fan1", FanMode::Fixed { percent: 60 }, t0).unwrap();
    fans.set_header_mode("gpu_fan2", FanMode::Fixed { percent: 70 }, t0).unwrap();
    assert!(gpu.manual_of(0) && gpu.manual_of(1));

    // Service stop.
    gpu.clear_calls();
    fans.restore_all("service stop");
    assert_eq!(gpu.default_calls(), [0, 1]);
    assert!(!gpu.manual_of(0) && !gpu.manual_of(1));

    // A controller dropped without a stop leaves both fans manual; the next startup frees them.
    fans.set_header_mode("gpu_fan1", FanMode::Fixed { percent: 60 }, t0).unwrap();
    fans.set_header_mode("gpu_fan2", FanMode::Fixed { percent: 70 }, t0).unwrap();
    drop(fans);
    assert!(gpu.manual_of(0) && gpu.manual_of(1));
    gpu.clear_calls();
    let _restarted = controller(&dir, Box::new(composite(&nct, &gpu)), confirmed(), t0);
    assert_eq!(gpu.default_calls(), [0, 1]);
    assert!(!gpu.manual_of(0) && !gpu.manual_of(1));

    // The `--restore-bios` path.
    gpu.clear_calls();
    let mut chip = composite(&nct, &gpu);
    let restored = FanController::restore_chip_only(&mut chip).unwrap();
    assert_eq!(restored, 8 + 2, "eight controllable NCT headers and two GPU fans");
    assert_eq!(gpu.default_calls(), [0, 1]);
}

#[test]
fn a_gpu_write_error_hands_back_only_that_gpu_header() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let nct = FakeChip::new(MSI_ALT1_MAP);
    let gpu = FakeGpuFans::new(2);
    let mut fans = controller(&dir, Box::new(composite(&nct, &gpu)), confirmed(), t0);
    fans.set_header_mode("sys_fan1", FanMode::Fixed { percent: 60 }, t0).unwrap();
    fans.set_header_mode("gpu_fan2", FanMode::Fixed { percent: 60 }, t0).unwrap();
    gpu.clear_calls();
    let nct_writes = nct.writes();

    gpu.set_fail_set_speed(true);
    fans.set_header_mode("gpu_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    assert_eq!(gpu.default_calls(), [0], "only gpu_fan1 is handed back");
    assert_eq!(nct.writes(), nct_writes, "the NCT headers are untouched");
    assert!(nct.manual(SYS_FAN1));
    assert!(gpu.manual_of(1), "gpu_fan2 stays on its profile");
    let status = fans.status();
    let h = header(&status, "gpu_fan1");
    assert!(h.on_bios);
    assert!(h.override_reason.as_deref().is_some_and(|r| r.contains("write failed")), "{h:?}");
    assert!(!header(&status, "gpu_fan2").on_bios);
}

#[test]
fn without_the_nct_chip_the_map_is_never_confirmed_and_nothing_is_written() {
    let t0 = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let gpu = FakeGpuFans::new(2);
    let chip = CompositeChip::new(None, Some(Box::new(gpu.clone())));
    assert_eq!(chip.map_name(), NVML_ONLY_MAP);
    assert_eq!(chip.chip_name(), "NVML 2 fans");
    assert!(chip.board_sensor_names().is_empty());
    let ids: Vec<&str> = chip.channels().iter().map(|c| c.id).collect();
    assert_eq!(ids, ["gpu_fan1", "gpu_fan2"]);

    // Even a chip.toml naming the NCT map does not allow writes.
    let mut fans = controller(&dir, Box::new(chip), confirmed(), t0);
    assert!(!fans.writes_allowed());
    gpu.clear_calls();
    fans.set_header_mode("gpu_fan1", FanMode::Fixed { percent: 50 }, t0).unwrap();
    fans.tick(t0);
    assert!(gpu.speed_calls().is_empty(), "no speed is written");
    assert!(!gpu.manual_of(0));
    // The handback still runs.
    fans.restore_all("test");
    assert_eq!(gpu.default_calls(), [0, 1]);
}

#[test]
fn a_gpu_that_cannot_count_its_fans_adds_no_headers() {
    let nct = FakeChip::new(MSI_ALT1_MAP);
    let gpu = FakeGpuFans::new(2);
    gpu.set_fail_count(true);
    let chip = composite(&nct, &gpu);
    assert_eq!(chip.channels().len(), 9);
    assert_eq!(chip.chip_name(), "NCT6687D-R @ 0xA20");
}
