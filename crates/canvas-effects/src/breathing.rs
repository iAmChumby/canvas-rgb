//! Breathing: a color that pulses in and out.

use crate::color::to_linear;
use crate::props_util as p;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, PropertySchema, Props};

/// The Breathing effect.
pub struct Breathing {
    schema: PropertySchema,
    color: [f32; 3],
    /// One full breath, in seconds.
    period: f32,
    /// The dimmest the pulse gets, 0..1.
    min_brightness: f32,
    phase: f32,
}

impl Breathing {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(
            "breathing",
            "Breathing",
            "A color that pulses in and out.",
        )
        .color("color", "Color", "#6000fc")
        .number("period", "Period", 0.5, 20.0, 0.1, 4.0)
        .number("min_brightness", "Min Brightness", 0.0, 100.0, 1.0, 5.0)
        .preset(
            "Slow violet",
            vec![("period", number_kind(6.0))],
        )
        .preset(
            "Fast heartbeat",
            vec![
                ("period", number_kind(1.0)),
                ("color", color_kind("#ff2d55")),
            ],
        )
        .preset(
            "Deep glow",
            vec![
                ("period", number_kind(8.0)),
                ("min_brightness", number_kind(0.0)),
            ],
        )
        // The tile covers one whole 4 s breath, so it ends where it began, and never dims below 15 %.
        .thumbnail(crate::schema::recipe(1.0, 4.0, 0, vec![("min_brightness", crate::schema::num(15.0))]))
        .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self { schema, color: [0.0; 3], period: 4.0, phase: 0.0, min_brightness: 0.05 };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for Breathing {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(Breathing::new())
}

impl Effect for Breathing {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.color = to_linear(p::color(&self.schema, props, "color"));
        self.period = p::number(&self.schema, props, "period").max(0.01) as f32;
        self.min_brightness = p::number(&self.schema, props, "min_brightness") as f32 / 100.0;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.phase = (self.phase + dt / self.period).rem_euclid(1.0);
        // A sine breathes smoothly between the minimum brightness and full.
        let wave = (self.phase * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        let level = self.min_brightness + (1.0 - self.min_brightness) * wave;
        let c = [self.color[0] * level, self.color[1] * level, self.color[2] * level];
        canvas.pixels.fill(c);
    }
}

fn number_kind(n: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: n }
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
    fn the_brightness_oscillates() {
        let mut e = Breathing::new();
        let mut canvas = Canvas::new(8, 8);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let first = canvas.pixels[0][0];
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for _ in 0..200 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            let v = canvas.pixels[0][0];
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(hi - lo > 0.1, "no breathing: lo={lo} hi={hi}");
        assert!(first >= lo - 0.01 && first <= hi + 0.01);
    }

    #[test]
    fn a_longer_period_slows_the_pulse() {
        let mut e = Breathing::new();
        let mut props = e.schema.defaults();
        props.insert("period".into(), canvas_proto::PropValue::Number(10.0));
        e.init(&props);
        let mut canvas = Canvas::new(8, 8);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let before = canvas.pixels[0][0];
        e.update(0.1, &mut canvas, &Inputs::default());
        assert!((canvas.pixels[0][0] - before).abs() < 0.2);
    }
}
