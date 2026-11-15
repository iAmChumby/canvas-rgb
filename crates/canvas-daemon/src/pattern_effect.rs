//! The effect that is a pattern: it answers [`Effect::as_pattern`] and draws nothing.
//!
//! A pattern has no picture. The engine reads its colours by LED (the canvas path is skipped for
//! the group) and never calls `update`, so this type exists to carry a saved pattern into a group
//! and to give every route and the library its schema: an id, a name, and a description read from
//! `built_on`. It has no properties: a pattern exposes no settings to its group in 0.16.0.

use canvas_proto::pattern::{BuiltOn, Pattern};
use canvas_proto::props::{InputNeeds, Props, PropertySchema};
use canvas_proto::scene::Arrangement;
use canvas_proto::traits::{Canvas, Effect, Inputs};
use std::sync::Arc;

/// A saved pattern, as the engine's effect.
pub struct PatternEffect {
    pattern: Arc<Pattern>,
}

impl PatternEffect {
    pub fn new(pattern: Pattern) -> Self {
        Self { pattern: Arc::new(pattern) }
    }

    /// The pattern this effect carries.
    pub fn pattern(&self) -> &Pattern {
        &self.pattern
    }
}

/// The schema every route and the library read for a pattern: its id, its name, what it was built
/// on, and no properties.
pub fn schema(pattern: &Pattern) -> PropertySchema {
    PropertySchema {
        id: pattern.id.clone(),
        name: pattern.name().to_string(),
        description: format!("Pattern, {}", kind_words(&pattern.built_on)),
        // A pattern reads no screen and no audio, and asks for nothing else.
        needs: InputNeeds::default(),
        properties: Vec::new(),
        // A pattern has no presets in 0.16.0: it has no settings to save.
        presets: Vec::new(),
        // Its tile is its dots, drawn by the page; it never goes through the tile renderer.
        thumbnail: None,
    }
}

/// The kind of device a pattern was built on, in the spec's own words: `ring of 12`,
/// `strip of 60`, `grid 5 by 4`, `single LED`.
pub fn kind_words(built_on: &BuiltOn) -> String {
    match built_on.arrangement {
        Arrangement::Ring => format!("ring of {}", built_on.led_count),
        Arrangement::Strip => format!("strip of {}", built_on.led_count),
        Arrangement::Single => "single LED".to_string(),
        Arrangement::Grid => match built_on.grid {
            Some(grid) if grid.width > 0 => format!("grid {} by {}", grid.width, grid.height),
            // A grid whose size the file does not carry: its LED count is what is known.
            _ => format!("grid of {}", built_on.led_count),
        },
    }
}

impl Effect for PatternEffect {
    fn schema(&self) -> PropertySchema {
        schema(&self.pattern)
    }

    fn init(&mut self, _props: &Props) {}

    /// A pattern draws nothing: the engine copies its fitted colours instead.
    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {}

    fn as_pattern(&self) -> Option<&Pattern> {
        Some(self.pattern.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::color::Rgb8;
    use canvas_proto::pattern::{Frame, GridSize, Motion};

    fn pattern(name: &str, built_on: BuiltOn) -> Pattern {
        Pattern {
            id: format!("pattern:{name}"),
            built_on,
            frames: vec![Frame {
                colors: vec![Rgb8::BLACK; 12],
                hold: 0,
                extra: serde_json::Map::new(),
            }],
            motion: Motion::none(),
            extra: serde_json::Map::new(),
        }
    }

    fn on(leds: u32, arrangement: Arrangement) -> BuiltOn {
        BuiltOn { led_count: leds, arrangement, grid: None }
    }

    #[test]
    fn the_schema_names_the_kind_of_device_it_was_built_on() {
        let ring = pattern("Intake", on(12, Arrangement::Ring));
        let schema = schema(&ring);
        assert_eq!(schema.id, "pattern:Intake");
        assert_eq!(schema.name, "Intake");
        assert_eq!(schema.description, "Pattern, ring of 12");
        assert!(schema.properties.is_empty(), "a pattern has no settings");
        assert!(schema.presets.is_empty());
        assert_eq!(schema.needs, InputNeeds::default(), "a pattern reads no screen and no audio");

        assert_eq!(kind_words(&on(60, Arrangement::Strip)), "strip of 60");
        assert_eq!(kind_words(&on(1, Arrangement::Single)), "single LED");
        assert_eq!(
            kind_words(&BuiltOn {
                led_count: 20,
                arrangement: Arrangement::Grid,
                grid: Some(GridSize { width: 5, height: 4 }),
            }),
            "grid 5 by 4"
        );
        // A grid the file does not size falls back to its count rather than saying nothing.
        assert_eq!(kind_words(&on(20, Arrangement::Grid)), "grid of 20");
    }

    #[test]
    fn a_pattern_effect_answers_as_pattern_and_draws_nothing() {
        let ring = pattern("Intake", on(12, Arrangement::Ring));
        let mut effect = PatternEffect::new(ring.clone());
        assert_eq!(effect.as_pattern(), Some(&ring));
        let mut canvas = Canvas::new(8, 8);
        effect.update(0.016, &mut canvas, &Inputs::default());
        assert!(
            canvas.pixels.iter().all(|p| *p == [0.0, 0.0, 0.0]),
            "a pattern paints no canvas"
        );
        assert_eq!(effect.schema().id, "pattern:Intake");
        assert_eq!(effect.pattern(), &ring);
    }
}
