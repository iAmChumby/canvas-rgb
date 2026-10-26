//! Solid: one flat color across the whole canvas.

use crate::color::to_linear;
use crate::props_util as p;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, PropertySchema, Props};

/// The Solid effect.
pub struct Solid {
    schema: PropertySchema,
    color: [f32; 3],
}

impl Solid {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new("solid", "Solid", "One flat color across the whole canvas.")
            .color("color", "Color", "#6000fc")
            .preset(
                "Purple",
                vec![("color", color_kind("#6000fc"))],
            )
            .preset(
                "Amber",
                vec![("color", color_kind("#ff8c00"))],
            )
            .preset(
                "Teal",
                vec![("color", color_kind("#00d8c0"))],
            )
            .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self { schema, color: [0.0; 3] };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for Solid {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(Solid::new())
}

impl Effect for Solid {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.color = to_linear(p::color(&self.schema, props, "color"));
    }

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        canvas.pixels.fill(self.color);
    }
}

fn color_kind(s: &str) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Color {
        default: s.parse().unwrap_or(canvas_proto::Rgb8::BLACK),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::Inputs;

    #[test]
    fn fills_the_canvas_with_the_color() {
        let mut e = Solid::new();
        let mut canvas = Canvas::new(16, 8);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let want = "#6000fc".parse::<canvas_proto::Rgb8>().unwrap().to_f32();
        for p in canvas.pixels.iter() {
            assert_eq!(p, &want);
        }
    }

    #[test]
    fn a_missing_color_falls_back_to_the_default() {
        let mut e = Solid::new();
        e.init(&Props::new());
        let mut canvas = Canvas::new(4, 4);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let want = "#6000fc".parse::<canvas_proto::Rgb8>().unwrap().to_f32();
        assert_eq!(canvas.pixels[0], want);
    }
}
