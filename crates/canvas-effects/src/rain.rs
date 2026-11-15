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

/// The fade per frame at the default Trail Length on the anchor frame: the fixed fade Rain
/// always had, at every Speed.
const DEFAULT_FADE: f32 = 0.88;
/// The Trail Length default. The fade above is measured at it, so it must stay the
/// schema's default (`the_fade_anchor_is_the_schema_default`).
const DEFAULT_TRAIL_LENGTH: f32 = 14.0;
/// The frame interval the fade anchor is measured at: the engine's default 30 frames a
/// second, which is also the frame every effect test drives the effects with.
const ANCHOR_FRAME_S: f32 = 1.0 / 30.0;
/// How the setting bends into the fade. A straight line would make the top third of the
/// control one solid column (at the default Speed, 60 would ask for a streak longer than the
/// 200-cell canvas); with this curve the streak at 60 is about nine tenths of the canvas and
/// 2 is still a short dash, so every part of the control looks different from its neighbour.
const TRAIL_CURVE: f32 = 0.69;

/// The fade for one frame of `dt` seconds at a Trail Length.
///
/// Trail Length sets how long a drop's light lingers. The streak that leaves depends on the
/// drop's Speed as it always did (a faster drop draws a longer streak in the same time), so
/// a scene or a preset saved before the control worked keeps its picture: at the default
/// Trail Length on the anchor frame the exponent is exactly 1 and the fade is `DEFAULT_FADE`
/// whatever the Speed. The frame interval is in the exponent, so the light lingers the same
/// time at 30 and at 60 frames a second.
fn per_frame_fade(trail_length: f32, dt: f32) -> f32 {
    let trail = trail_length.max(1.0);
    let frames = if dt.is_finite() { (dt / ANCHOR_FRAME_S).max(0.0) } else { 0.0 };
    let exponent = frames * (DEFAULT_TRAIL_LENGTH / trail).powf(TRAIL_CURVE);
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
        let fade = per_frame_fade(self.trail_length, dt);
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
        assert_eq!(per_frame_fade(DEFAULT_TRAIL_LENGTH, ANCHOR_FRAME_S), DEFAULT_FADE);
    }

    /// The fade anchor is the schema's own default, so a changed default cannot leave
    /// the anchor behind.
    #[test]
    fn the_fade_anchor_is_the_schema_default() {
        let schema = Rain::schema();
        let trail = schema.properties.iter().find(|p| p.key == "trail_length").expect("trail_length");
        assert_eq!(trail.kind.default_value(), canvas_proto::PropValue::Number(14.0));
        assert_eq!(DEFAULT_TRAIL_LENGTH, 14.0);
    }

    /// The streak behind a drop moving `step` cells a frame, in cells, from the fade alone.
    fn streak_cells(trail: f32, step: f32, dt: f32) -> f32 {
        step * (DARK.ln() / per_frame_fade(trail, dt).ln())
    }

    /// A scene saved at the default Trail Length keeps its picture at every Speed: the fade
    /// does not read the Speed, so it is the old constant for a slow drizzle and a storm.
    #[test]
    fn a_saved_scene_at_the_default_trail_length_keeps_its_fade_at_every_speed() {
        for speed in [60.0f32, 120.0, 220.0, 247.0] {
            let mut e = Rain::with_seed(3);
            let mut props = e.schema.defaults();
            props.insert("speed".into(), canvas_proto::PropValue::Number(f64::from(speed)));
            e.init(&props);
            assert_eq!(per_frame_fade(e.trail_length, ANCHOR_FRAME_S), DEFAULT_FADE, "speed {speed}");
        }
    }

    /// The light lingers the same time whatever the frame rate: two frames of 1/60 s fade as
    /// much as one of 1/30 s.
    #[test]
    fn the_trail_lasts_the_same_time_at_30_and_60_frames_a_second() {
        for trail in [2.0f32, 14.0, 60.0] {
            let one = per_frame_fade(trail, 1.0 / 30.0);
            let two = per_frame_fade(trail, 1.0 / 60.0).powi(2);
            assert!((one - two).abs() < 1e-5, "trail {trail}: {one} vs {two}");
        }
    }

    /// Every part of the control looks different from its neighbour, and the top of it still
    /// fits the canvas: at the default Speed the streak grows by at least a fifth from one
    /// stop to the next and stays shorter than the 200 cells the canvas is tall.
    #[test]
    fn every_part_of_the_control_changes_the_streak_and_the_top_fits_the_canvas() {
        let step = 120.0 * ANCHOR_FRAME_S;
        let stops = [2.0f32, 14.0, 30.0, 45.0, 60.0];
        let cells: Vec<f32> = stops.iter().map(|t| streak_cells(*t, step, ANCHOR_FRAME_S)).collect();
        for pair in cells.windows(2) {
            assert!(pair[1] > pair[0] * 1.2, "the streak grows too little: {cells:?}");
        }
        assert!(cells[4] < 200.0, "Trail Length 60 fills the whole column: {cells:?}");
        assert!(cells[0] > 8.0, "Trail Length 2 leaves no streak: {cells:?}");
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
