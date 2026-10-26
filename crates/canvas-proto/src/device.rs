//! Devices, zones and adapter capabilities as the daemon, engine and editor see them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Stable device fingerprint: `<adapter>:<serial>`, or `<adapter>:<location>` when the
/// device reports no serial. Zone sizes in `zones.json` are keyed by this.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(transparent)]
pub struct DeviceId(pub String);

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Motherboard,
    Dram,
    Keyboard,
    Mouse,
    Microphone,
    Headset,
    LedStrip,
    Gpu,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ZoneKind {
    Single,
    Linear,
    /// `map[row * width + col]` is the LED index inside the zone, or `None` for a gap.
    Matrix {
        width: u32,
        height: u32,
        map: Vec<Option<u32>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ZoneInfo {
    pub index: u32,
    pub name: String,
    pub kind: ZoneKind,
    pub led_count: u32,
    pub min_leds: u32,
    pub max_leds: u32,
    /// True when `led_count` can be changed (`set_zone_size`), e.g. MSI JARGB headers.
    pub resizable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub vendor: String,
    pub kind: DeviceKind,
    /// Adapter name, e.g. `"openrgb"`.
    pub adapter: String,
    /// Adapter-local index (OpenRGB controller index). Changes across reconnects.
    pub adapter_index: u32,
    pub serial: String,
    pub location: String,
    pub zones: Vec<ZoneInfo>,
    pub connected: bool,
}

impl DeviceInfo {
    /// Total LEDs across zones; frames for this device are this long, in zone order.
    pub fn led_count(&self) -> u32 {
        self.zones.iter().map(|z| z.led_count).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AdapterCapabilities {
    pub resizable_zones: bool,
    pub identify: bool,
    pub hardware_brightness: bool,
    pub needs_elevation: bool,
}
