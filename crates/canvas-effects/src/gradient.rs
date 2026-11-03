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
        // The scroll shifts the gradient along its own axis. The period is the mirrored
        // triangle wave below: the position runs 0..1..0, so a scrolling gradient runs
        // color1..color2..color1 with no jump anywhere, and the wrap point never lands
        // inside the canvas. The offset keeps its accumulated value when Scroll Speed
        // goes back to 0: at any constant offset the triangle position is continuous,
        // so the still gradient stays seamless, and resetting or unwinding it would
        // jump or animate. With offset 0 the position inside the canvas is exactly the
        // plain color1..color2 blend.
        let shift = (self.offset / 320.0).rem_euclid(2.0);
        let inv_span = 1.0 / span;
        let width = canvas.width as usize;
        for y in 0..canvas.height {
            let ly = (y as f32 + 0.5) / s;
            let row_t = ly * dy - lo;
            let out_row = &mut canvas.pixels[y as usize * width..(y as usize + 1) * width];
            for (x, out) in out_row.iter_mut().enumerate() {
                let lx = (x as f32 + 0.5) / s;
                // The canvas projection is in 0..1 and the shift in 0..2, so one
                // conditional wrap puts the position inside its 0..2 period. The
                // triangle wave reads 0 at the ends of the period and 1 in the
                // middle, which is continuous everywhere the sawtooth was not.
                let mut u = (lx * dx + row_t) * inv_span + shift;
                if u >= 2.0 {
                    u -= 2.0;
                }
                let t = 1.0 - (u - 1.0).abs();
                *out = mix(self.from, self.to, t);
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

    /// The biggest per-channel step a seamless blend allows between two pixels one
    /// render row apart, along the default angle's axis (90 °, so the axis runs down
    /// the 200 logical-unit height). On a 64x40 canvas (scale 0.2) neighbouring rows
    /// are 5 logical units apart, so the plain blend steps 5/200 = 0.025 of the full
    /// blend per row; the mirrored period at worst doubles that slope. A wrap seam
    /// instead jumps nearly the whole blend (about 0.85 on the green channel for the
    /// default colors), far above this bound.
    fn neighbour_step_bound(e: &Gradient, canvas: &Canvas) -> f32 {
        let span = 200.0; // the default angle's axis span, in logical units
        let step = (1.0 / canvas.scale()) / span;
        let range = (0..3).map(|i| (e.to[i] - e.from[i]).abs()).fold(0.0f32, f32::max);
        2.0 * step * range + 0.01
    }

    /// Fails on the first pair of vertically neighbouring pixels whose channels differ
    /// by more than `bound`.
    fn assert_no_seam(canvas: &Canvas, bound: f32, context: &str) {
        let w = canvas.width as usize;
        for y in 0..canvas.height as usize - 1 {
            for x in 0..w {
                let a = canvas.pixels[y * w + x];
                let b = canvas.pixels[(y + 1) * w + x];
                let d = (0..3).map(|i| (a[i] - b[i]).abs()).fold(0.0f32, f32::max);
                assert!(d <= bound, "{context}: rows {y}/{y1} at x {x} differ by {d} > {bound}", y1 = y + 1);
            }
        }
    }

    /// A scroll that stops leaves no seam: after the offset has scrolled in, Scroll
    /// Speed back to 0 still renders a seamless blend. 240 frames at 20 units/s puts
    /// the old wrap point exactly mid-canvas, where it freezes.
    #[test]
    fn a_scroll_that_stops_leaves_no_seam() {
        let mut e = Gradient::new();
        let mut props = e.schema.defaults();
        props.insert("scroll_speed".into(), canvas_proto::PropValue::Number(20.0));
        e.init(&props);
        let mut canvas = Canvas::new(64, 40);
        for _ in 0..240 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        }
        props.insert("scroll_speed".into(), canvas_proto::PropValue::Number(0.0));
        e.init(&props);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        let bound = neighbour_step_bound(&e, &canvas);
        assert_no_seam(&canvas, bound, "after scroll stops");
    }

    /// While scrolling, every frame is seamless. 960 frames at 20 units/s move the
    /// offset by 640 logical units, one full mirrored period, so the position sweeps
    /// the whole 0..2 range: up the blend, back down, and past the wrap point.
    #[test]
    fn the_gradient_blends_without_a_jump_while_scrolling() {
        let mut e = Gradient::new();
        let mut props = e.schema.defaults();
        props.insert("scroll_speed".into(), canvas_proto::PropValue::Number(20.0));
        e.init(&props);
        let mut canvas = Canvas::new(64, 40);
        for frame in 0..960 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            let bound = neighbour_step_bound(&e, &canvas);
            assert_no_seam(&canvas, bound, &format!("frame {frame}"));
        }
    }
}
