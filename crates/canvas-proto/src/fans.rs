//! Fan headers, modes, curves, profiles and the safety floors.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// Header ids are lowercase chip channel names, e.g. `cpu_fan`, `pump_fan`, `sys_fan1`.
pub type HeaderId = String;

pub const CPU_FAN_HEADER: &str = "cpu_fan";

/// Default configurable floor for every header (spec, fans line 5).
pub const DEFAULT_FLOOR_PERCENT: u8 = 20;
/// CPU_FAN never goes below this, whatever the configured floor (spec).
pub const CPU_FAN_MIN_PERCENT: u8 = 30;
/// GPU fan headers are `gpu_fan1`, `gpu_fan2`, ... (NVML fan index + 1). Added at the user's
/// request (lighting session 2), after an NVML write test on the test RTX 5070 Ti.
pub const GPU_FAN_HEADER_PREFIX: &str = "gpu_fan";
/// GPU fans never go below this: NVML's own manual minimum on the RTX 5070 Ti
/// (`nvmlDeviceGetMinMaxFanSpeed` = 30..100).
pub const GPU_FAN_MIN_PERCENT: u8 = 30;
/// G4: while a manual header's curve temperature is at or above this, the hot floors apply.
pub const HOT_TEMP_C: f32 = 50.0;
/// G4: hot floor for every header in a manual mode.
pub const HOT_FLOOR_PERCENT: u8 = 35;
/// G4: hot floor for CPU_FAN in a manual mode.
pub const HOT_CPU_FAN_FLOOR_PERCENT: u8 = 50;
/// Spec: a curve's temperature source missing for this long hands its headers to BIOS.
pub const SENSOR_LOSS_TIMEOUT_MS: u64 = 3000;
/// Spec: curve loop rate.
pub const CURVE_LOOP_HZ: u32 = 1;
pub const CURVE_MIN_POINTS: usize = 2;
pub const CURVE_MAX_POINTS: usize = 8;
/// G4: the daemon shows a red banner and tray warning after the pipe is gone this long.
pub const PIPE_LOST_WARN_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "type", content = "name")]
pub enum TempSource {
    /// AMD Tctl via PawnIO.
    Cpu,
    /// NVML.
    Gpu,
    /// A named NCT6687D-R board sensor.
    Board(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CurvePoint {
    pub temp_c: f32,
    pub percent: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum FanMode {
    /// Chip's own automatic control, as the BIOS configured it.
    Bios,
    Fixed { percent: u8 },
    /// 2..=8 points, sorted by `temp_c`; linear between points, flat outside.
    Curve { source: TempSource, points: Vec<CurvePoint> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FanProfile {
    pub name: String,
    /// Headers not listed stay on BIOS.
    pub headers: BTreeMap<HeaderId, FanMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FanHeaderStatus {
    pub id: HeaderId,
    pub label: String,
    pub rpm: Option<u32>,
    /// PWM duty the chip reports, 0..100.
    pub pwm_percent: Option<u8>,
    /// What was asked for.
    pub mode: FanMode,
    /// What the chip is doing: `true` when it is under BIOS control right now
    /// (asked for BIOS, or handed back by safety).
    pub on_bios: bool,
    /// Why the header is not doing what `mode` asks (safety handback, hot floor), if so.
    pub override_reason: Option<String>,
    /// `false` when this board does not route the channel to a header (issue #128: the
    /// chip has the channel, the board has no connector). Such a header stays in the list,
    /// is never written, and cannot be ticked in bulk. Old payloads without it parse as
    /// available.
    #[serde(default = "available_by_default")]
    pub available: bool,
}

fn available_by_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TempReading {
    pub source: TempSource,
    pub label: String,
    pub celsius: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FansStatus {
    /// `"NCT6687D-R @ 0xA20"` once detected.
    pub chip: Option<String>,
    /// G4: whether the chip has a hardware fan watchdog reachable via PawnIO
    /// (`None` = not settled).
    pub hardware_watchdog: Option<bool>,
    pub headers: Vec<FanHeaderStatus>,
    pub temps: Vec<TempReading>,
    pub active_profile: String,
    pub profiles: Vec<String>,
    pub floor_percent: u8,
    /// Issue #130: the hot line in force per kind of source (CPU, GPU, board), for the curve
    /// chart. Old payloads without it parse as empty.
    #[serde(default)]
    pub hot: Vec<HotLimitStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FanValidationError {
    #[error("{header}: {percent}% is below the floor of {floor}%")]
    BelowFloor { header: HeaderId, percent: u8, floor: u8 },
    #[error("{header}: {percent}% is above 100%")]
    AbovePercent { header: HeaderId, percent: u8 },
    #[error("{header}: a curve needs {CURVE_MIN_POINTS} to {CURVE_MAX_POINTS} points, got {got}")]
    CurvePoints { header: HeaderId, got: usize },
    #[error("{header}: curve points must be sorted by temperature with no repeats")]
    CurveUnsorted { header: HeaderId },
}

/// The lowest percent a header may be set to (what the API rejects below with 422).
/// The configured floor can only raise the default: a floor below
/// [`DEFAULT_FLOOR_PERCENT`] from an imported file, a config edit or a pipe peer is ignored.
pub fn floor_for(header: &str, configured_floor: u8) -> u8 {
    let floor = configured_floor.max(DEFAULT_FLOOR_PERCENT);
    if header == CPU_FAN_HEADER {
        floor.max(CPU_FAN_MIN_PERCENT)
    } else if header.starts_with(GPU_FAN_HEADER_PREFIX) {
        floor.max(GPU_FAN_MIN_PERCENT)
    } else {
        floor
    }
}

/// G4: the floor applied at runtime, given the curve temperature for this header.
pub fn runtime_floor(header: &str, configured_floor: u8, temp_c: Option<f32>) -> u8 {
    let base = floor_for(header, configured_floor);
    match temp_c {
        Some(t) if t >= HOT_TEMP_C => {
            let hot = if header == CPU_FAN_HEADER {
                HOT_CPU_FAN_FLOOR_PERCENT
            } else {
                HOT_FLOOR_PERCENT
            };
            base.max(hot)
        }
        _ => base,
    }
}

/// Rejects any mode that could drive `header` below its floor. Used by the API (422),
/// the service, and config import (G10).
pub fn validate_mode(header: &str, mode: &FanMode, configured_floor: u8) -> Result<(), FanValidationError> {
    let floor = floor_for(header, configured_floor);
    let check = |percent: u8| {
        if percent > 100 {
            Err(FanValidationError::AbovePercent { header: header.into(), percent })
        } else if percent < floor {
            Err(FanValidationError::BelowFloor { header: header.into(), percent, floor })
        } else {
            Ok(())
        }
    };
    match mode {
        FanMode::Bios => Ok(()),
        FanMode::Fixed { percent } => check(*percent),
        FanMode::Curve { points, .. } => {
            if !(CURVE_MIN_POINTS..=CURVE_MAX_POINTS).contains(&points.len()) {
                return Err(FanValidationError::CurvePoints { header: header.into(), got: points.len() });
            }
            if points.windows(2).any(|w| w[0].temp_c >= w[1].temp_c) {
                return Err(FanValidationError::CurveUnsorted { header: header.into() });
            }
            points.iter().try_for_each(|p| check(p.percent))
        }
    }
}

pub fn validate_profile(profile: &FanProfile, configured_floor: u8) -> Result<(), FanValidationError> {
    profile
        .headers
        .iter()
        .try_for_each(|(h, m)| validate_mode(h, m, configured_floor))
}

// ---------------------------------------------------------------------------------------------
// Issue #130: the user says what "hot" is ("don't exceed"), per kind of temperature source,
// instead of the fixed `HOT_TEMP_C`. At or above a source's hot limit, the hot floors above
// apply to every manual header whose curve reads that source (today's G4 shape, with the line
// movable). At or above the source's ceiling, no setting can stop those headers going to 100 %.
// ---------------------------------------------------------------------------------------------

/// The kind of temperature source a hot limit is set for. Every board sensor shares one limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum HotKind {
    Cpu,
    Gpu,
    Board,
}

impl TempSource {
    pub fn hot_kind(&self) -> HotKind {
        match self {
            TempSource::Cpu => HotKind::Cpu,
            TempSource::Gpu => HotKind::Gpu,
            TempSource::Board(_) => HotKind::Board,
        }
    }
}

/// The automatic hot limit sits this far under the part's rated maximum.
pub const HOT_MARGIN_C: f32 = 15.0;
/// The safety ceiling sits this far under the part's rated maximum. Fixed: no setting moves it.
pub const CEILING_MARGIN_C: f32 = 5.0;
/// A hot limit may be set no closer to the ceiling than this.
pub const HOT_LIMIT_CEILING_GAP_C: f32 = 2.0;
/// No hot limit may be set below this.
pub const HOT_LIMIT_MIN_C: f32 = 40.0;
/// The rated maximum used for a CPU that is not in [`rated_cpu_max_c`]'s table. The lowest
/// Tjmax among the listed parts (7800X3D), so an unknown CPU gets the most cautious line.
pub const FALLBACK_CPU_MAX_C: f32 = 89.0;
/// The rated maximum used for a GPU whose driver does not report a slowdown threshold.
pub const FALLBACK_GPU_MAX_C: f32 = 88.0;
/// The band a reported rated maximum must fall in to be believed (°C). Outside it, the
/// fallback is used. Every listed part is inside it (89..=95).
pub const RATED_MAX_SANE_MIN_C: f32 = 60.0;
pub const RATED_MAX_SANE_MAX_C: f32 = 115.0;
/// Board sensors have no rated part maximum: the automatic limit stays today's `HOT_TEMP_C`,
/// and the ceiling is this fixed value.
pub const BOARD_CEILING_C: f32 = 90.0;

/// Rated maximum operating temperature (Tjmax) of a CPU, from its brand string, when it is
/// one we have the vendor's figure for. Source: AMD's product pages (checked 2026-11-05).
pub fn rated_cpu_max_c(brand: &str) -> Option<f32> {
    let b = brand.to_ascii_uppercase();
    const TABLE: &[(&str, f32)] = &[("7800X3D", 89.0), ("9800X3D", 95.0), ("5800X3D", 90.0)];
    TABLE.iter().find(|(model, _)| b.contains(model)).map(|(_, c)| *c)
}

/// What the user set. `None` for a kind means automatic: the part's rated maximum minus
/// [`HOT_MARGIN_C`] (board sensors: `HOT_TEMP_C`). Old `fans.toml` files without it read as
/// all automatic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HotLimits {
    #[serde(default)]
    pub cpu_c: Option<f32>,
    #[serde(default)]
    pub gpu_c: Option<f32>,
    #[serde(default)]
    pub board_c: Option<f32>,
}

impl HotLimits {
    pub fn get(&self, kind: HotKind) -> Option<f32> {
        match kind {
            HotKind::Cpu => self.cpu_c,
            HotKind::Gpu => self.gpu_c,
            HotKind::Board => self.board_c,
        }
    }
}

/// The hot limit in force for one kind, as the editor draws it on the curve chart.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HotLimitStatus {
    pub kind: HotKind,
    /// The line in force: the user's value, or the automatic one.
    pub limit_c: f32,
    /// `true` when the user set it; `false` when it is automatic.
    pub user_set: bool,
    /// The automatic value (what "Reset to automatic" gives).
    pub auto_c: f32,
    /// The part's rated maximum, when known (`None` for board sensors).
    pub rated_max_c: Option<f32>,
    /// At or above this, every manual header on this source goes to 100 %. Fixed.
    pub ceiling_c: f32,
    /// The highest value the user may set.
    pub max_settable_c: f32,
}

/// The status of one kind, given what the user set and the part's rated maximum
/// (`rated_max_c` is ignored for [`HotKind::Board`]).
pub fn hot_limit_status(kind: HotKind, limits: &HotLimits, rated_max_c: Option<f32>) -> HotLimitStatus {
    let (auto_c, rated, ceiling_c) = match kind {
        HotKind::Board => (HOT_TEMP_C, None, BOARD_CEILING_C),
        HotKind::Cpu | HotKind::Gpu => {
            let fallback = if kind == HotKind::Cpu { FALLBACK_CPU_MAX_C } else { FALLBACK_GPU_MAX_C };
            // A rated maximum outside a sane band (a driver that reports 0, or 200) is not
            // believed: the cautious fallback stands, so the ceiling can never move up past a
            // real part's limit and the settable range is never empty.
            let sane = rated_max_c.filter(|r| r.is_finite() && (RATED_MAX_SANE_MIN_C..=RATED_MAX_SANE_MAX_C).contains(r));
            let rated = sane.unwrap_or(fallback);
            (rated - HOT_MARGIN_C, sane, rated - CEILING_MARGIN_C)
        }
    };
    let max_settable_c = ceiling_c - HOT_LIMIT_CEILING_GAP_C;
    let user = limits.get(kind);
    HotLimitStatus {
        kind,
        limit_c: user.map(|c| c.clamp(HOT_LIMIT_MIN_C, max_settable_c)).unwrap_or(auto_c),
        user_set: user.is_some(),
        auto_c,
        rated_max_c: rated,
        ceiling_c,
        max_settable_c,
    }
}

/// Rejects a hot limit outside `HOT_LIMIT_MIN_C..=max_settable_c` (or not a finite number).
/// Used by the API (422), the service and config import.
pub fn validate_hot_limits(
    limits: &HotLimits,
    rated_cpu_max_c: Option<f32>,
    rated_gpu_max_c: Option<f32>,
) -> Result<(), HotLimitError> {
    for (kind, rated) in [(HotKind::Cpu, rated_cpu_max_c), (HotKind::Gpu, rated_gpu_max_c), (HotKind::Board, None)] {
        if let Some(c) = limits.get(kind) {
            let max = hot_limit_status(kind, limits, rated).max_settable_c;
            if !c.is_finite() || c < HOT_LIMIT_MIN_C || c > max {
                return Err(HotLimitError::OutOfRange { kind, celsius: c, min: HOT_LIMIT_MIN_C, max });
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HotLimitError {
    #[error("the {kind:?} hot limit of {celsius} °C is outside {min} to {max} °C")]
    OutOfRange { kind: HotKind, celsius: f32, min: f32, max: f32 },
}

/// [`runtime_floor`] with the hot line at `hot_c` instead of the fixed `HOT_TEMP_C`.
pub fn runtime_floor_at(header: &str, configured_floor: u8, temp_c: Option<f32>, hot_c: f32) -> u8 {
    let base = floor_for(header, configured_floor);
    match temp_c {
        Some(t) if t >= hot_c => {
            let hot = if header == CPU_FAN_HEADER { HOT_CPU_FAN_FLOOR_PERCENT } else { HOT_FLOOR_PERCENT };
            base.max(hot)
        }
        _ => base,
    }
}

#[cfg(test)]
mod hot_limit_tests {
    use super::*;

    #[test]
    fn the_7800x3d_gets_74_automatic_and_an_84_ceiling() {
        let rated = rated_cpu_max_c("AMD Ryzen 7 7800X3D 8-Core Processor");
        assert_eq!(rated, Some(89.0));
        let s = hot_limit_status(HotKind::Cpu, &HotLimits::default(), rated);
        assert_eq!((s.limit_c, s.auto_c, s.ceiling_c, s.max_settable_c, s.user_set), (74.0, 74.0, 84.0, 82.0, false));
    }

    #[test]
    fn an_unknown_cpu_uses_the_cautious_fallback() {
        assert_eq!(rated_cpu_max_c("Some Future CPU"), None);
        let s = hot_limit_status(HotKind::Cpu, &HotLimits::default(), None);
        assert_eq!((s.limit_c, s.ceiling_c, s.rated_max_c), (FALLBACK_CPU_MAX_C - HOT_MARGIN_C, FALLBACK_CPU_MAX_C - CEILING_MARGIN_C, None));
    }

    #[test]
    fn board_sensors_keep_todays_line_by_default() {
        let s = hot_limit_status(HotKind::Board, &HotLimits::default(), Some(200.0));
        assert_eq!((s.limit_c, s.ceiling_c, s.rated_max_c), (HOT_TEMP_C, BOARD_CEILING_C, None));
    }

    #[test]
    fn a_user_value_wins_and_a_value_past_the_ceiling_is_refused() {
        let limits = HotLimits { cpu_c: Some(70.0), ..Default::default() };
        let s = hot_limit_status(HotKind::Cpu, &limits, Some(89.0));
        assert_eq!((s.limit_c, s.user_set), (70.0, true));
        assert!(validate_hot_limits(&limits, Some(89.0), None).is_ok());
        let too_hot = HotLimits { cpu_c: Some(83.0), ..Default::default() };
        assert!(validate_hot_limits(&too_hot, Some(89.0), None).is_err());
        let too_cold = HotLimits { gpu_c: Some(30.0), ..Default::default() };
        assert!(validate_hot_limits(&too_cold, None, None).is_err());
        let nan = HotLimits { board_c: Some(f32::NAN), ..Default::default() };
        assert!(validate_hot_limits(&nan, None, None).is_err());
    }

    #[test]
    fn the_floor_follows_the_movable_line() {
        assert_eq!(runtime_floor_at(CPU_FAN_HEADER, 20, Some(60.0), 74.0), CPU_FAN_MIN_PERCENT);
        assert_eq!(runtime_floor_at(CPU_FAN_HEADER, 20, Some(74.0), 74.0), HOT_CPU_FAN_FLOOR_PERCENT);
        assert_eq!(runtime_floor_at("sys_fan1", 20, Some(74.0), 74.0), HOT_FLOOR_PERCENT);
        // The fixed-line function is unchanged.
        assert_eq!(runtime_floor(CPU_FAN_HEADER, 20, Some(50.0)), runtime_floor_at(CPU_FAN_HEADER, 20, Some(50.0), HOT_TEMP_C));
    }

    #[test]
    fn a_nonsense_rated_maximum_falls_back_and_never_panics() {
        for bad in [0.0, -5.0, 200.0, f32::NAN, f32::INFINITY, 59.0, 116.0] {
            let s = hot_limit_status(HotKind::Gpu, &HotLimits { gpu_c: Some(70.0), ..Default::default() }, Some(bad));
            assert_eq!(s.rated_max_c, None, "{bad} is not believed");
            assert_eq!(s.ceiling_c, FALLBACK_GPU_MAX_C - CEILING_MARGIN_C);
            assert!(s.max_settable_c >= HOT_LIMIT_MIN_C);
            assert!(validate_hot_limits(&HotLimits { gpu_c: Some(70.0), ..Default::default() }, None, Some(bad)).is_ok());
        }
    }

    #[test]
    fn old_files_without_hot_limits_read_as_automatic() {
        let parsed: HotLimits = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, HotLimits::default());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_header_status_without_available_parses_as_available() {
        // Issue #128: payloads from before the field must still parse, as available headers.
        let old = r#"{"id":"sys_fan1","label":"System fan 1","rpm":null,"pwm_percent":null,"mode":{"mode":"bios"},"on_bios":true,"override_reason":null}"#;
        let status: FanHeaderStatus = serde_json::from_str(old).expect("an old payload parses");
        assert!(status.available);
        let back = serde_json::to_value(&status).unwrap();
        assert_eq!(back["available"], serde_json::json!(true));
    }

    use super::*;

    #[test]
    fn floors() {
        assert_eq!(floor_for("sys_fan1", 20), 20);
        assert_eq!(floor_for(CPU_FAN_HEADER, 20), 30);
        assert_eq!(floor_for(CPU_FAN_HEADER, 40), 40);
        assert_eq!(floor_for("gpu_fan1", 20), GPU_FAN_MIN_PERCENT);
        assert_eq!(floor_for("gpu_fan2", 45), 45);
        assert!(validate_mode("gpu_fan1", &FanMode::Fixed { percent: 29 }, 20).is_err());
        // A floor below the default (e.g. from an imported file) is ignored.
        assert_eq!(floor_for("sys_fan1", 0), DEFAULT_FLOOR_PERCENT);
        assert!(validate_mode("sys_fan1", &FanMode::Fixed { percent: 5 }, 0).is_err());
        assert_eq!(runtime_floor("sys_fan1", 0, None), DEFAULT_FLOOR_PERCENT);
        assert!(validate_mode("sys_fan1", &FanMode::Fixed { percent: 19 }, 20).is_err());
        assert!(validate_mode("sys_fan1", &FanMode::Fixed { percent: 20 }, 20).is_ok());
        assert!(validate_mode(CPU_FAN_HEADER, &FanMode::Fixed { percent: 29 }, 20).is_err());
        assert_eq!(runtime_floor("sys_fan1", 20, Some(55.0)), 35);
        assert_eq!(runtime_floor(CPU_FAN_HEADER, 20, Some(50.0)), 50);
        assert_eq!(runtime_floor(CPU_FAN_HEADER, 20, Some(49.9)), 30);
    }

    #[test]
    fn curve_rules() {
        let c = |pts: &[(f32, u8)]| FanMode::Curve {
            source: TempSource::Cpu,
            points: pts.iter().map(|&(t, p)| CurvePoint { temp_c: t, percent: p }).collect(),
        };
        assert!(validate_mode("sys_fan1", &c(&[(30.0, 30)]), 20).is_err());
        assert!(validate_mode("sys_fan1", &c(&[(30.0, 30), (70.0, 100)]), 20).is_ok());
        assert!(validate_mode("sys_fan1", &c(&[(70.0, 30), (30.0, 100)]), 20).is_err());
        assert!(validate_mode("sys_fan1", &c(&[(30.0, 10), (70.0, 100)]), 20).is_err());
    }
}
