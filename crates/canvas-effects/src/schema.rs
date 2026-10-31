//! A small builder for [`PropertySchema`], so each effect declares its properties once
//! and reads clearly.

use canvas_proto::props::{Preset, ThumbnailRecipe};
use canvas_proto::{InputNeeds, PropKind, PropValue, PropertyDef, PropertySchema, Rgb8};
use std::collections::BTreeMap;

/// Builds a [`PropertySchema`] from a list of properties and presets.
pub struct SchemaBuilder {
    id: String,
    name: String,
    description: String,
    needs: InputNeeds,
    properties: Vec<PropertyDef>,
    presets: Vec<Preset>,
    thumbnail: Option<ThumbnailRecipe>,
}

impl SchemaBuilder {
    pub fn new(id: &str, name: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            needs: InputNeeds::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    /// Marks the effect as needing the screen capture.
    pub fn needs_screen(mut self) -> Self {
        self.needs.screen = true;
        self
    }

    /// Marks the effect as needing the audio spectrum.
    pub fn needs_audio(mut self) -> Self {
        self.needs.audio = true;
        self
    }

    /// Sets how the effect's library tile is rendered.
    pub fn thumbnail(mut self, recipe: ThumbnailRecipe) -> Self {
        self.thumbnail = Some(recipe);
        self
    }

    pub fn color(mut self, key: &str, label: &str, default: &str) -> Self {
        let default: Rgb8 = default.parse().unwrap_or(Rgb8::BLACK);
        self.properties.push(PropertyDef {
            key: key.to_string(),
            label: label.to_string(),
            kind: PropKind::Color { default },
        });
        self
    }

    pub fn number(mut self, key: &str, label: &str, min: f64, max: f64, step: f64, default: f64) -> Self {
        self.properties.push(PropertyDef {
            key: key.to_string(),
            label: label.to_string(),
            kind: PropKind::Number { min, max, step, default },
        });
        self
    }

    pub fn boolean(mut self, key: &str, label: &str, default: bool) -> Self {
        self.properties.push(PropertyDef {
            key: key.to_string(),
            label: label.to_string(),
            kind: PropKind::Bool { default },
        });
        self
    }

    pub fn choice(mut self, key: &str, label: &str, options: &[&str], default: &str) -> Self {
        self.properties.push(PropertyDef {
            key: key.to_string(),
            label: label.to_string(),
            kind: PropKind::Choice {
                options: options.iter().map(|s| s.to_string()).collect(),
                default: default.to_string(),
            },
        });
        self
    }

    pub fn hue(mut self, key: &str, label: &str, default: f64) -> Self {
        self.properties.push(PropertyDef {
            key: key.to_string(),
            label: label.to_string(),
            kind: PropKind::Hue { default },
        });
        self
    }

    /// Adds a preset with the given properties. Any property the preset does not
    /// mention takes the schema's default, so every preset covers every property.
    pub fn preset(mut self, name: &str, props: Vec<(&str, PropKind)>) -> Self {
        let mut values: BTreeMap<String, PropValue> = props
            .into_iter()
            .map(|(k, kind)| (k.to_string(), kind.default_value()))
            .collect();
        for def in &self.properties {
            values
                .entry(def.key.clone())
                .or_insert_with(|| def.kind.default_value());
        }
        self.presets.push(Preset { name: name.to_string(), props: values });
        self
    }

    /// Adds a preset built from literal values. Any property the preset does not
    /// mention takes the schema's default.
    pub fn preset_values(mut self, name: &str, props: Vec<(&str, PropValue)>) -> Self {
        let mut values: BTreeMap<String, PropValue> =
            props.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        for def in &self.properties {
            values
                .entry(def.key.clone())
                .or_insert_with(|| def.kind.default_value());
        }
        self.presets.push(Preset { name: name.to_string(), props: values });
        self
    }

    pub fn build(self) -> PropertySchema {
        PropertySchema {
            id: self.id,
            name: self.name,
            description: self.description,
            needs: self.needs,
            properties: self.properties,
            presets: self.presets,
            thumbnail: self.thumbnail,
        }
    }
}

/// A thumbnail recipe for an effect: seconds of warm-up, seconds the tile's loop covers,
/// frames crossfaded from the loop's end into its start, and the showcase props laid over
/// the schema defaults. See [`ThumbnailRecipe`].
pub fn recipe(
    warm_up_s: f64,
    loop_s: f64,
    crossfade_frames: u32,
    props: Vec<(&str, PropValue)>,
) -> ThumbnailRecipe {
    ThumbnailRecipe {
        warm_up_s,
        loop_s,
        crossfade_frames,
        props: props.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        dark_on_purpose: None,
    }
}

/// A number value for a recipe's showcase props.
pub fn num(value: f64) -> PropValue {
    PropValue::Number(value)
}
