//! Layouts: zones placed in canvas space, plus per-layout sampling and render scale.

use crate::device::DeviceId;
use crate::props::Props;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Logical canvas size (SignalRGB's), so the Poison port shares its coordinates.
pub const CANVAS_WIDTH: u32 = 320;
pub const CANVAS_HEIGHT: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SamplingMode {
    Nearest,
    Bilinear,
    #[default]
    AreaAverage,
}

/// Render scale as a percentage of the logical canvas. Only 25, 50 and 100 are valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(try_from = "u8", into = "u8")]
#[ts(type = "25 | 50 | 100")]
pub enum RenderScale {
    P25,
    P50,
    #[default]
    P100,
}

impl RenderScale {
    pub fn percent(self) -> u8 {
        match self {
            Self::P25 => 25,
            Self::P50 => 50,
            Self::P100 => 100,
        }
    }
    pub fn factor(self) -> f32 {
        self.percent() as f32 / 100.0
    }
}

impl TryFrom<u8> for RenderScale {
    type Error = String;
    fn try_from(v: u8) -> Result<Self, String> {
        match v {
            25 => Ok(Self::P25),
            50 => Ok(Self::P50),
            100 => Ok(Self::P100),
            _ => Err(format!("render scale must be 25, 50 or 100, got {v}")),
        }
    }
}

impl From<RenderScale> for u8 {
    fn from(s: RenderScale) -> u8 {
        s.percent()
    }
}

/// One zone of one device, placed as a rectangle in logical canvas coordinates
/// (0..320 x 0..200). Linear zones run left to right along the rect's long axis before
/// rotation; matrix zones fill the rect as their grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlacedZone {
    pub device_id: DeviceId,
    pub zone_index: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Degrees clockwise around the rect's center.
    pub rotation_deg: f32,
    pub reversed: bool,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActiveEffect {
    pub id: String,
    pub props: Props,
}

/// One effect per layout in v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Layout {
    pub id: String,
    pub name: String,
    pub render_scale: RenderScale,
    pub sampling: SamplingMode,
    pub zones: Vec<PlacedZone>,
    pub effect: Option<ActiveEffect>,
}
