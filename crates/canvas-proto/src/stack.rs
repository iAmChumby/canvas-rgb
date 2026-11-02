//! Effect stacks: an effect built from layers, each a picture with an area and a blend.
//!
//! A stack is saved whole in `stacks.json` (see [`StackFile`]) and runs as one `Effect` the
//! daemon composes in Rust (see the daemon's `stack` module). The types here are what crosses
//! the API: the editor reads and writes them, and the CLI activates the id `stack:<name>`.

use crate::device::DeviceId;
use crate::props::Props;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The id prefix of a stack: a stack's id is `stack:` + its name.
pub const STACK_ID_PREFIX: &str = "stack:";
/// The file a stack is saved in, beside `layouts.json`.
pub const STACKS_FILE: &str = "stacks.json";
/// The `schema_version` of `stacks.json`.
pub const STACKS_SCHEMA: u32 = 1;
/// The most layers one stack may hold.
pub const STACK_MAX_LAYERS: usize = 8;
/// The most HTML layers one stack may hold: each is a QuickJS runtime and the slowest layer.
pub const STACK_HTML_MAX: usize = 2;
/// The longest a stack's name may be, before the id's prefix.
pub const STACK_NAME_MAX: usize = 60;
/// The longest a layer's own name may be.
pub const LAYER_NAME_MAX: usize = 40;

/// One stack: its name in its id, and its layers, bottom first.
///
/// `layers[0]` is painted first, so later layers sit over it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Stack {
    /// `stack:` + the name. The name is the id without the prefix, so there is one string.
    pub id: String,
    /// Bottom first: `layers[0]` is painted first.
    #[serde(default)]
    pub layers: Vec<StackLayer>,
}

impl Stack {
    /// The name: the id without [`STACK_ID_PREFIX`]. An id that does not carry the prefix is
    /// its own name, so a file written by hand still names something.
    pub fn name(&self) -> &str {
        self.id.strip_prefix(STACK_ID_PREFIX).unwrap_or(&self.id)
    }
}

/// One layer: a picture, where it is drawn, and how it sits over what is under it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StackLayer {
    /// `l` + 1–2 digits (D95.1.7). The merged property key is `{id}.{key}`.
    pub id: String,
    /// The layer's name, as the builder shows it.
    #[serde(default)]
    pub name: String,
    /// An effect id: a built-in, an `html:` one, or a stock one. Never `stack:`.
    pub effect: String,
    /// The values the effect runs with; missing keys take its schema's defaults.
    #[serde(default)]
    pub props: Props,
    /// Where the layer is painted.
    #[serde(default)]
    pub area: LayerArea,
    /// How it sits over what is under it.
    #[serde(default)]
    pub blend: BlendMode,
    /// 0..=100. 0 draws nothing.
    #[serde(default = "full_opacity")]
    pub opacity: u8,
    #[serde(default = "default_true")]
    pub visible: bool,
}

fn full_opacity() -> u8 {
    100
}

fn default_true() -> bool {
    true
}

/// Where a layer's picture is drawn, in canvas coordinates (320 × 200).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LayerArea {
    /// The whole canvas.
    #[default]
    Canvas,
    /// One rectangle per placement of every zone of the device.
    Device { device: DeviceId },
    /// One rectangle per placement of that zone.
    Zone { device: DeviceId, zone: u32 },
    /// The four numbers, exactly as drawn.
    Rect { x: f32, y: f32, width: f32, height: f32 },
}

/// How a layer sits over what is under it. The names are Photoshop's; the arithmetic is the
/// compositor's own (see the daemon's `stack::blend_pixel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Multiply,
    Screen,
}

impl BlendMode {
    /// The four modes' names, as the API writes them.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Add => "add",
            Self::Multiply => "multiply",
            Self::Screen => "screen",
        }
    }
}

/// `stacks.json`: every saved stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StackFile {
    pub schema_version: u32,
    #[serde(default)]
    pub stacks: Vec<Stack>,
}

impl Default for StackFile {
    fn default() -> Self {
        Self { schema_version: STACKS_SCHEMA, stacks: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the spec draws: no field of a layer is required, and the id is the name.
    const SPEC: &str = r#"{
        "schema_version": 1,
        "stacks": [
            { "id": "stack:Sunset glow",
              "layers": [
                { "id": "l2", "name": "Heat", "effect": "fire",
                  "area": { "kind": "device", "device": "royuan:3151:4015" },
                  "blend": "add", "opacity": 70 },
                { "id": "l1", "name": "Sunset", "effect": "gradient",
                  "area": { "kind": "canvas" }, "blend": "normal", "opacity": 100 }
              ] }
        ]
    }"#;

    #[test]
    fn the_spec_example_round_trips() {
        let file: StackFile = serde_json::from_str(SPEC).expect("the spec's example reads");
        assert_eq!(file.schema_version, STACKS_SCHEMA);
        let stack = &file.stacks[0];
        assert_eq!(stack.id, "stack:Sunset glow");
        assert_eq!(stack.name(), "Sunset glow");
        assert_eq!(stack.layers[0].blend, BlendMode::Add);
        assert!(stack.layers[0].visible, "a layer is visible unless it says otherwise");
        assert_eq!(
            stack.layers[0].area,
            LayerArea::Device { device: DeviceId("royuan:3151:4015".into()) }
        );
        let again = serde_json::to_string(&file).expect("it writes");
        let back: StackFile = serde_json::from_str(&again).expect("and reads again");
        assert_eq!(back, file);
    }

    #[test]
    fn a_rect_and_a_zone_keep_their_numbers() {
        let area: LayerArea = serde_json::from_str(
            r#"{"kind":"rect","x":16.0,"y":150.0,"width":110.0,"height":46.0}"#,
        )
        .expect("a rect reads");
        assert_eq!(area, LayerArea::Rect { x: 16.0, y: 150.0, width: 110.0, height: 46.0 });
        let zone: LayerArea = serde_json::from_str(r#"{"kind":"zone","device":"d","zone":2}"#)
            .expect("a zone reads");
        assert_eq!(zone, LayerArea::Zone { device: DeviceId("d".into()), zone: 2 });
    }

    #[test]
    fn a_bare_layer_takes_canvas_normal_full_and_visible() {
        let layer: StackLayer = serde_json::from_str(r#"{"id":"l1","effect":"solid"}"#).unwrap();
        assert_eq!(layer.area, LayerArea::Canvas);
        assert_eq!(layer.blend, BlendMode::Normal);
        assert_eq!(layer.opacity, 100);
        assert!(layer.visible);
        assert!(layer.props.is_empty());
    }

    #[test]
    fn a_file_with_no_stacks_reads_as_empty() {
        let file: StackFile = serde_json::from_str(r#"{"schema_version":1}"#).unwrap();
        assert!(file.stacks.is_empty());
    }

    #[test]
    fn every_blend_name_is_the_api_name() {
        for (mode, name) in [
            (BlendMode::Normal, "normal"),
            (BlendMode::Add, "add"),
            (BlendMode::Multiply, "multiply"),
            (BlendMode::Screen, "screen"),
        ] {
            assert_eq!(mode.as_str(), name);
            assert_eq!(serde_json::to_string(&mode).unwrap(), format!("\"{name}\""));
        }
    }
}
