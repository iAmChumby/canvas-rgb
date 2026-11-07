//! The layer names a stack's schema carries, after
//! `app/web/src/lib/stackLayers.ts`.
//!
//! The daemon labels each property `{layer} · {property}` (D95.4.3), so a
//! layer's name is everything before the first separator. A layer's name
//! cannot hold one (the daemon's name rule allows letters, digits, spaces,
//! `-`, `_` and `.`), but a user effect's property label can, so the first
//! separator is the boundary.

#![allow(dead_code)]

use canvas_proto::props::PropertySchema;

/// The layer names the schema carries, in the order the properties name them.
pub fn stack_layer_names(schema: &PropertySchema) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for def in &schema.properties {
        let name = match def.label.find(" · ") {
            Some(at) => &def.label[..at],
            None => def.label.as_str(),
        };
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    if names.is_empty() {
        // A stack whose layers have no properties still names them in its
        // description.
        if let Some(after) = schema.description.split(": ").nth(1) {
            return after.split(", ").map(String::from).collect();
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::props::{PropKind, PropertyDef};

    fn schema(labels: &[(&str, &str)], description: &str) -> PropertySchema {
        PropertySchema {
            id: "stack:s".into(),
            name: "S".into(),
            description: description.into(),
            needs: Default::default(),
            properties: labels
                .iter()
                .map(|(key, label)| PropertyDef {
                    key: (*key).into(),
                    label: (*label).into(),
                    kind: PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 1.0 },
                })
                .collect(),
            presets: vec![],
            thumbnail: None,
        }
    }

    #[test]
    fn a_layer_name_is_everything_before_the_first_separator() {
        let s = schema(&[("a", "Rain · Speed"), ("b", "Fire · Ink")], "two layers");
        assert_eq!(stack_layer_names(&s), vec!["Rain".to_string(), "Fire".to_string()]);
    }

    #[test]
    fn a_name_that_repeats_is_only_listed_once() {
        let s = schema(&[("a", "Rain · Speed"), ("b", "Rain · Ink")], "two layers");
        assert_eq!(stack_layer_names(&s), vec!["Rain".to_string()]);
    }

    #[test]
    fn a_label_with_no_separator_is_the_whole_name() {
        let s = schema(&[("a", "Rain")], "one layer");
        assert_eq!(stack_layer_names(&s), vec!["Rain".to_string()]);
    }

    #[test]
    fn a_stack_with_no_properties_names_its_layers_in_the_description() {
        // React: `schema.description.split(": ")[1]` then `split(", ")`.
        let s = schema(&[], "3 layers: Gradient, Heat, Sparkles");
        assert_eq!(
            stack_layer_names(&s),
            vec!["Gradient".to_string(), "Heat".to_string(), "Sparkles".to_string()]
        );
    }

    #[test]
    fn a_stack_with_neither_has_no_names() {
        let s = schema(&[], "No layers yet");
        assert!(stack_layer_names(&s).is_empty());
    }
}
