//! Rainbow Wave: a rainbow band that travels across the canvas.

use crate::color::hsv_to_rgb;
use crate::props_util as p;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, PropertySchema, Props};

/// The hue table's size: pure hues at full saturation, one entry per 1/1024 turn.
const HUE_STEPS: usize = 1024;

/// The Rainbow Wave effect.
pub struct RainbowWave {
    schema: PropertySchema,
    speed: f32,
    width: f32,
    /// Travel direction in degrees: 0 = right, 90 = down.
    direction: f32,
    /// How many full hue cycles the band spans.
    cycles: f32,
    phase: f32,
    /// Pure hues (saturation 1, value 1), so a pixel is a lookup times its brightness.
    hue_table: Vec<[f32; 3]>,
}

impl RainbowWave {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(
            "rainbow_wave",
            "Rainbow Wave",
            "A rainbow band that travels across the canvas.",
        )
        .number("speed", "Speed", 0.0, 100.0, 1.0, 30.0)
        // Width is in logical units, as it always was: a saved 60 stays a 60-unit band. The
        // default (190, about 60 % of the 320-unit canvas) is what keeps a new effect from
        // being a thin sliver on black. The range reaches the whole travel period (480),
        // so a band can be as wide as the canvas is tall and never leave a black gap.
        .number("width", "Width", 5.0, 480.0, 1.0, 190.0)
        .number("direction", "Direction", 0.0, 360.0, 1.0, 0.0)
        .number("cycles", "Cycles", 1.0, 8.0, 1.0, 1.0)
        .preset(
            "Fast sweep",
            vec![("speed", number_kind(60.0)), ("width", number_kind(40.0))],
        )
        .preset(
            "Diagonal",
            vec![("direction", number_kind(45.0)), ("speed", number_kind(40.0))],
        )
        .preset(
            "Wide arc",
            vec![("width", number_kind(150.0)), ("cycles", number_kind(2.0))],
        )
        // The tile shows the whole spectrum across the tile: the band is 400 units wide, so
        // it covers the canvas from end to end and no part of the tile is ever black, and
        // the loop drifts slowly enough that the crossfade at its restart does not double
        // the picture.
        .thumbnail(crate::schema::recipe(18.0, 3.75, 8, vec![("speed", crate::schema::num(0.4)), ("direction", crate::schema::num(90.0)), ("cycles", crate::schema::num(2.0)), ("width", crate::schema::num(400.0))]))
        .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let hue_table = (0..HUE_STEPS)
            .map(|i| hsv_to_rgb(i as f32 / HUE_STEPS as f32, 1.0, 1.0))
            .collect();
        let mut e = Self {
            schema,
            speed: 0.0,
            width: 0.0,
            direction: 0.0,
            cycles: 1.0,
            phase: 0.0,
            hue_table,
        };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for RainbowWave {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(RainbowWave::new())
}

impl Effect for RainbowWave {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.speed = p::number(&self.schema, props, "speed") as f32;
        self.width = p::number(&self.schema, props, "width") as f32;
        self.direction = p::number(&self.schema, props, "direction") as f32;
        self.cycles = p::number(&self.schema, props, "cycles") as f32;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let cycles = self.cycles;
        self.phase = (self.phase + self.speed * dt * 0.1).rem_euclid(1.0);

        let rad = self.direction.to_radians();
        let (dx, dy) = (rad.cos(), rad.sin());
        // The wave's position along the axis, in logical units.
        let travel = self.phase * 480.0 - 80.0;
        let width = self.width.max(1.0);
        let s = canvas.scale();
        let half = width * 0.5;
        let inv_half = 1.0 / half;
        let phase = self.phase;
        let table = &self.hue_table;
        let width_px = canvas.width as usize;
        for y in 0..canvas.height {
            let ly = (y as f32 + 0.5) / s;
            // The row's part of the projection onto the axis, minus the wave's travel.
            let row_t = ly * dy - travel;
            let out_row = &mut canvas.pixels[y as usize * width_px..(y as usize + 1) * width_px];
            for (x, out) in out_row.iter_mut().enumerate() {
                let lx = (x as f32 + 0.5) / s;
                // The distance from the wave's center, wrapped into one 480-unit band.
                let v = lx * dx + row_t;
                let whole = (v * (1.0 / 480.0)) as i32 as f32;
                let mut r = v - whole * 480.0;
                if r < 0.0 {
                    r += 480.0;
                }
                let d = (r - 240.0).abs();
                if d > half {
                    *out = [0.0, 0.0, 0.0];
                    continue;
                }
                // Hue runs across the band; the band is dimmer at its edges.
                let edge = d * inv_half;
                let local = edge * 0.5;
                let h = phase + local * cycles;
                let mut hue = h - (h as i32) as f32;
                if hue < 0.0 {
                    hue += 1.0;
                }
                let falloff = 1.0 - edge * 0.35;
                let index = ((hue * HUE_STEPS as f32) as usize).min(HUE_STEPS - 1);
                let [hr, hg, hb] = table[index];
                *out = [hr * falloff, hg * falloff, hb * falloff];
            }
        }
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
    fn draws_colored_pixels() {
        let mut e = RainbowWave::new();
        let mut canvas = Canvas::new(64, 40);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let lit = canvas.pixels.iter().filter(|p| p.iter().any(|v| *v > 0.05)).count();
        assert!(lit > 0, "nothing was drawn");
    }

    #[test]
    fn the_wave_moves() {
        let mut e = RainbowWave::new();
        let mut canvas = Canvas::new(64, 40);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let before = canvas.pixels.clone();
        for _ in 0..20 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        assert_ne!(before, canvas.pixels);
    }

    #[test]
    fn output_stays_in_range() {
        let mut e = RainbowWave::new();
        let mut canvas = Canvas::new(32, 20);
        for _ in 0..120 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        for p in canvas.pixels.iter() {
            for v in p {
                assert!((0.0..=1.0).contains(v), "{p:?}");
            }
        }
    }

    /// At the defaults the band spans most of a canvas width, as the thumbnail shows: black
    /// pixels are a minority of the frames, not nine in ten.
    #[test]
    fn default_band_is_not_a_thin_sliver_on_black() {
        let mut e = RainbowWave::new();
        let mut canvas = Canvas::new(320, 200);
        let (mut black, mut total) = (0usize, 0usize);
        for _ in 0..48 {
            e.update(1.0 / 12.0, &mut canvas, &Inputs::default());
            black += canvas.pixels.iter().filter(|p| p.iter().all(|v| *v < 0.02)).count();
            total += canvas.pixels.len();
        }
        let fraction = black as f32 / total as f32;
        assert!(fraction < 0.7, "{:.0}% of the pixels are black", fraction * 100.0);
    }

    fn black_fraction(props: &[(&str, f64)]) -> f32 {
        let mut e = RainbowWave::new();
        let mut p = e.schema.defaults();
        for (k, v) in props {
            p.insert((*k).into(), canvas_proto::PropValue::Number(*v));
        }
        e.init(&p);
        let mut canvas = Canvas::new(320, 200);
        let (mut black, mut total) = (0usize, 0usize);
        for _ in 0..48 {
            e.update(1.0 / 12.0, &mut canvas, &Inputs::default());
            black += canvas.pixels.iter().filter(|p| p.iter().all(|v| *v < 0.02)).count();
            total += canvas.pixels.len();
        }
        black as f32 / total as f32
    }

    /// A saved width keeps its old meaning (logical units): 60 stays the thin band it was,
    /// and the "Fast sweep" (40) and "Wide arc" (150) presets look as they always did.
    #[test]
    fn saved_widths_keep_their_old_meaning() {
        let thin = black_fraction(&[("width", 60.0)]);
        assert!(thin > 0.8, "a saved 60 is a 60-unit band, got {:.0}% black", thin * 100.0);
        let fast = black_fraction(&[("width", 40.0)]);
        assert!(fast > thin, "a 40-unit band is thinner still");
        let wide = black_fraction(&[("width", 150.0)]);
        assert!(wide < 0.8 && wide > black_fraction(&[]), "150 is wider than 60 and narrower than the new default");
    }
}
