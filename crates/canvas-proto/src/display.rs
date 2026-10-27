//! Numeric displays: a pump head or panel that shows one number (the GAMDIAS CHIONE Digital Display
//! shows four digits, one decimal point and a °C/°F mark). They take a value, not LED colors, so they
//! have their own adapter trait (`NumericDisplayAdapter` in `traits`) instead of `DeviceAdapter`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::device::DeviceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DisplayUnit {
    Celsius,
    Fahrenheit,
}

/// What the display's own mode marks say the number is. The CHIONE has a temperature mode and a
/// fan-speed mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    Temperature,
    FanRpm,
}

/// One value to show. `value: None` blanks the digits. The adapter rounds to `decimals` and
/// refuses a value that does not fit its digits (`AdapterError::Protocol`), never truncates it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DisplayValue {
    pub value: Option<f32>,
    pub decimals: u8,
    pub unit: DisplayUnit,
    pub mode: DisplayMode,
    pub blink: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DisplayInfo {
    pub id: DeviceId,
    pub name: String,
    /// Digits the display has (the CHIONE: 4).
    pub digits: u8,
    pub max_decimals: u8,
    pub modes: Vec<DisplayMode>,
    pub connected: bool,
}

/// Where a display's number comes from. Sensor and header ids are the ones `canvas-svc` reports
/// (`board_temp0..3`, `cpu_fan`, `gpu_fan1`, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisplaySource {
    CpuTemp,
    GpuTemp,
    BoardTemp { sensor: String },
    FanRpm { header: String },
    Fixed { value: f32 },
    /// The last value written to the display's API route; blank until one arrives.
    Api,
}

/// Saved per display. `blink_above` blinks the display while the shown temperature is at or above
/// it (in the display's unit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DisplayConfig {
    pub device: DeviceId,
    pub source: DisplaySource,
    pub unit: DisplayUnit,
    pub decimals: u8,
    pub blink_above: Option<f32>,
}
