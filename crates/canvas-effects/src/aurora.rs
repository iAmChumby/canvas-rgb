//! Aurora: layered moving noise bands in a three-color palette.
//!
//! The noise is a sum of sine waves. Each sine splits into a column share and a row
//! share: `sin(a + b) = sin(a) cos(b) + cos(a) sin(b)`. The column shares are computed
//! once per frame and the row shares once per row, so a pixel costs two multiplies and
//! an add for its sine, plus the ribbon fold and palette blend.

use crate::color::{to_linear, Rgb};
use crate::props_util as p;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, Inputs, PropertySchema, Props};

/// The Aurora effect.
pub struct Aurora {
    schema: PropertySchema,
    colors: [Rgb; 3],
    speed: f32,
    scale: f32,
    /// How many ribbons the noise field is folded into.
    bands: f32,
    phase: f32,
    /// The per-column shares for this frame. Storage is reused across frames.
    columns: Vec<Column>,
}

/// One column's share of the noise, for this frame. Every value is already multiplied
/// by the band count, and the sine term by 0.1, so a pixel only adds them up.
#[derive(Clone, Copy, Debug, Default)]
struct Column {
    /// The first sine term.
    wave: f32,
    /// `sin` of the second sine's column angle, scaled.
    drift_sin: f32,
    /// `cos` of the second sine's column angle, scaled.
    drift_cos: f32,
}

impl Aurora {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(
            "aurora",
            "Aurora",
            "Layered moving noise bands in a three-color palette.",
        )
        .color("color1", "Color 1", "#00d8a0")
        .color("color2", "Color 2", "#0066ff")
        .color("color3", "Color 3", "#a300ff")
        .number("speed", "Speed", 0.0, 100.0, 1.0, 20.0)
        .number("scale", "Scale", 1.0, 20.0, 0.5, 6.0)
        .number("bands", "Bands", 1.0, 6.0, 1.0, 3.0)
        .preset(
            "Northern lights",
            vec![
                ("color1", color_kind("#00ff9c")),
                ("color2", color_kind("#0077ff")),
                ("color3", color_kind("#c400ff")),
            ],
        )
        .preset(
            "Ember",
            vec![
                ("color1", color_kind("#ff5e00")),
                ("color2", color_kind("#ffb703")),
                ("color3", color_kind("#8a0303")),
            ],
        )
        .preset(
            "Slow and wide",
            vec![("speed", number_kind(6.0)), ("scale", number_kind(3.0))],
        )
        .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self {
            schema,
            colors: [[0.0; 3]; 3],
            speed: 0.0,
            scale: 6.0,
            bands: 3.0,
            phase: 0.0,
            columns: Vec::new(),
        };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for Aurora {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(Aurora::new())
}

impl Effect for Aurora {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.colors = [
            to_linear(p::color(&self.schema, props, "color1")),
            to_linear(p::color(&self.schema, props, "color2")),
            to_linear(p::color(&self.schema, props, "color3")),
        ];
        self.speed = p::number(&self.schema, props, "speed") as f32;
        self.scale = p::number(&self.schema, props, "scale").max(0.1) as f32;
        self.bands = p::number(&self.schema, props, "bands").max(1.0) as f32;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.phase = (self.phase + self.speed * dt * 0.1).rem_euclid(1000.0);

        let s = canvas.scale();
        let bands = self.bands.max(1.0);
        let scale = self.scale;
        let phase = self.phase;
        // The three colors, with the first repeated at the end so that the ribbon blend
        // from the last color back to the first needs no wrap-around branch.
        let [c0, c1, c2] = self.colors;
        let palette = [c0, c1, c2, c0];
        let width = canvas.width as usize;

        // The column shares, once per frame. The storage is reused.
        self.columns.clear();
        for x in 0..canvas.width {
            let lx = (x as f32 + 0.5) / s;
            let xn = lx / 40.0 * scale;
            let (drift_sin, drift_cos) = (xn * 0.45).sin_cos();
            self.columns.push(Column {
                wave: (xn * 0.7 + phase).sin() * 0.25 * bands,
                drift_sin: drift_sin * 0.1 * bands,
                drift_cos: drift_cos * 0.1 * bands,
            });
        }

        for y in 0..canvas.height {
            let ly = (y as f32 + 0.5) / s;
            // The row's shares, once per row. The noise is n = wave/4 + (b/2 + 1/2) + c/10
            // with b = 0.3 sin(row term) and c = sin(drift + row offset); here `base` is
            // the row's part of u = n * bands.
            let row = ly / 40.0 * scale;
            let b_term = (row * 1.3 - phase * 0.6).sin() * 0.3;
            let base = (b_term * 0.5 + 0.5) * bands;
            let (row_sin, row_cos) = (row * 0.45 + phase * 1.7).sin_cos();

            let start = y as usize * width;
            let out_row = &mut canvas.pixels[start..start + width];
            for (out, col) in out_row.iter_mut().zip(self.columns.iter()) {
                // u = n * bands, and the second sine is folded in through its two shares.
                let u = col.wave + base + col.drift_sin * row_cos + col.drift_cos * row_sin;
                // Fold the noise into `bands` soft ribbons: t is its position in one ribbon.
                let t = (u - (u as i32) as f32).max(0.0);
                let t3 = t * 3.0;
                let idx = (t3 as i32).clamp(0, 2) as usize;
                let local = t3 - idx as f32;
                let (a, b) = (palette[idx], palette[idx + 1]);
                // Brighten the ribbon's core and fade its edges.
                let shape = 1.0 - (local * 2.0 - 1.0).abs();
                let level = 0.15 + 0.85 * shape * shape;
                *out = [
                    (a[0] + (b[0] - a[0]) * local) * level,
                    (a[1] + (b[1] - a[1]) * local) * level,
                    (a[2] + (b[2] - a[2]) * local) * level,
                ];
            }
        }
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

    /// The straightforward formula, with a direct `sin` per term, for one pixel. The
    /// optimized update must agree with it.
    fn reference_pixel(colors: [Rgb; 3], bands: f32, scale: f32, phase: f32, lx: f32, ly: f32) -> Rgb {
        let xn = lx / 40.0 * scale;
        let row = ly / 40.0 * scale;
        let a = (xn * 0.7 + phase).sin();
        let b = (row * 1.3 - phase * 0.6).sin() * 0.3;
        let c = (xn * 0.45 + row * 0.45 + phase * 1.7).sin();
        let n = (a * 0.5 + b + c * 0.2) * 0.5 + 0.5;
        let t = (n * bands).rem_euclid(1.0);
        let t3 = t * 3.0;
        let idx = (t3 as usize).min(2);
        let local = t3 - idx as f32;
        let next = (idx + 1) % 3;
        let shape = (1.0 - (local * 2.0 - 1.0).abs()).clamp(0.0, 1.0);
        let level = 0.15 + 0.85 * shape * shape;
        let (p, q) = (colors[idx], colors[next]);
        [
            (p[0] + (q[0] - p[0]) * local) * level,
            (p[1] + (q[1] - p[1]) * local) * level,
            (p[2] + (q[2] - p[2]) * local) * level,
        ]
    }

    /// The split-sine update matches the direct formula on every pixel of a frame.
    #[test]
    fn the_split_sine_matches_the_direct_formula() {
        let mut e = Aurora::new();
        let mut canvas = Canvas::new(64, 40);
        for _ in 0..7 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        let s = canvas.scale();
        let mut worst = 0.0f32;
        for y in 0..canvas.height {
            for x in 0..canvas.width {
                let lx = (x as f32 + 0.5) / s;
                let ly = (y as f32 + 0.5) / s;
                let want = reference_pixel(e.colors, e.bands, e.scale, e.phase, lx, ly);
                let got = canvas.get(x, y);
                for (a, b) in want.iter().zip(got.iter()) {
                    worst = worst.max((a - b).abs());
                }
            }
        }
        assert!(worst < 0.01, "worst channel difference {worst}");
    }

    #[test]
    fn draws_soft_bands() {
        let mut e = Aurora::new();
        let mut canvas = Canvas::new(64, 40);
        for _ in 0..60 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        assert!(canvas.pixels.iter().any(|p| p.iter().any(|v| *v > 0.05)));
        for p in canvas.pixels.iter() {
            for v in p {
                assert!((0.0..=1.0).contains(v), "{p:?}");
            }
        }
    }

    #[test]
    fn the_bands_move() {
        let mut e = Aurora::new();
        let mut canvas = Canvas::new(32, 20);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let before = canvas.pixels.clone();
        for _ in 0..30 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        assert_ne!(before, canvas.pixels);
    }
}
