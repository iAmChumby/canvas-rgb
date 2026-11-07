//! The picture of a part: the crude drawing inside its own box.
//!
//! This is `app/web/src/views/ZoneArt.tsx` ported. The drawings are written
//! in a 100 x 100 space and stretched to the part's own box, the way the
//! React SVG does it (`viewBox="0 0 100 100"`, `preserveAspectRatio="none"`).
//! Every stroke keeps its own pixel width: React's
//! `vector-effect: non-scaling-stroke`.
//!
//! Only the strokes are drawn. The shapes React marks `artFill` (a fan's
//! hub, a stick of RAM's notch, a case's glass) are filled with the part's
//! own light, and at rest the part has no light: `--lit` is transparent, so
//! those fills are transparent in both modes and nothing of them is on the
//! screen. The same goes for the `artPanel` shapes (a card's fans, a CPU's
//! face): they are never lit.

use gpui::{PathBuilder, Window, point, px};

use canvas_proto::layout::{Facing, PartKind};

/// One stroke in the part's 100 x 100 space.
pub struct Stroke {
    /// The points of the line, in order.
    pub points: Vec<(f32, f32)>,
    /// True when the line closes on itself (a box, a ring).
    pub closed: bool,
}

impl Stroke {
    fn new(points: Vec<(f32, f32)>, closed: bool) -> Self {
        Stroke { points, closed }
    }
}

/// A straight line.
fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> Stroke {
    Stroke::new(vec![(x1, y1), (x2, y2)], false)
}

/// A box with rounded corners. The radius is a share of the 100 x 100 space,
/// and the box is stretched to the part, so the corners become ellipses.
fn rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Stroke {
    let mut points = Vec::with_capacity(40);
    let (x0, y0, x1, y1) = (x, y, x + w, y + h);
    let r = r.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        points.push((x0, y0));
        points.push((x1, y0));
        points.push((x1, y1));
        points.push((x0, y1));
        return Stroke::new(points, true);
    }
    // A quarter circle in eight steps: the line is one pixel wide, so the
    // corner reads the same as the curve React draws.
    let steps = 8;
    let corner = |cx: f32, cy: f32, from: f32, to: f32, points: &mut Vec<(f32, f32)>| {
        for i in 0..=steps {
            let a = from + (to - from) * (i as f32 / steps as f32);
            points.push((cx + r * a.cos(), cy + r * a.sin()));
        }
    };
    corner(x1 - r, y0 + r, -std::f32::consts::FRAC_PI_2, 0.0, &mut points);
    corner(x1 - r, y1 - r, 0.0, std::f32::consts::FRAC_PI_2, &mut points);
    corner(x0 + r, y1 - r, std::f32::consts::FRAC_PI_2, std::f32::consts::PI, &mut points);
    corner(x0 + r, y0 + r, std::f32::consts::PI, std::f32::consts::PI * 1.5, &mut points);
    Stroke::new(points, true)
}

/// A ring.
fn circle(cx: f32, cy: f32, r: f32) -> Stroke {
    let steps = 48;
    let mut points = Vec::with_capacity(steps);
    for i in 0..steps {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        points.push((cx + r * a.cos(), cy + r * a.sin()));
    }
    Stroke::new(points, true)
}

/// A closed path of straight corners.
fn poly(points: &[(f32, f32)]) -> Stroke {
    Stroke::new(points.to_vec(), true)
}

/// A curve: a start, a control point and an end (SVG's `Q`).
fn quad(x1: f32, y1: f32, cx: f32, cy: f32, x2: f32, y2: f32) -> Stroke {
    let steps = 16;
    let mut points = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let u = 1.0 - t;
        points.push((
            u * u * x1 + 2.0 * u * t * cx + t * t * x2,
            u * u * y1 + 2.0 * u * t * cy + t * t * y2,
        ));
    }
    Stroke::new(points, false)
}

/// A curve: a start, two control points and an end (SVG's `C`).
fn cubic(
    x1: f32,
    y1: f32,
    cax: f32,
    cay: f32,
    cbx: f32,
    cby: f32,
    x2: f32,
    y2: f32,
) -> Stroke {
    let steps = 20;
    let mut points = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let u = 1.0 - t;
        points.push((
            u * u * u * x1 + 3.0 * u * u * t * cax + 3.0 * u * t * t * cbx + t * t * t * x2,
            u * u * u * y1 + 3.0 * u * u * t * cay + 3.0 * u * t * t * cby + t * t * t * y2,
        ));
    }
    Stroke::new(points, false)
}

/// True when the part is seen from the side, lying flat: a fan pointing up or
/// down is a bar (`isFlat` in `views/editor/parts.ts`).
fn is_flat(facing: Facing) -> bool {
    matches!(facing, Facing::Up | Facing::Down)
}

/// The drawing of a part, in the 100 x 100 space.
pub fn art(kind: PartKind, facing: Facing) -> Vec<Stroke> {
    match kind {
        PartKind::Fan => {
            if is_flat(facing) {
                fan_flat()
            } else if matches!(facing, Facing::Left | Facing::Right) {
                fan_standing()
            } else {
                fan()
            }
        }
        PartKind::Ram => ram(),
        PartKind::Gpu => gpu(),
        PartKind::Cpu => cpu(),
        PartKind::Pump => pump(),
        PartKind::Radiator => radiator(),
        PartKind::Strip => strip(),
        PartKind::Keyboard => keyboard(),
        PartKind::Mouse => mouse(),
        PartKind::Microphone => microphone(),
        PartKind::Headset => headset(),
        PartKind::Case => case_box(),
        PartKind::Generic => generic(),
    }
}

/// A fan pointing at the reader: the bracket, its four holes and its ring.
fn fan() -> Vec<Stroke> {
    let mut out = vec![rect(4.0, 4.0, 92.0, 92.0, 6.0)];
    for (cx, cy) in [(13.0, 13.0), (87.0, 13.0), (13.0, 87.0), (87.0, 87.0)] {
        out.push(circle(cx, cy, 2.6));
    }
    out.push(circle(50.0, 50.0, 44.0));
    out
}

/// A fan seen from the side, lying flat: a bar with its LEDs along it.
fn fan_flat() -> Vec<Stroke> {
    let mut out = vec![rect(2.0, 28.0, 96.0, 44.0, 5.0), line(2.0, 50.0, 98.0, 50.0)];
    for x in [10.0, 22.0, 34.0, 46.0, 58.0, 70.0, 82.0, 92.0] {
        out.push(line(x, 34.0, x, 46.0));
    }
    out
}

/// A fan seen from the side, standing up.
fn fan_standing() -> Vec<Stroke> {
    let mut out = vec![rect(28.0, 2.0, 44.0, 96.0, 5.0), line(50.0, 2.0, 50.0, 98.0)];
    for y in [10.0, 22.0, 34.0, 46.0, 58.0, 70.0, 82.0, 92.0] {
        out.push(line(34.0, y, 46.0, y));
    }
    out
}

/// A stick of RAM: the slot it sits in, the notch in its foot and its LEDs.
fn ram() -> Vec<Stroke> {
    let mut out = vec![poly(&[
        (26.0, 2.0),
        (74.0, 2.0),
        (74.0, 88.0),
        (58.0, 88.0),
        (58.0, 96.0),
        (42.0, 96.0),
        (42.0, 88.0),
        (26.0, 88.0),
    ])];
    for y in [18.0, 30.0, 42.0, 54.0, 66.0, 78.0] {
        out.push(line(74.0, y, 88.0, y));
    }
    out
}

/// A graphics card: a long box.
fn gpu() -> Vec<Stroke> {
    vec![rect(2.0, 6.0, 96.0, 86.0, 4.0)]
}

/// A CPU: a square with a notch in one corner.
fn cpu() -> Vec<Stroke> {
    vec![poly(&[
        (24.0, 4.0),
        (96.0, 4.0),
        (96.0, 96.0),
        (4.0, 96.0),
        (4.0, 24.0),
    ])]
}

/// A pump head: the cap, the motor and its mount.
fn pump() -> Vec<Stroke> {
    vec![
        circle(50.0, 42.0, 32.0),
        poly(&[(44.0, 74.0), (56.0, 74.0), (56.0, 90.0), (44.0, 90.0)]),
        line(26.0, 90.0, 74.0, 90.0),
    ]
}

/// A radiator: a wide box with its fins.
fn radiator() -> Vec<Stroke> {
    let mut out = vec![rect(2.0, 14.0, 96.0, 72.0, 3.0)];
    for x in [12.0, 22.0, 32.0, 42.0, 52.0, 62.0, 72.0, 82.0, 92.0] {
        out.push(line(x, 20.0, x, 80.0));
    }
    out
}

/// A strip: a bar.
fn strip() -> Vec<Stroke> {
    vec![rect(1.0, 36.0, 98.0, 28.0, 3.0)]
}

/// A keyboard: a slab and the rows of keys.
fn keyboard() -> Vec<Stroke> {
    vec![
        rect(1.0, 8.0, 98.0, 84.0, 5.0),
        line(8.0, 30.0, 92.0, 30.0),
        line(8.0, 50.0, 92.0, 50.0),
        line(8.0, 70.0, 92.0, 70.0),
        line(8.0, 86.0, 92.0, 86.0),
    ]
}

/// A mouse: its shell and the split.
fn mouse() -> Vec<Stroke> {
    vec![
        rect(10.0, 2.0, 80.0, 96.0, 38.0),
        line(10.0, 42.0, 90.0, 42.0),
        line(50.0, 4.0, 50.0, 42.0),
    ]
}

/// A microphone: the capsule, its grille and the stand.
fn microphone() -> Vec<Stroke> {
    vec![
        rect(26.0, 2.0, 48.0, 48.0, 24.0),
        line(26.0, 18.0, 74.0, 18.0),
        line(26.0, 32.0, 74.0, 32.0),
        quad(14.0, 30.0, 14.0, 66.0, 50.0, 66.0),
        quad(50.0, 66.0, 86.0, 66.0, 86.0, 30.0),
        line(50.0, 66.0, 50.0, 90.0),
        line(26.0, 94.0, 74.0, 94.0),
    ]
}

/// A headset: the band and its two cups.
fn headset() -> Vec<Stroke> {
    vec![
        cubic(10.0, 56.0, 10.0, -6.0, 90.0, -6.0, 90.0, 56.0),
        rect(2.0, 46.0, 24.0, 46.0, 10.0),
        rect(74.0, 46.0, 24.0, 46.0, 10.0),
    ]
}

/// The case: the box.
fn case_box() -> Vec<Stroke> {
    vec![rect(3.0, 3.0, 94.0, 94.0, 6.0)]
}

/// A box: a plain rectangle.
fn generic() -> Vec<Stroke> {
    vec![rect(6.0, 6.0, 88.0, 88.0, 4.0)]
}

/// Paints a part's drawing.
///
/// `box` is the part's own box in canvas units, `origin` the desk's pixel
/// origin, `k` the pixels to a canvas unit, and `rotation` the part's turn
/// in degrees. `inset` is how far the drawing sits inside the part's border,
/// in css pixels: the browser draws the picture on the part's padding box.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    window: &mut Window,
    kind: PartKind,
    facing: Facing,
    box_x: f64,
    box_y: f64,
    box_w: f64,
    box_h: f64,
    rotation_deg: f64,
    origin_x: f32,
    origin_y: f32,
    k: f32,
    stroke: gpui::Rgba,
    inset: f32,
) {
    let strokes = art(kind, facing);
    if strokes.is_empty() {
        return;
    }
    // The drawing's own box: the part's box without its border.
    let inset_units = inset as f64 / k as f64;
    let bx = box_x + inset_units;
    let by = box_y + inset_units;
    let bw = (box_w - 2.0 * inset_units).max(0.0);
    let bh = (box_h - 2.0 * inset_units).max(0.0);
    // The part's centre, the point the drawing turns about.
    let cx = box_x + box_w / 2.0;
    let cy = box_y + box_h / 2.0;
    let rad = rotation_deg.to_radians();
    let (sin, cos) = rad.sin_cos();
    let mut builder = PathBuilder::stroke(px(1.0));
    for s in &strokes {
        for (i, (sx, sy)) in s.points.iter().enumerate() {
            // The 100 x 100 space, stretched to the drawing's own box.
            let ux = bx + (*sx as f64 / 100.0) * bw;
            let uy = by + (*sy as f64 / 100.0) * bh;
            // The part's turn, about its own centre.
            let dx = ux - cx;
            let dy = uy - cy;
            let rx = cx + dx * cos - dy * sin;
            let ry = cy + dx * sin + dy * cos;
            let px_x = origin_x + rx as f32 * k;
            let px_y = origin_y + ry as f32 * k;
            if i == 0 {
                builder.move_to(point(px(px_x), px(px_y)));
            } else {
                builder.line_to(point(px(px_x), px(px_y)));
            }
        }
        if s.closed {
            builder.close();
        }
    }
    let path = match builder.build() {
        Ok(path) => path,
        Err(_) => return,
    };
    window.paint_path(path, stroke);
}

/// The part's drawing, for the layout's own tests: how many strokes it has.
pub fn strokes_for(kind: PartKind, facing: Facing) -> usize {
    art(kind, facing).len()
}
