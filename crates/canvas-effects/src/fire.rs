//! Fire: classic heat propagation upward, from a palette of flame colors.
//!
//! The heat field is a small grid (160x100) stepped a few times per frame. Drawing maps
//! each canvas pixel to a heat cell through two index tables built once per canvas size,
//! and colors come from a 1024-entry lookup built once per palette change, so a frame
//! does no division, no `floor` and no allocation.

use crate::props_util as p;
use crate::rng::Rng;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, PropertySchema, Props};

/// Entries in the palette lookup. Fine enough that the ramp looks continuous.
const LUT_SIZE: usize = 1024;
/// The stretches the source row is cut into: each is one tongue of flame that rises and dies.
const TONGUES: usize = 14;

/// The Fire effect.
pub struct Fire {
    schema: PropertySchema,
    /// The heat field, one value per cell. 0 is cold, 1 is the hottest.
    heat: Vec<f32>,
    w: usize,
    h: usize,
    intensity: f32,
    palette: String,
    rng: Rng,
    /// The palette as linear RGB triples, from cold to hot.
    ramp: Vec<[f32; 3]>,
    /// The ramp sampled at `LUT_SIZE` evenly spaced heats.
    lut: Vec<[f32; 3]>,
    /// Scratch buffer for the next heat step, reused so the hot path does not allocate.
    next: Vec<f32>,
    cooling: f32,
    /// The canvas size the index tables below were built for: width, height, scale bits.
    map_key: Option<(u32, u32, u32)>,
    /// For each canvas column, the heat column it shows.
    col_map: Vec<usize>,
    /// How strongly each stretch of the source row burns, drifting with time (see `step`).
    tongues: Vec<f32>,
    /// For each canvas row, the heat row it shows, already multiplied by the width.
    row_map: Vec<usize>,
}

impl Fire {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new("fire", "Fire", "Classic heat propagation upward.")
            .number("intensity", "Intensity", 0.0, 100.0, 1.0, 55.0)
            .choice("palette", "Palette", &["Classic", "Blue", "Green", "Mono"], "Classic")
            .number("cooling", "Cooling", 0.0, 100.0, 1.0, 30.0)
            .preset(
                "Blue flame",
                vec![("palette", choice_kind("Blue"))],
            )
            .preset(
                "Slow burn",
                vec![("intensity", number_kind(30.0)), ("cooling", number_kind(50.0))],
        )
        .preset(
            "White hot",
            vec![
                ("palette", choice_kind("Mono")),
                ("intensity", number_kind(90.0)),
            ],
        )
        // At the defaults the flames cool within the bottom fifth of the canvas. Full intensity and
        // low cooling carry the heat to the top, four seconds of warm-up let it climb there, and the
        // end crossfades into the start because the tongues of flame never repeat.
        .thumbnail(crate::schema::recipe(4.0, 2.0, 8, vec![("intensity", crate::schema::num(100.0)), ("cooling", crate::schema::num(9.0))]))
        .build()
    }

    pub fn new() -> Self {
        Self::with_seed(Rng::from_entropy().next_u64())
    }

    pub fn with_seed(seed: u64) -> Self {
        let schema = Self::schema();
        let (w, h) = (160usize, 100usize);
        let mut e = Self {
            schema,
            heat: vec![0.0; w * h],
            w,
            h,
            intensity: 55.0,
            palette: "Classic".to_string(),
            rng: Rng::new(seed),
            ramp: Vec::new(),
            lut: Vec::new(),
            next: vec![0.0; w * h],
            cooling: 30.0,
            map_key: None,
            col_map: Vec::new(),
            tongues: vec![0.5; TONGUES],
            row_map: Vec::new(),
        };
        e.init(&e.schema.defaults());
        e
    }

    /// Builds the color ramp for the chosen palette.
    fn build_ramp(palette: &str) -> Vec<[f32; 3]> {
        let hexes: &[&str] = match palette {
            "Blue" => &["#000000", "#001a66", "#0044aa", "#0088ff", "#66ccff", "#ffffff"],
            "Green" => &["#000000", "#003300", "#006600", "#00aa00", "#88ff88", "#ffffff"],
            "Mono" => &["#000000", "#1a1a1a", "#4d4d4d", "#808080", "#bfbfbf", "#ffffff"],
            _ => &["#000000", "#3b0808", "#8b1500", "#cd2b00", "#ff6a00", "#ffd700", "#fff5b0"],
        };
        hexes
            .iter()
            .map(|h| h.parse::<canvas_proto::Rgb8>().unwrap_or(canvas_proto::Rgb8::BLACK).to_f32())
            .collect()
    }

    /// Samples the ramp at `LUT_SIZE` evenly spaced heats.
    fn build_lut(ramp: &[[f32; 3]]) -> Vec<[f32; 3]> {
        (0..LUT_SIZE)
            .map(|i| ramp_at(ramp, i as f32 / (LUT_SIZE - 1) as f32))
            .collect()
    }
}

impl Default for Fire {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(Fire::new())
}

/// Interpolates the ramp at `v` (0..1). Used only to build the lookup table.
fn ramp_at(ramp: &[[f32; 3]], v: f32) -> [f32; 3] {
    if ramp.is_empty() {
        return [0.0, 0.0, 0.0];
    }
    let n = ramp.len();
    let t = (v.clamp(0.0, 1.0) * (n - 1) as f32).min((n - 1) as f32);
    let i = t.floor() as usize;
    let j = (i + 1).min(n - 1);
    let f = t - i as f32;
    let a = ramp[i];
    let b = ramp[j];
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
}

impl Effect for Fire {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.intensity = p::number(&self.schema, props, "intensity") as f32;
        self.palette = p::choice(&self.schema, props, "palette");
        self.cooling = p::number(&self.schema, props, "cooling") as f32;
        self.ramp = Self::build_ramp(&self.palette);
        self.lut = Self::build_lut(&self.ramp);
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.update_with(dt, canvas);
    }
}

impl Fire {
    /// Builds the canvas-to-heat index tables for this canvas size. Runs only when the
    /// size or scale changes.
    fn rebuild_maps(&mut self, width: u32, height: u32, scale: f32) {
        let (w, h) = (self.w, self.h);
        self.col_map.clear();
        for x in 0..width {
            let gx = ((x as f32 + 0.5) / scale / 320.0 * w as f32) as usize;
            self.col_map.push(gx.min(w - 1));
        }
        self.row_map.clear();
        for y in 0..height {
            let gy = ((y as f32 + 0.5) / scale / 200.0 * h as f32) as usize;
            self.row_map.push(gy.min(h - 1) * w);
        }
        self.map_key = Some((width, height, scale.to_bits()));
    }

    /// The hot-path update.
    fn update_with(&mut self, dt: f32, canvas: &mut Canvas) {
        let w = self.w;
        // Step the heat field a few times per frame so the fire moves smoothly.
        let steps = ((dt * 60.0).round() as i32).clamp(1, 4);
        for _ in 0..steps {
            self.step();
        }

        let scale = canvas.scale();
        let key = (canvas.width, canvas.height, scale.to_bits());
        if self.map_key != Some(key) {
            self.rebuild_maps(canvas.width, canvas.height, scale);
        }

        // Draw the field, scaled to the canvas, through the lookup table.
        let last = (LUT_SIZE - 1) as f32;
        let width = canvas.width as usize;
        for (y, &row) in self.row_map.iter().enumerate() {
            let src = &self.heat[row..row + w];
            let out_row = &mut canvas.pixels[y * width..(y + 1) * width];
            for (out, &col) in out_row.iter_mut().zip(self.col_map.iter()) {
                let index = (src[col].clamp(0.0, 1.0) * last + 0.5) as usize;
                *out = self.lut[index.min(LUT_SIZE - 1)];
            }
        }
    }

    /// One heat-propagation step.
    fn step(&mut self) {
        let (w, h) = (self.w, self.h);
        // The bottom row is the source, heated by the intensity.
        let source = (self.intensity / 100.0).clamp(0.0, 1.0);
        // The source is not one flat line: it burns in tongues, each a stretch of the row that
        // drifts between a third and all of the intensity, so flames rise and die instead of
        // the picture being one gradient.
        for tongue in self.tongues.iter_mut() {
            *tongue = *tongue * 0.9 + 0.1 * self.rng.next_f32();
        }
        let last = (TONGUES - 1) as f32;
        for (x, cell) in self.next[(h - 1) * w..h * w].iter_mut().enumerate() {
            let at = x as f32 / (w - 1) as f32 * last;
            let i = (at as usize).min(TONGUES - 2);
            let f = at - i as f32;
            let burn = self.tongues[i] * (1.0 - f) + self.tongues[i + 1] * f;
            // Noise has a mean of 0.5 and spread of about 0.15, so scale it back to 0..1.
            let level = ((burn - 0.5) * 3.0 + 0.5).clamp(0.0, 1.0);
            *cell = source * (0.3 + 0.7 * level);
        }
        let cool = (self.cooling / 100.0).clamp(0.0, 1.0) * 0.08;
        for y in 0..h - 1 {
            let below = &self.heat[(y + 1) * w..(y + 2) * w];
            let out = &mut self.next[y * w..(y + 1) * w];
            // Average the three cells below, the classic fire algorithm. The first and
            // last columns repeat their edge neighbour, as before.
            out[0] = ((below[0] + below[0] + below[1]) / 3.0 - cool).clamp(0.0, 1.0);
            for (o, triple) in out[1..w - 1].iter_mut().zip(below.windows(3)) {
                *o = ((triple[0] + triple[1] + triple[2]) / 3.0 - cool).clamp(0.0, 1.0);
            }
            out[w - 1] = ((below[w - 2] + below[w - 1] + below[w - 1]) / 3.0 - cool).clamp(0.0, 1.0);
        }
        // A little lateral noise keeps the flame from looking like a solid block.
        for cell in self.next[..(h - 1) * w].iter_mut() {
            if self.rng.next_f32() < 0.02 {
                *cell = (*cell + 0.05).min(1.0);
            }
        }
        std::mem::swap(&mut self.heat, &mut self.next);
    }
}

fn number_kind(n: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: n }
}

fn choice_kind(s: &str) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Choice { options: vec!["Classic".into(), "Blue".into(), "Green".into(), "Mono".into()], default: s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::Inputs;

    #[test]
    fn the_fire_burns() {
        let mut e = Fire::with_seed(5);
        let mut canvas = Canvas::new(64, 40);
        for _ in 0..120 {
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
    fn the_top_is_cooler_than_the_bottom() {
        let mut e = Fire::with_seed(5);
        let mut canvas = Canvas::new(32, 20);
        for _ in 0..120 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        let bottom = e.heat[(e.h - 1) * e.w + e.w / 2];
        let top = e.heat[e.w / 2];
        assert!(bottom > top, "bottom={bottom} top={top}");
    }

    #[test]
    fn the_palette_choice_changes_the_colors() {
        let mut e = Fire::with_seed(5);
        let mut canvas = Canvas::new(32, 20);
        for _ in 0..60 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        let classic = canvas.pixels.clone();
        let mut props = e.schema.defaults();
        props.insert("palette".into(), canvas_proto::PropValue::Choice("Blue".into()));
        e.init(&props);
        for _ in 0..60 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        assert_ne!(classic, canvas.pixels);
    }

    /// The lookup table agrees with the exact ramp to within its step size.
    #[test]
    fn the_lookup_table_matches_the_ramp() {
        let e = Fire::with_seed(5);
        for i in 0..200 {
            let v = i as f32 / 199.0;
            let exact = ramp_at(&e.ramp, v);
            let index = (v * (LUT_SIZE - 1) as f32 + 0.5) as usize;
            let got = e.lut[index.min(LUT_SIZE - 1)];
            for (a, b) in exact.iter().zip(got.iter()) {
                assert!((a - b).abs() < 0.01, "v={v}: {exact:?} vs {got:?}");
            }
        }
    }
}
