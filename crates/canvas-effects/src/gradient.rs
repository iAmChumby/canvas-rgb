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
        .angle("angle", "Angle", 90.0)
        .number("scroll_speed", "Scroll Speed", 0.0, 100.0, 1.0, 0.0)
        .preset(
            "Sunset",
            vec![
                ("color1", color_kind("#ff2d55")),
                ("color2", color_kind("#ffb703")),
                ("angle", angle_kind(120.0)),
            ],
        )
        .preset(
            "Ocean",
            vec![
                ("color1", color_kind("#0a0a4a")),
                ("color2", color_kind("#1e6fd9")),
                ("angle", angle_kind(90.0)),
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
        self.angle_deg = p::angle(&self.schema, props, "angle") as f32;
        self.scroll = p::number(&self.schema, props, "scroll_speed") as f32;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        // Wrapped to one mirrored period in logical units (2 shift turns x 320
        // logical units) so a long run cannot grind f32 precision down. 640 is an
        // exact number of periods, so the rendered position is unchanged.
        self.offset = (self.offset + self.scroll * dt).rem_euclid(640.0);

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
                // The exact mirrored form: below the period's middle the plain
                // blend, above it the same ramp mirrored. Written without abs so
                // offset 0 is bit-identical to the plain blend: `u - 1.0` loses
                // the low bits of a small u, and `1.0 - (u - 1.0).abs()` rounds
                // them away.
                let t = if u < 1.0 { u } else { 2.0 - u };
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

fn angle_kind(a: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Angle { default: a }
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
        props.insert("angle".into(), canvas_proto::PropValue::Angle(0.0));
        e.init(&props);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        assert_ne!(before, canvas.pixels);
    }

    #[test]
    fn the_angle_is_declared_as_a_direction() {
        let schema = Gradient::schema();
        let def = schema
            .properties
            .iter()
            .find(|p| p.key == "angle")
            .expect("the angle property");
        assert_eq!(def.kind, canvas_proto::PropKind::Angle { default: 90.0 });
        // The presets that set an angle hold angle values, not hues.
        for preset in &schema.presets {
            if let Some(v) = preset.props.get("angle") {
                assert!(
                    matches!(v, canvas_proto::PropValue::Angle(_)),
                    "preset {} holds {v:?}",
                    preset.name
                );
            }
        }
    }

    /// The angles the seam tests sweep. 0 and 180 run the gradient across the
    /// columns, 45 and 90 across the rows, so a seam on either axis can fail them.
    const SEAM_ANGLES: [f64; 4] = [0.0, 45.0, 90.0, 180.0];

    /// A Gradient with Scroll Speed 20 and the given angle.
    fn scrolling_gradient(angle: f64) -> Gradient {
        let mut e = Gradient::new();
        let mut props = e.schema.defaults();
        props.insert("scroll_speed".into(), canvas_proto::PropValue::Number(20.0));
        props.insert("angle".into(), canvas_proto::PropValue::Angle(angle));
        e.init(&props);
        e
    }

    /// The biggest per-channel step a seamless blend allows between two neighbouring
    /// pixels, on either canvas axis. The blend parameter is piecewise linear in the
    /// gradient position with slope 1 on every branch (the plain ramp and the
    /// mirrored half alike), so two neighbours differ by at most their position
    /// step: one render row or column is 1/scale logical units apart, the angle's
    /// axis span is 320*|dx| + 200*|dy| logical units, so the step is
    /// max(|dx|, |dy|) / (scale * span) of the full blend, plus one 8-bit level of
    /// rounding slack. A wrap seam instead jumps nearly the whole blend.
    fn neighbour_step_bound(e: &Gradient, angle: f64, canvas: &Canvas) -> f32 {
        let rad = (angle as f32).to_radians();
        let (dx, dy) = (rad.cos().abs(), rad.sin().abs());
        let span = 320.0 * dx + 200.0 * dy;
        let step = dx.max(dy) / (canvas.scale() * span);
        let range = (0..3).map(|i| (e.to[i] - e.from[i]).abs()).fold(0.0f32, f32::max);
        step * range + 1.0 / 255.0
    }

    /// Fails on the first pair of horizontally or vertically neighbouring pixels
    /// whose channels differ by more than `bound`.
    fn assert_no_seam(canvas: &Canvas, bound: f32, context: &str) {
        let w = canvas.width as usize;
        let h = canvas.height as usize;
        for y in 0..h {
            for x in 0..w {
                let a = canvas.pixels[y * w + x];
                if x + 1 < w {
                    let x1 = x + 1;
                    let b = canvas.pixels[y * w + x1];
                    let d = (0..3).map(|i| (a[i] - b[i]).abs()).fold(0.0f32, f32::max);
                    assert!(d <= bound, "{context}: row {y}, columns {x}/{x1} differ by {d} > {bound}");
                }
                if y + 1 < h {
                    let y1 = y + 1;
                    let b = canvas.pixels[y1 * w + x];
                    let d = (0..3).map(|i| (a[i] - b[i]).abs()).fold(0.0f32, f32::max);
                    assert!(d <= bound, "{context}: rows {y}/{y1} at column {x} differ by {d} > {bound}");
                }
            }
        }
    }

    /// A scroll that stops leaves no seam at any angle: after the offset has
    /// scrolled in, Scroll Speed back to 0 still renders a seamless blend.
    /// 240 frames at 20 units/s put the old wrap point exactly mid-canvas,
    /// where it freezes.
    #[test]
    fn a_scroll_that_stops_leaves_no_seam_at_every_angle() {
        for &angle in &SEAM_ANGLES {
            let mut e = scrolling_gradient(angle);
            let mut canvas = Canvas::new(64, 40);
            for _ in 0..240 {
                e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            }
            let mut props = e.schema.defaults();
            props.insert("scroll_speed".into(), canvas_proto::PropValue::Number(0.0));
            props.insert("angle".into(), canvas_proto::PropValue::Angle(angle));
            e.init(&props);
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            let bound = neighbour_step_bound(&e, angle, &canvas);
            assert_no_seam(&canvas, bound, &format!("angle {angle}: after scroll stops"));
        }
    }

    /// While scrolling, every frame is seamless at every angle. 960 frames at
    /// 20 units/s move the offset by 640 logical units, one full mirrored period,
    /// so the position sweeps the whole 0..2 range: up the blend, back down, and
    /// past the wrap point.
    #[test]
    fn the_gradient_blends_without_a_jump_while_scrolling() {
        for &angle in &SEAM_ANGLES {
            let mut e = scrolling_gradient(angle);
            let mut canvas = Canvas::new(64, 40);
            for frame in 0..960 {
                e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
                let bound = neighbour_step_bound(&e, angle, &canvas);
                assert_no_seam(&canvas, bound, &format!("angle {angle}, frame {frame}"));
            }
        }
    }

    /// The offset wraps every 640 logical units (one mirrored period), so a long
    /// run cannot grind f32 precision down. The wrap is phase-exact, so the blend
    /// stays seamless across it: 2880 frames at 20 units/s run 1920 logical units,
    /// three periods, crossing the wrap twice.
    #[test]
    fn the_offset_wrap_stays_seamless() {
        let mut e = scrolling_gradient(90.0);
        let mut canvas = Canvas::new(64, 40);
        for frame in 0..2880 {
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            let bound = neighbour_step_bound(&e, 90.0, &canvas);
            assert_no_seam(&canvas, bound, &format!("wrap, frame {frame}"));
        }
    }

    /// The plain blend the old code rendered for a still gradient: the projection
    /// clamped to 0..1, mixed between the colors. The same corner order and the
    /// same expressions as `update`, so the comparison is bit-for-bit.
    fn old_still_blend(e: &Gradient, angle: f64, canvas: &mut Canvas) {
        let rad = (angle as f32).to_radians();
        let (dx, dy) = (rad.cos(), rad.sin());
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
        let inv_span = 1.0 / span;
        let width = canvas.width as usize;
        for y in 0..canvas.height {
            let ly = (y as f32 + 0.5) / s;
            let row_t = ly * dy - lo;
            let out_row = &mut canvas.pixels[y as usize * width..(y as usize + 1) * width];
            for (x, out) in out_row.iter_mut().enumerate() {
                let lx = (x as f32 + 0.5) / s;
                let t = (lx * dx + row_t) * inv_span;
                *out = mix(e.from, e.to, t.clamp(0.0, 1.0));
            }
        }
    }

    /// A still gradient (offset 0, Scroll Speed 0) is exactly the plain
    /// color1..color2 blend the old code rendered, at every angle: every rendered
    /// pixel equals the old formula's pixel.
    #[test]
    fn a_still_gradient_is_pixel_identical_to_the_plain_blend_at_every_angle() {
        for &angle in &[0.0f64, 45.0, 90.0, 135.0, 180.0, 270.0, 359.0] {
            let mut e = Gradient::new();
            let mut props = e.schema.defaults();
            props.insert("angle".into(), canvas_proto::PropValue::Angle(angle));
            e.init(&props);
            let mut canvas = Canvas::new(64, 40);
            e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            let mut expected = Canvas::new(64, 40);
            old_still_blend(&e, angle, &mut expected);
            for (i, (got, want)) in canvas.pixels.iter().zip(expected.pixels.iter()).enumerate() {
                assert_eq!(got, want, "angle {angle}: pixel {i}: got {got:?}, want {want:?}");
            }
        }
    }

    /// A gradient saved before the angle kind existed stored its angle as a hue: it loads
    /// with the angle it was saved with, not the default.
    #[test]
    fn a_saved_hue_angle_loads_as_the_same_direction() {
        let mut e = Gradient::new();
        assert_eq!(e.angle_deg, 90.0, "the default");
        let mut props = e.schema.defaults();
        props.insert("angle".into(), canvas_proto::PropValue::Hue(120.0));
        e.init(&props);
        assert_eq!(e.angle_deg, 120.0);
    }

    #[test]
    fn an_angle_prop_and_the_hue_it_replaced_render_the_same() {
        let mut e = Gradient::new();
        let mut via_angle = Canvas::new(64, 40);
        let mut props = e.schema.defaults();
        props.insert("angle".into(), canvas_proto::PropValue::Angle(30.0));
        e.init(&props);
        e.update(1.0 / 30.0, &mut via_angle, &Inputs::default());
        let mut via_hue = Canvas::new(64, 40);
        let mut props = e.schema.defaults();
        props.insert("angle".into(), canvas_proto::PropValue::Hue(30.0));
        e.init(&props);
        e.update(1.0 / 30.0, &mut via_hue, &Inputs::default());
        assert_eq!(via_angle.pixels, via_hue.pixels);
    }
}
