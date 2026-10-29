//! The two onboarding effects: `onboard-hello` and `onboard-sweep`.
//!
//! They are built-ins, but hidden: they are not in [`crate::registry`] and not in the
//! Effects list. [`crate::create`] and the daemon's schema lookup find them by id, so the
//! existing activate route runs them.
//!
//! - **hello:** one slow, soft wave of the brand colour across the whole canvas (about 2 s),
//!   then a calm glow that holds.
//! - **sweep:** a bright band crossing the canvas left to right once (about 2.5 s), then the
//!   canvas holds dim.
//!
//! Both run on their own clock, which `init` resets, so every activation plays from the start.

use crate::color::{mix, to_linear};
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, PropertySchema, Props, Rgb8};

/// The id of the hello effect.
pub const HELLO_ID: &str = "onboard-hello";
/// The id of the sweep effect.
pub const SWEEP_ID: &str = "onboard-sweep";

/// How long the hello wave takes to cross the canvas, in seconds.
const HELLO_SECONDS: f32 = 2.0;
/// The glow the hello settles on, 0..1.
const HELLO_GLOW: f32 = 0.35;
/// How long the sweep takes to cross the canvas, in seconds.
const SWEEP_SECONDS: f32 = 2.5;
/// The level the canvas holds at after the sweep, 0..1.
const SWEEP_HOLD: f32 = 0.12;
/// Half the sweep band's width, as a fraction of the canvas width.
const SWEEP_HALF_WIDTH: f32 = 0.08;

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The brand colour: the violet the other effects default to.
fn brand() -> Rgb8 {
    "#6000fc".parse().unwrap_or(Rgb8::BLACK)
}

/// True for the ids of the hidden onboarding effects.
pub fn is_onboard_id(id: &str) -> bool {
    id == HELLO_ID || id == SWEEP_ID
}

/// The schemas of the hidden effects (hello, then sweep).
pub fn hidden_schemas() -> Vec<PropertySchema> {
    vec![Hello::schema(), Sweep::schema()]
}

/// Builds a hidden effect by id, or `None`.
pub fn create_hidden(id: &str) -> Option<Box<dyn Effect>> {
    match id {
        HELLO_ID => Some(create_hello()),
        SWEEP_ID => Some(create_sweep()),
        _ => None,
    }
}

// ---------------------------------------------------------------- hello

/// The hello wave.
pub struct Hello {
    schema: PropertySchema,
    color: [f32; 3],
    time: f32,
}

impl Hello {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(HELLO_ID, "Hello", "One slow wave of colour, then a calm glow.")
            .color("color", "Color", "#6000fc")
            .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self { schema, color: to_linear(brand()), time: 0.0 };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for Hello {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory for `onboard-hello`.
pub fn create_hello() -> Box<dyn Effect> {
    Box::new(Hello::new())
}

impl Effect for Hello {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.color = to_linear(crate::props_util::color(&self.schema, props, "color"));
        self.time = 0.0;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if dt.is_finite() && dt > 0.0 {
            self.time += dt;
        }
        let width = canvas.width.max(1);
        // A soft hump of light travels left to right. It starts and ends past the edges,
        // so the whole canvas rises and falls; then everything settles on one glow.
        let progress = (self.time / HELLO_SECONDS).clamp(0.0, 1.0);
        let front = -0.4 + progress * 1.8;
        let settle = smoothstep((self.time - HELLO_SECONDS) / 0.8);
        for (i, px) in canvas.pixels.iter_mut().enumerate() {
            let x = (i as u32 % width) as f32 / width as f32;
            let hump = (-((x - front) / 0.28).powi(2)).exp();
            let level = (hump + (HELLO_GLOW - hump) * settle).clamp(0.0, 1.0);
            *px = [self.color[0] * level, self.color[1] * level, self.color[2] * level];
        }
    }
}

// ---------------------------------------------------------------- sweep

/// The sweep band.
pub struct Sweep {
    schema: PropertySchema,
    color: [f32; 3],
    time: f32,
}

impl Sweep {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(SWEEP_ID, "Sweep", "A bright band crosses once, then the canvas holds dim.")
            .color("color", "Color", "#6000fc")
            .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self { schema, color: to_linear(brand()), time: 0.0 };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for Sweep {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory for `onboard-sweep`.
pub fn create_sweep() -> Box<dyn Effect> {
    Box::new(Sweep::new())
}

impl Effect for Sweep {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.color = to_linear(crate::props_util::color(&self.schema, props, "color"));
        self.time = 0.0;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if dt.is_finite() && dt > 0.0 {
            self.time += dt;
        }
        let width = canvas.width.max(1);
        // The band starts fully off the left edge and ends fully off the right edge.
        let progress = (self.time / SWEEP_SECONDS).clamp(0.0, 1.0);
        let centre = -SWEEP_HALF_WIDTH + progress * (1.0 + 2.0 * SWEEP_HALF_WIDTH);
        let done = progress >= 1.0;
        for (i, px) in canvas.pixels.iter_mut().enumerate() {
            let x = (i as u32 % width) as f32 / width as f32;
            let band = if done {
                0.0
            } else {
                smoothstep(1.0 - (x - centre).abs() / SWEEP_HALF_WIDTH)
            };
            let level = SWEEP_HOLD.max(band);
            let base = [self.color[0] * level, self.color[1] * level, self.color[2] * level];
            // The band's core goes toward white; the hold stays in the brand colour.
            let c = mix(base, [1.0; 3], band * band * 0.6);
            *px = [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::Inputs;

    fn brightness(canvas: &Canvas, x: u32) -> f32 {
        let p = canvas.get(x, canvas.height / 2);
        p[0] + p[1] + p[2]
    }

    fn assert_in_range(canvas: &Canvas) {
        for p in &canvas.pixels {
            for v in p {
                assert!(v.is_finite() && (0.0..=1.0).contains(v), "out of range: {v:?}");
            }
        }
    }

    /// Plays an effect for `seconds` at 30 fps and checks every frame is in range.
    fn play(mut e: Box<dyn Effect>, seconds: f32, canvas: &mut Canvas) {
        e.init(&e.schema().defaults());
        for _ in 0..(seconds * 30.0) as u32 {
            e.update(1.0 / 30.0, canvas, &Inputs::default());
            assert_in_range(canvas);
        }
    }

    #[test]
    fn both_build_by_id_and_stay_out_of_the_visible_registry() {
        for id in [HELLO_ID, SWEEP_ID] {
            let e = create_hidden(id).expect("builds");
            assert_eq!(e.schema().id, id);
            assert!(is_onboard_id(id));
            assert!(crate::registry().iter().all(|f| f().schema().id != id));
            assert!(crate::create(id).is_some(), "create() finds {id}");
        }
        assert!(create_hidden("solid").is_none());
        assert_eq!(hidden_schemas().len(), 2);
    }

    #[test]
    fn hello_renders_moves_and_settles_to_an_even_glow() {
        let mut canvas = Canvas::new(64, 8);
        let mut e = create_hello();
        e.init(&e.schema().defaults());
        let (mut early_left, mut late_right) = (false, false);
        for frame in 0..60 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            assert_in_range(&canvas);
            if frame == 10 && brightness(&canvas, 8) > brightness(&canvas, 56) {
                early_left = true;
            }
            if frame == 50 && brightness(&canvas, 56) > brightness(&canvas, 8) {
                late_right = true;
            }
        }
        assert!(early_left, "the wave starts on the left");
        assert!(late_right, "the wave reaches the right");
        let mut after = Canvas::new(64, 8);
        play(create_hello(), 4.0, &mut after);
        let (a, b) = (brightness(&after, 5), brightness(&after, 58));
        assert!(a > 0.1, "a glow remains: {a}");
        assert!((a - b).abs() < 0.01, "the glow is even: {a} vs {b}");
    }

    #[test]
    fn sweep_renders_moves_once_and_holds_dim() {
        let mut canvas = Canvas::new(100, 4);
        let mut e = create_sweep();
        e.init(&e.schema().defaults());
        let mut peaks = Vec::new();
        for _ in 0..60 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            assert_in_range(&canvas);
            let peak = (0..100)
                .max_by(|a, b| brightness(&canvas, *a).total_cmp(&brightness(&canvas, *b)))
                .unwrap_or(0);
            peaks.push(peak);
        }
        assert!(peaks[10] < peaks[30] && peaks[30] < peaks[55], "the band moves right: {peaks:?}");
        let mut after = Canvas::new(100, 4);
        play(create_sweep(), 4.0, &mut after);
        let (a, b) = (brightness(&after, 3), brightness(&after, 96));
        assert!(a > 0.02 && a < 0.6, "holds dim: {a}");
        assert!((a - b).abs() < 0.001, "even hold: {a} vs {b}");
    }

    #[test]
    fn both_ignore_a_bad_dt() {
        for id in [HELLO_ID, SWEEP_ID] {
            let mut e = create_hidden(id).unwrap();
            e.init(&e.schema().defaults());
            let mut canvas = Canvas::new(16, 4);
            for dt in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
                e.update(dt, &mut canvas, &Inputs::default());
                assert_in_range(&canvas);
            }
        }
    }
}
