//! Color Cycle: a hue that sweeps around the whole spectrum.

use crate::color::hsv_to_rgb;
use crate::props_util as p;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, PropertySchema, Props};

/// The Color Cycle effect.
pub struct ColorCycle {
    schema: PropertySchema,
    speed: f32,
    saturation: f32,
    /// The color's brightness, 0..1.
    value: f32,
    /// Hue offset in turns.
    offset: f32,
}

impl ColorCycle {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(
            "color_cycle",
            "Color Cycle",
            "A hue that sweeps around the whole spectrum.",
        )
        .number("speed", "Speed", 0.0, 100.0, 1.0, 20.0)
        .number("saturation", "Saturation", 0.0, 100.0, 1.0, 100.0)
        .number("value", "Brightness", 0.0, 100.0, 1.0, 100.0)
        .preset(
            "Fast rainbow",
            vec![("speed", number_kind(60.0))],
        )
        .preset(
            "Deep blues",
            vec![
                ("speed", number_kind(10.0)),
                ("saturation", number_kind(90.0)),
                ("value", number_kind(70.0)),
            ],
        )
        .preset(
            "Slow purple",
            vec![("speed", number_kind(8.0))],
        )
        // Speed 10 turns the hue once in exactly 2 s, the length of the tile, so the loop closes.
        .thumbnail(crate::schema::recipe(1.0, 2.0, 0, vec![("speed", crate::schema::num(10.0))]))
        .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self { schema, speed: 0.0, saturation: 1.0, offset: 0.0, value: 1.0 };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for ColorCycle {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(ColorCycle::new())
}

impl Effect for ColorCycle {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.speed = p::number(&self.schema, props, "speed") as f32;
        self.saturation = p::number(&self.schema, props, "saturation") as f32 / 100.0;
        self.value = p::number(&self.schema, props, "value") as f32 / 100.0;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.offset = (self.offset + self.speed * dt * 0.05).rem_euclid(1.0);
        let c = hsv_to_rgb(self.offset, self.saturation, self.value);
        canvas.pixels.fill(c);
    }
}

fn number_kind(n: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: n }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::Inputs;

    #[test]
    fn the_hue_advances() {
        let mut e = ColorCycle::new();
        let mut canvas = Canvas::new(4, 4);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let first = canvas.pixels[0];
        e.update(0.5, &mut canvas, &Inputs::default());
        assert_ne!(first, canvas.pixels[0]);
    }

    #[test]
    fn output_stays_in_range() {
        let mut e = ColorCycle::new();
        let mut canvas = Canvas::new(8, 8);
        for _ in 0..120 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        for p in canvas.pixels.iter() {
            for v in p {
                assert!((0.0..=1.0).contains(v), "{p:?}");
            }
        }
    }
}
