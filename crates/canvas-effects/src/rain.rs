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

/// The fade per frame at the default Trail Length and the default Speed on the anchor
/// frame: the fixed fade Rain always had.
const DEFAULT_FADE: f32 = 0.88;
/// The Trail Length default. The fade above is measured at it, so it must stay the
/// schema's default (`the_fade_anchor_is_the_schema_default`).
const DEFAULT_TRAIL_LENGTH: f32 = 14.0;
/// The Speed default, in logical cells a second.
const DEFAULT_SPEED: f32 = 120.0;
/// The frame interval the fade anchor is measured at: the 1/30 s frame every effect test
/// drives the effects with.
const ANCHOR_FRAME_S: f32 = 1.0 / 30.0;
/// The logical cells a drop moves in one anchor frame at the default Speed: four.
const DEFAULT_STEP: f32 = DEFAULT_SPEED * ANCHOR_FRAME_S;

/// The per-frame fade for a trail `trail_length` cells long, with the drop moving `step`
/// logical cells this frame.
///
/// Trail Length sets the streak's length in *cells*, not in frames: the frames a
/// brightness survives are `cells / step`, so the fade's exponent is `step` times the
/// anchor frame's and `DEFAULT_TRAIL_LENGTH / trail_length` times its own. The decay per
/// cell is then the same at every Speed and every frame interval: a fast drop fades
/// faster each frame and leaves the same streak, and a slower PC frame changes nothing.
/// At the defaults (Trail Length 14, Speed 120, 4 cells a frame) the exponent is exactly
/// 1 and the fade is `DEFAULT_FADE`, so an effect saved with the defaults renders what it
/// always did.
fn per_frame_fade(trail_length: f32, step: f32) -> f32 {
    let trail = trail_length.max(1.0);
    let step = if step.is_finite() { step.max(0.0) } else { 0.0 };
    let exponent = (step / DEFAULT_STEP) * (DEFAULT_TRAIL_LENGTH / trail);
    if exponent == 1.0 {
        // The old constant itself, not `powf`'s answer for it.
        DEFAULT_FADE
    } else {
        DEFAULT_FADE.powf(exponent)
    }
}

/// The Rain effect.
pub struct Rain {
    schema: PropertySchema,
    drop_color: [f32; 3],
    background: [f32; 3],
    density: u32,
    speed: f32,
    /// How long a streak is, in logical cells: the control sets the fade below.
    trail_length: f32,
    drops: Vec<Drop>,
    rng: Rng,
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
        // The drops start above the canvas and need about three seconds to fill it; the rain has no
        // period, so the end of the loop crossfades into the start.
        .thumbnail(crate::schema::recipe(4.0, 2.0, 8, vec![("density", crate::schema::num(400.0)), ("trail_length", crate::schema::num(40.0)), ("drop_color", crate::schema::col("#ffffff"))]))
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
            trail_length: DEFAULT_TRAIL_LENGTH,
            drops: Vec::new(),
            rng: Rng::new(seed),
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
        // The library tile is rendered with a fixed seed, so the tile is the same rain
        // every time. On the lights the seed comes from the clock, so every run of the
        // effect is a different rain.
        if let Some(seed) = p::tile_seed(props) {
            self.rng = Rng::new(seed);
        }
        let len = p::number(&self.schema, props, "trail_length").max(1.0) as f32;
        self.trail_length = len;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }

        // Fade the previous frame toward the background, leaving the trails. How slowly
        // it fades is Trail Length: see `per_frame_fade`.
        let fade = per_frame_fade(self.trail_length, self.speed * dt);
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

    /// The mean streak, in cells, behind the drop heads: for every column that has a
    /// lit cell, the run of consecutive lit cells upward from the lowest one (the drop
    /// head leaves the trail above it, because the drops fall).
    fn mean_streak(canvas: &Canvas, threshold: f32) -> f32 {
        let w = canvas.width as usize;
        let h = canvas.height as usize;
        let lit = |x: usize, y: usize| {
            canvas.pixels[y * w + x].iter().copied().fold(0.0f32, f32::max) > threshold
        };
        let mut total = 0usize;
        let mut columns = 0usize;
        for x in 0..w {
            let Some(head) = (0..h).rev().find(|&y| lit(x, y)) else {
                continue;
            };
            let mut len = 1usize;
            let mut y = head;
            while y > 0 && lit(x, y - 1) {
                len += 1;
                y -= 1;
            }
            total += len;
            columns += 1;
        }
        total as f32 / columns.max(1) as f32
    }

    /// The brightness a trail has to fall below to count as dark. The default
    /// `#050814` background sits at 0.078 in its brightest channel (20/255), so this is
    /// one step above it: the background alone is never counted as a streak.
    const DARK: f32 = 0.12;

    /// Rain with one Trail Length, `frames` frames of drops falling.
    fn rain_with(trail_length: f64, density: f64, frames: usize) -> Canvas {
        let mut e = Rain::with_seed(3);
        let mut props = e.schema.defaults();
        props.insert("trail_length".into(), canvas_proto::PropValue::Number(trail_length));
        props.insert("density".into(), canvas_proto::PropValue::Number(density));
        e.init(&props);
        let mut canvas = Canvas::new(320, 200);
        for _ in 0..frames {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        canvas
    }

    /// Trail Length sets how long the streak behind a drop is.
    #[test]
    fn trail_length_changes_the_streak_behind_a_drop() {
        let short = mean_streak(&rain_with(2.0, 120.0, 90), DARK);
        let long = mean_streak(&rain_with(60.0, 120.0, 90), DARK);
        assert!(
            long > short * 2.0,
            "Trail Length 60 leaves a mean streak of {long:.1} cells behind a drop head, \
             not clearly longer than the {short:.1} cells at Trail Length 2"
        );
    }

    /// FNV-1a over the 8-bit frame, the precision the lights see.
    fn fingerprint(canvas: &Canvas) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for p in &canvas.pixels {
            for v in p {
                hash ^= (v.clamp(0.0, 1.0) * 255.0).round() as u64 & 0xff;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    /// The default Trail Length renders the frames the fixed 0.88 fade rendered.
    #[test]
    fn the_default_trail_length_keeps_the_old_frames() {
        // Recorded from the code before Trail Length reached the frame: a fixed 0.88
        // fade, seed 3, the schema defaults, 150 frames of 1/30 s on a 320x200 canvas.
        let recorded = 0xe660_d6a5_70f3_1204;
        assert_eq!(
            fingerprint(&rain_with(14.0, 120.0, 150)),
            recorded,
            "the default Trail Length no longer renders the fixed 0.88 fade's frames"
        );
        // The derived fade is the old constant itself at the defaults, not merely close.
        assert_eq!(per_frame_fade(DEFAULT_TRAIL_LENGTH, DEFAULT_STEP), DEFAULT_FADE);
    }

    /// The fade anchor is the schema's own default, so a changed default cannot leave
    /// the anchor behind.
    #[test]
    fn the_fade_anchor_is_the_schema_default() {
        let schema = Rain::schema();
        let trail = schema.properties.iter().find(|p| p.key == "trail_length").expect("trail_length");
        assert_eq!(trail.kind.default_value(), canvas_proto::PropValue::Number(14.0));
        assert_eq!(DEFAULT_TRAIL_LENGTH, 14.0);
        assert_eq!(DEFAULT_SPEED, 120.0);
    }

    /// A faster drop fades faster each frame, so its streak is the same length in cells.
    #[test]
    fn a_faster_drop_leaves_a_streak_of_the_same_length() {
        let cells = |trail: f32, step: f32| {
            let fade = per_frame_fade(trail, step);
            step * (DARK.ln() / fade.ln())
        };
        for trail in [2.0f32, 14.0, 60.0] {
            let slow = cells(trail, 1.0);
            let fast = cells(trail, 10.0);
            assert!((slow - fast).abs() < slow * 0.001, "trail {trail}: {slow} vs {fast}");
        }
    }

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
