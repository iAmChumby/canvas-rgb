//! Gradient: a linear gradient between two colors, optionally scrolling.

use crate::color::{mix, to_linear, Rgb};
use crate::props_util as p;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, PropertySchema, Props};

/// The Gradient effect.
pub struct Gradient {
    schema: PropertySchema,
    from: Rgb,
    to: Rgb,
    angle_deg: f32,
    scroll: f32,
    /// Accumulated scroll offset in logical units.
    offset: f32,
}

impl Gradient {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(
            "gradient",
            "Gradient",
            "A linear gradient between two colors, optionally scrolling.",
        )
        .color("color1", "Color 1", "#6000fc")
        .color("color2", "Color 2", "#00d8ff")
        .hue("angle", "Angle", 90.0)
        .number("scroll_speed", "Scroll Speed", 0.0, 100.0, 1.0, 0.0)
        .preset(
            "Sunset",
            vec![
                ("color1", color_kind("#ff2d55")),
                ("color2", color_kind("#ffb703")),
                ("angle", hue_kind(120.0)),
            ],
        )
        .preset(
            "Ocean",
            vec![
                ("color1", color_kind("#0a0a4a")),
                ("color2", color_kind("#1e6fd9")),
                ("angle", hue_kind(90.0)),
            ],
        )
        .preset(
            "Scrolling",
            vec![("scroll_speed", number_kind(20.0))],
        )
        .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self { schema, from: [0.0; 3], to: [0.0; 3], angle_deg: 90.0, scroll: 0.0, offset: 0.0 };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for Gradient {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(Gradient::new())
}

impl Effect for Gradient {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.from = to_linear(p::color(&self.schema, props, "color1"));
        self.to = to_linear(p::color(&self.schema, props, "color2"));
        self.angle_deg = p::hue(&self.schema, props, "angle") as f32;
        self.scroll = p::number(&self.schema, props, "scroll_speed") as f32;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.offset += self.scroll * dt;

        let rad = self.angle_deg.to_radians();
        // The gradient axis, in logical units.
        let (dx, dy) = (rad.cos(), rad.sin());
        // Project the canvas corners onto the axis to find the 0..1 range.
        let corners = [(0.0f32, 0.0f32), (320.0, 0.0), (0.0, 200.0), (320.0, 200.0)];
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for (x, y) in corners {
            let t = x * dx + y * dy;
            lo = lo.min(t);
            hi = hi.max(t);
        }
        let span = (hi - lo).max(f32::EPSILON);
        let s = canvas.scale();
        // The scroll shifts the gradient along its own axis, as a fraction of a period.
        let shift = (self.offset / 320.0).rem_euclid(1.0);
        let inv_span = 1.0 / span;
        let width = canvas.width as usize;
        for y in 0..canvas.height {
            let ly = (y as f32 + 0.5) / s;
            let row_t = ly * dy - lo;
            let out_row = &mut canvas.pixels[y as usize * width..(y as usize + 1) * width];
            for (x, out) in out_row.iter_mut().enumerate() {
                let lx = (x as f32 + 0.5) / s;
                // Wrap into 0..1 with one conditional add or subtract: the position is
                // already within one period, and the shift is in 0..1.
                let mut t = (lx * dx + row_t) * inv_span + shift;
                if t >= 1.0 {
                    t -= 1.0;
                } else if t < 0.0 {
                    t += 1.0;
                }
                *out = mix(self.from, self.to, t.clamp(0.0, 1.0));
            }
        }
    }
}

fn color_kind(s: &str) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Color {
        default: s.parse().unwrap_or(canvas_proto::Rgb8::BLACK),
    }
}

fn number_kind(n: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: n }
}

fn hue_kind(h: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Hue { default: h }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::Inputs;

    #[test]
    fn both_colors_appear_somewhere_on_the_canvas() {
        let mut e = Gradient::new();
        let mut canvas = Canvas::new(64, 40);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let c1 = "#6000fc".parse::<canvas_proto::Rgb8>().unwrap().to_f32();
        let c2 = "#00d8ff".parse::<canvas_proto::Rgb8>().unwrap().to_f32();
        let first = canvas.pixels[0];
        let mut found_c1 = false;
        let mut found_c2 = false;
        for p in canvas.pixels.iter() {
            let d1: f32 = (0..3).map(|i| (p[i] - c1[i]).abs()).sum();
            let d2: f32 = (0..3).map(|i| (p[i] - c2[i]).abs()).sum();
            if d1 < 0.05 {
                found_c1 = true;
            }
            if d2 < 0.05 {
                found_c2 = true;
            }
        }
        assert!(found_c1, "color1 never appears: first {first:?}");
        assert!(found_c2, "color2 never appears");
    }

    #[test]
    fn the_gradient_changes_when_the_angle_changes() {
        let mut e = Gradient::new();
        let mut canvas = Canvas::new(32, 32);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let before = canvas.pixels.clone();
        let mut props = e.schema.defaults();
        props.insert("angle".into(), canvas_proto::PropValue::Hue(0.0));
        e.init(&props);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        assert_ne!(before, canvas.pixels);
    }
}
