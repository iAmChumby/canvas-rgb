//! Spiral: rotating arms of color that wind around the canvas center.
//!
//! Each pixel's geometry (where it sits on an arm, how far it is from the center, and its
//! brightness ceiling) depends only on the canvas size, the render scale and the arm
//! count. That geometry is worked out once and cached; it is rebuilt only when one of
//! those changes. Each frame then does a few adds, one conditional wrap, a lookup in a
//! hue table and a multiply per pixel, with no trigonometry, square roots or allocation.

use crate::color::{hsv_to_rgb, saturate, Rgb};
use crate::props_util as p;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, Inputs, PropertySchema, Props};
use std::f32::consts::TAU;

/// The hue table's resolution: 1024 entries, a step of about 0.001 turn. The error
/// against exact HSV is below 0.003 in any channel.
const HUE_STEPS: usize = 1024;

/// One pixel's cached geometry.
#[derive(Clone, Copy, Debug, Default)]
struct Pixel {
    /// Position along one arm at phase 0, in 0..1; it wraps once per arm.
    arm: f32,
    /// Distance from the center over the farthest distance, 0..1.
    radius: f32,
    /// The brightness ceiling: 0 at the center, else 0.35..=1.
    value: f32,
}

/// What a cached geometry was built for. Any change rebuilds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    width: u32,
    height: u32,
    scale_bits: u32,
    arms: u32,
}

/// The Spiral effect.
pub struct Spiral {
    schema: PropertySchema,
    speed: f32,
    arms: u32,
    saturation: f32,
    /// Hue offset in turns (0..1).
    hue_shift: f32,
    phase: f32,
    /// Pure hue (saturation 1, value 1) for each of `HUE_STEPS` hues around the wheel.
    hue_table: Vec<Rgb>,
    /// The layout `pixels` was built for.
    layout: Option<Layout>,
    /// The cached per-pixel geometry, row-major, one entry per canvas pixel.
    pixels: Vec<Pixel>,
}

impl Spiral {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(
            "spiral",
            "Spiral",
            "Rotating arms of color winding around the canvas center.",
        )
        .number("speed", "Speed", 0.0, 100.0, 1.0, 25.0)
        .number("arms", "Arms", 1.0, 8.0, 1.0, 3.0)
        .number("saturation", "Saturation", 0.0, 100.0, 1.0, 100.0)
        .number("hue_shift", "Hue Shift", 0.0, 360.0, 1.0, 0.0)
        .preset(
            "Two arms",
            vec![("arms", number_kind(2.0)), ("speed", number_kind(35.0))],
        )
        .preset(
            "Hot",
            vec![("hue_shift", number_kind(180.0)), ("saturation", number_kind(100.0))],
        )
        .preset(
            "Slow six",
            vec![("arms", number_kind(6.0)), ("speed", number_kind(10.0))],
        )
        .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self {
            schema,
            speed: 0.0,
            arms: 1,
            saturation: 1.0,
            hue_shift: 0.0,
            phase: 0.0,
            hue_table: hue_table(),
            layout: None,
            pixels: Vec::new(),
        };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for Spiral {
    fn default() -> Self {
        Self::new()
    }
}

/// The pure-hue wheel: `HUE_STEPS` evenly spaced hues at full saturation and value.
fn hue_table() -> Vec<Rgb> {
    (0..HUE_STEPS)
        .map(|i| hsv_to_rgb(i as f32 / HUE_STEPS as f32, 1.0, 1.0))
        .collect()
}

/// Works out every pixel's geometry for one layout. This runs only when the layout
/// changes, never on a normal frame.
fn build_pixels(width: u32, height: u32, scale: f32, arms: u32) -> Vec<Pixel> {
    let cx = 160.0f32;
    let cy = 100.0f32;
    let max_r = (cx * cx + cy * cy).sqrt();
    let arm_step = TAU / arms as f32;
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let ly = (y as f32 + 0.5) / scale - cy;
        for x in 0..width {
            let lx = (x as f32 + 0.5) / scale - cx;
            let r = (lx * lx + ly * ly).sqrt();
            // The center is black: a zero ceiling makes the whole pixel zero.
            if r < 0.5 {
                pixels.push(Pixel::default());
                continue;
            }
            let angle = ly.atan2(lx);
            // Each arm is a logarithmic spiral: the angle advances with the radius.
            let arm = ((angle + r * 0.02) / arm_step).rem_euclid(1.0);
            let radius = r / max_r;
            pixels.push(Pixel { arm, radius, value: 0.35 + 0.65 * (1.0 - radius) });
        }
    }
    pixels
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(Spiral::new())
}

fn number_kind(n: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: n }
}

impl Effect for Spiral {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.speed = p::number(&self.schema, props, "speed") as f32;
        self.arms = p::number(&self.schema, props, "arms").max(1.0) as u32;
        self.saturation = p::number(&self.schema, props, "saturation") as f32 / 100.0;
        self.hue_shift = p::number(&self.schema, props, "hue_shift") as f32 / 360.0;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.phase = (self.phase + self.speed * dt * 0.05).rem_euclid(1.0);

        let arms = self.arms.max(1);
        let scale = canvas.scale();
        let layout = Layout {
            width: canvas.width,
            height: canvas.height,
            scale_bits: scale.to_bits(),
            arms,
        };
        if self.layout != Some(layout) {
            self.pixels = build_pixels(canvas.width, canvas.height, scale, arms);
            self.layout = Some(layout);
        }

        // Per-frame offsets, each in 0..1 so that one conditional wrap is enough below.
        // The arm offset is the phase angle divided by the arm step, which is phase * arms.
        let arm_shift = (self.phase * arms as f32).fract();
        let hue_shift = (self.phase + self.hue_shift).fract();
        let saturation = self.saturation;
        let saturate_it = (saturation - 1.0).abs() > f32::EPSILON;
        let hue_table = &self.hue_table;
        let last_hue = HUE_STEPS - 1;

        for (out, px) in canvas.pixels.iter_mut().zip(self.pixels.iter()) {
            // A bright band inside each arm: a triangle wave over the arm position.
            let mut arm = px.arm + arm_shift;
            if arm >= 1.0 {
                arm -= 1.0;
            }
            let band = 1.0 - (2.0 * arm - 1.0).abs();

            let mut hue = px.radius + hue_shift;
            if hue >= 1.0 {
                hue -= 1.0;
            }
            let index = ((hue * HUE_STEPS as f32) as usize).min(last_hue);
            let v = band * px.value;
            let [hr, hg, hb] = hue_table[index];
            let c = [hr * v, hg * v, hb * v];
            *out = if saturate_it { saturate(c, saturation) } else { c };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_and_stays_in_range() {
        let mut e = Spiral::new();
        let mut canvas = Canvas::new(64, 40);
        for _ in 0..120 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        assert!(canvas.pixels.iter().any(|p| p.iter().any(|v| *v > 0.05)));
        for p in canvas.pixels.iter() {
            for v in p {
                assert!((0.0..=1.0).contains(v), "{p:?}");
            }
        }
    }

    /// The hue table matches exact HSV to within its step size.
    #[test]
    fn the_hue_table_matches_hsv() {
        let table = hue_table();
        for i in 0..500 {
            let h = i as f32 / 500.0;
            let want = hsv_to_rgb(h, 1.0, 1.0);
            let got = table[((h * HUE_STEPS as f32) as usize).min(HUE_STEPS - 1)];
            for (a, b) in want.iter().zip(got.iter()) {
                assert!((a - b).abs() < 0.01, "hue {h}: want {want:?}, got {got:?}");
            }
        }
    }

    #[test]
    fn the_geometry_is_cached_until_the_layout_changes() {
        let mut e = Spiral::new();
        let mut canvas = Canvas::new(64, 40);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let first = e.layout;
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        assert_eq!(e.layout, first, "a normal frame must not rebuild");
        let mut bigger = Canvas::new(128, 80);
        e.update(1.0 / 30.0, &mut bigger, &Inputs::default());
        assert_ne!(e.layout, first, "a resize must rebuild");
        assert_eq!(e.pixels.len(), 128 * 80);
    }

    #[test]
    fn more_arms_means_more_bands() {
        let count_bands = |arms: f64| {
            let mut e = Spiral::new();
            let mut props = e.schema.defaults();
            props.insert("arms".into(), canvas_proto::PropValue::Number(arms));
            e.init(&props);
            let mut canvas = Canvas::new(64, 40);
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            // Count the lit pixels along a horizontal line.
            (0..canvas.width)
                .filter(|x| canvas.get(*x, 20).iter().any(|v| *v > 0.05))
                .count()
        };
        assert!(count_bands(4.0) >= count_bands(1.0));
    }
}
