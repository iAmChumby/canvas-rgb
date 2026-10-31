//! Effect property schema. Each effect declares its properties once; the editor renders
//! controls from this with no per-effect UI code.

use crate::color::Rgb8;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum PropValue {
    Color(Rgb8),
    Number(f64),
    Bool(bool),
    Choice(String),
    /// Degrees, 0..360.
    Hue(f64),
}

/// Property values by property key.
pub type Props = BTreeMap<String, PropValue>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum PropKind {
    Color { default: Rgb8 },
    Number { min: f64, max: f64, step: f64, default: f64 },
    Bool { default: bool },
    Choice { options: Vec<String>, default: String },
    Hue { default: f64 },
}

impl PropKind {
    pub fn default_value(&self) -> PropValue {
        match self {
            Self::Color { default } => PropValue::Color(*default),
            Self::Number { default, .. } => PropValue::Number(*default),
            Self::Bool { default } => PropValue::Bool(*default),
            Self::Choice { default, .. } => PropValue::Choice(default.clone()),
            Self::Hue { default } => PropValue::Hue(*default),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PropertyDef {
    pub key: String,
    pub label: String,
    pub kind: PropKind,
}

/// Inputs an effect consumes. The daemon starts capture only while the active effect
/// needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InputNeeds {
    pub screen: bool,
    pub audio: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Preset {
    pub name: String,
    pub props: Props,
}

/// How an effect's library tile is rendered (`GET /api/v1/effects/{id}/thumbnail`). An effect
/// that carries none is rendered with the renderer's own defaults.
///
/// The recipe is for the tile only: it never changes how the effect runs on the lights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ThumbnailRecipe {
    /// Seconds the effect runs before the first recorded frame, so trails and build-ups
    /// have content. The renderer default is 1.
    #[serde(default = "default_warm_up_s")]
    pub warm_up_s: f64,
    /// Seconds the tile's loop lasts, matched to the effect's period where it has one.
    /// The renderer default is 2.
    #[serde(default = "default_loop_s")]
    pub loop_s: f64,
    /// Frames at the end of the loop that are crossfaded into the first frames, for an
    /// effect with no period (random rain). 0 is no crossfade.
    #[serde(default)]
    pub crossfade_frames: u32,
    /// The showcase props for the tile, laid over the schema defaults.
    #[serde(default)]
    pub props: Props,
    /// Set when the effect is dark on purpose: the reason, which exempts the tile from the
    /// lit-share bar (never from the rows, columns or seam bars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dark_on_purpose: Option<String>,
}

fn default_warm_up_s() -> f64 {
    1.0
}

fn default_loop_s() -> f64 {
    2.0
}

impl Default for ThumbnailRecipe {
    /// The renderer's defaults: one second of warm-up and a two second loop.
    fn default() -> Self {
        Self {
            warm_up_s: default_warm_up_s(),
            loop_s: default_loop_s(),
            crossfade_frames: 0,
            props: Props::new(),
            dark_on_purpose: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PropertySchema {
    /// Stable id: one of [`crate::EFFECT_IDS`], or `html:` + the file stem for a user effect.
    pub id: String,
    pub name: String,
    pub description: String,
    pub needs: InputNeeds,
    pub properties: Vec<PropertyDef>,
    pub presets: Vec<Preset>,
    /// How the library tile is rendered. `None` is the renderer default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thumbnail: Option<ThumbnailRecipe>,
}

impl PropertySchema {
    pub fn defaults(&self) -> Props {
        self.properties
            .iter()
            .map(|p| (p.key.clone(), p.kind.default_value()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A schema as an older client or file wrote it, with no `thumbnail` field.
    const OLD: &str = r#"{"id":"solid","name":"Solid","description":"","needs":{"screen":false,"audio":false},"properties":[],"presets":[]}"#;

    #[test]
    fn a_schema_without_a_thumbnail_recipe_still_reads_and_writes_without_one() {
        let schema: PropertySchema = serde_json::from_str(OLD).expect("an old schema reads");
        assert_eq!(schema.thumbnail, None);
        let again = serde_json::to_string(&schema).expect("writes");
        assert!(!again.contains("thumbnail"), "a schema with no recipe writes none: {again}");
    }

    #[test]
    fn a_recipe_fills_what_it_leaves_out_with_the_renderer_defaults() {
        let json = OLD.replace("\"presets\":[]", "\"presets\":[],\"thumbnail\":{\"loop_s\":4.0}");
        let schema: PropertySchema = serde_json::from_str(&json).expect("a partial recipe reads");
        let recipe = schema.thumbnail.expect("a recipe");
        assert_eq!(recipe, ThumbnailRecipe { loop_s: 4.0, ..ThumbnailRecipe::default() });
        assert_eq!((recipe.warm_up_s, recipe.crossfade_frames), (1.0, 0));
    }

    #[test]
    fn a_recipe_round_trips() {
        let recipe = ThumbnailRecipe {
            warm_up_s: 3.0,
            loop_s: 4.0,
            crossfade_frames: 6,
            props: [("speed".to_string(), PropValue::Number(90.0))].into_iter().collect(),
            dark_on_purpose: Some("a spark on black".into()),
        };
        let json = serde_json::to_string(&recipe).expect("writes");
        assert_eq!(serde_json::from_str::<ThumbnailRecipe>(&json).expect("reads"), recipe);
    }
}
