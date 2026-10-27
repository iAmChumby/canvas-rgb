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
/// GPU fan headers are `gpu_fan1`, `gpu_fan2`, ... (NVML fan index + 1). Added on Luke's
/// request (lighting session 2), after an NVML write test on his RTX 5070 Ti.
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

#[cfg(test)]
mod tests {
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
