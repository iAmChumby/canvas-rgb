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
}

impl PropertySchema {
    pub fn defaults(&self) -> Props {
        self.properties
            .iter()
            .map(|p| (p.key.clone(), p.kind.default_value()))
            .collect()
    }
}
