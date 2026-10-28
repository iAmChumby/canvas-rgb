//! Layouts: zones placed in canvas space, plus per-layout sampling and render scale.

use crate::device::DeviceId;
use crate::props::Props;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Logical canvas size (SignalRGB's), so an effect ported from it shares its coordinates.
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
/// Default 50%, Artemis's default: a quarter of the pixels, and LEDs are area-averaged over
/// cells several render pixels wide, so the lights barely change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(try_from = "u8", into = "u8")]
#[ts(type = "25 | 50 | 100")]
pub enum RenderScale {
    P25,
    #[default]
    P50,
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
    /// How the zone's LEDs sit inside the rect. Layouts saved before shapes existed load as
    /// `Auto`, which keeps their old placement.
    #[serde(default)]
    pub shape: ZoneShape,
    /// The owner's own name for this zone ("Front fans"), shown before the hardware's name
    /// ("JARGB 1"). `None` shows the hardware name alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub label: Option<String>,
}

/// Where a zone's LEDs sit inside its placed rect (before rotation). The rect is the shape's
/// bounding box, so resizing a zone resizes its shape, and each LED samples the effect where it sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ZoneShape {
    /// Follows the device's zone kind: single = one point, linear = a strip along the long
    /// axis, matrix = its grid map.
    #[default]
    Auto,
    /// LEDs evenly spaced along the rect's long axis.
    Strip,
    /// LEDs evenly spaced around the ellipse inscribed in the rect, clockwise from the top
    /// (a fan ring).
    Ring,
    /// LEDs evenly spaced around the rect's edge, clockwise from the top-left corner (a case
    /// or desk outline).
    Outline,
    /// Every LED samples the whole rect (one blob of light, e.g. a mouse logo or a mic).
    Point,
    /// LEDs fill a grid of `columns` row by row, top-left first.
    Grid { columns: u32 },
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
    /// The PC's form factor, drawn behind the zones as a generic case with standard slots
    /// (fans, RAM, GPU, edges) plus a desk area for peripherals. Layouts saved before this
    /// existed load as `None`: a free layout with no case drawn, so nothing moves.
    #[serde(default)]
    pub case_form: CaseForm,
}

/// A generic case shape for the desk. It is a drawing and a set of snap slots only: the
/// effect is still sampled where each zone sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CaseForm {
    /// No case drawn: zones sit freely on the desk.
    #[default]
    None,
    MidTower,
    FullTower,
    /// Small form factor (mini-ITX).
    SmallFormFactor,
    /// An open-frame bench case.
    OpenFrame,
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD_ZONE: &str = r#"{"device_id":"openrgb:board","zone_index":1,"x":4.0,"y":4.0,"width":120.0,
        "height":10.0,"rotation_deg":0.0,"reversed":false,"visible":true}"#;

    #[test]
    fn a_zone_saved_before_shapes_and_labels_loads_with_auto_and_no_label() {
        let zone: PlacedZone = serde_json::from_str(OLD_ZONE).expect("an old zone loads");
        assert_eq!(zone.shape, ZoneShape::Auto);
        assert_eq!(zone.label, None);
    }

    #[test]
    fn a_label_round_trips_and_a_missing_one_is_not_written() {
        let mut zone: PlacedZone = serde_json::from_str(OLD_ZONE).unwrap();
        let bare = serde_json::to_string(&zone).unwrap();
        assert!(!bare.contains("label"), "no label, nothing written: {bare}");

        zone.label = Some("Front fans".to_string());
        zone.shape = ZoneShape::Ring;
        let json = serde_json::to_string(&zone).unwrap();
        assert!(json.contains(r#""label":"Front fans""#));
        let back: PlacedZone = serde_json::from_str(&json).unwrap();
        assert_eq!(back.label.as_deref(), Some("Front fans"));
        assert_eq!(back.shape, ZoneShape::Ring);
    }
}
