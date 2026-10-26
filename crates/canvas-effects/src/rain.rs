//! Rain: drops falling from the top, each leaving a trail.

use crate::color::to_linear;
use crate::props_util as p;
use crate::rng::Rng;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, PropertySchema, Props};

/// One falling drop.
#[derive(Debug, Clone, Copy)]
struct Drop {
    x: f32,
    y: f32,
    /// Pixels per second.
    speed: f32,
    len: f32,
    brightness: f32,
}

/// The Rain effect.
pub struct Rain {
    schema: PropertySchema,
    drop_color: [f32; 3],
    background: [f32; 3],
    density: u32,
    speed: f32,
    drops: Vec<Drop>,
    rng: Rng,
    trail: Vec<[f32; 3]>,
}

impl Rain {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new("rain", "Rain", "Drops falling from the top, each leaving a trail.")
            .color("drop_color", "Drop Color", "#a8d8ff")
            .color("background", "Background", "#050814")
            .number("density", "Density", 1.0, 400.0, 1.0, 120.0)
            .number("speed", "Speed", 10.0, 400.0, 1.0, 120.0)
            .number("trail_length", "Trail Length", 2.0, 60.0, 1.0, 14.0)
            .preset(
                "Neon rain",
                vec![
                    ("drop_color", color_kind("#00ffc8")),
                    ("background", color_kind("#000000")),
                ],
            )
            .preset(
                "Heavy storm",
                vec![("density", number_kind(300.0)), ("speed", number_kind(220.0))],
        )
        .preset(
            "Drizzle",
            vec![("density", number_kind(40.0)), ("speed", number_kind(60.0))],
        )
        .build()
    }

    pub fn new() -> Self {
        Self::with_seed(Rng::from_entropy().next_u64())
    }

    pub fn with_seed(seed: u64) -> Self {
        let schema = Self::schema();
        let mut e = Self {
            schema,
            drop_color: [0.0; 3],
            background: [0.0; 3],
            density: 120,
            speed: 120.0,
            drops: Vec::new(),
            rng: Rng::new(seed),
            trail: Vec::new(),
        };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for Rain {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(Rain::new())
}

impl Effect for Rain {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.drop_color = to_linear(p::color(&self.schema, props, "drop_color"));
        self.background = to_linear(p::color(&self.schema, props, "background"));
        self.density = p::number(&self.schema, props, "density").max(1.0) as u32;
        self.speed = p::number(&self.schema, props, "speed").max(1.0) as f32;
        let len = p::number(&self.schema, props, "trail_length").max(1.0) as f32;
        if self.trail.len() != len as usize {
            self.trail.resize(len as usize, [0.0, 0.0, 0.0]);
        }
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }

        // Fade the previous frame toward the background, leaving the trails.
        let fade = 0.88f32;
        let bg = self.background;
        for p in canvas.pixels.iter_mut() {
            p[0] = p[0] * fade + bg[0] * (1.0 - fade);
            p[1] = p[1] * fade + bg[1] * (1.0 - fade);
            p[2] = p[2] * fade + bg[2] * (1.0 - fade);
        }

        // Top the pool up to the requested density.
        while self.drops.len() < self.density as usize {
            let x = self.rng.range(0.0, 320.0);
            let y = self.rng.range(-200.0, 0.0);
            let speed = self.speed * self.rng.range(0.7, 1.3);
            let len = self.rng.range(6.0, 22.0);
            let brightness = self.rng.range(0.35, 1.0);
            self.drops.push(Drop { x, y, speed, len, brightness });
        }
        if self.drops.len() > self.density as usize {
            self.drops.truncate(self.density as usize);
        }

        let color = self.drop_color;
        for drop in self.drops.iter_mut() {
            drop.y += drop.speed * dt;
            if drop.y > 210.0 {
                drop.x = self.rng.range(0.0, 320.0);
                drop.y = self.rng.range(-220.0, -20.0);
                drop.speed = self.speed * self.rng.range(0.7, 1.3);
                drop.len = self.rng.range(6.0, 22.0);
                drop.brightness = self.rng.range(0.35, 1.0);
                continue;
            }
            // Draw the drop and its trail as a tapering line.
            let tail_y = drop.y - drop.len;
            crate::raster::stroke_line(
                canvas,
                drop.x,
                tail_y,
                drop.x,
                drop.y,
                1.4,
                crate::raster::Paint::new(color, drop.brightness * 0.85),
            );
            crate::raster::fill_circle(canvas, drop.x, drop.y, 1.0, color, drop.brightness);
        }
    }
}

fn number_kind(n: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Number { min: 0.0, max: 400.0, step: 1.0, default: n }
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
    fn rain_falls() {
        let mut e = Rain::with_seed(3);
        let mut canvas = Canvas::new(64, 40);
        for _ in 0..60 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        assert!(canvas.pixels.iter().any(|p| p.iter().any(|v| *v > 0.1)));
        for p in canvas.pixels.iter() {
            for v in p {
                assert!((0.0..=1.0).contains(v), "{p:?}");
            }
        }
    }

    #[test]
    fn the_density_is_respected() {
        let mut e = Rain::with_seed(3);
        let mut props = e.schema.defaults();
        props.insert("density".into(), canvas_proto::PropValue::Number(20.0));
        e.init(&props);
        let mut canvas = Canvas::new(32, 20);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        assert_eq!(e.drops.len(), 20);
    }
}
