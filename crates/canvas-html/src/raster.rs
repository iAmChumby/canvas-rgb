//! A software Canvas 2D context backed by `tiny-skia`.
//!
//! The context keeps its own state stack (as the HTML spec requires) and rasterizes into
//! an RGBA8 premultiplied pixmap. Drawing is anti-aliased and source-over with straight
//! alpha, which is what a browser does, so the same script produces the same picture.
//!
//! The JS bindings in [`crate::js`] call into this module; nothing here knows about JS.

use crate::color::CssColor;
use tiny_skia::{
    BlendMode, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point,
    RadialGradient, Shader, SpreadMode, Stroke, Transform,
};

/// Default canvas size when the file declares none.
pub const DEFAULT_WIDTH: u32 = 320;
pub const DEFAULT_HEIGHT: u32 = 200;

/// A paint source: a solid color or a gradient.
#[derive(Debug, Clone)]
pub enum FillStyle {
    Solid(CssColor),
    Linear(LinearGradientSpec),
    Radial(RadialGradientSpec),
}

#[derive(Debug, Clone)]
pub struct LinearGradientSpec {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub stops: Vec<(f32, CssColor)>,
}

#[derive(Debug, Clone)]
pub struct RadialGradientSpec {
    pub x0: f32,
    pub y0: f32,
    pub r0: f32,
    pub x1: f32,
    pub y1: f32,
    pub r1: f32,
    pub stops: Vec<(f32, CssColor)>,
}

/// The composite operations this runtime supports. Anything else falls back to
/// source-over, which is what a browser does for an unsupported value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Composite {
    SourceOver,
    Lighter,
    Copy,
    Source,
    Destination,
    DestinationOver,
    SourceIn,
    DestinationIn,
    SourceOut,
    DestinationOut,
    SourceAtop,
    DestinationAtop,
    Xor,
}

impl Composite {
    /// Parses a `globalCompositeOperation` value. Unknown values keep source-over.
    pub fn parse(value: &str) -> Self {
        match value {
            "lighter" => Self::Lighter,
            "copy" => Self::Copy,
            "source" => Self::Source,
            "destination" => Self::Destination,
            "destination-over" => Self::DestinationOver,
            "source-in" => Self::SourceIn,
            "destination-in" => Self::DestinationIn,
            "source-out" => Self::SourceOut,
            "destination-out" => Self::DestinationOut,
            "source-atop" => Self::SourceAtop,
            "destination-atop" => Self::DestinationAtop,
            "xor" => Self::Xor,
            _ => Self::SourceOver,
        }
    }

    fn to_blend(self) -> BlendMode {
        match self {
            Self::SourceOver => BlendMode::SourceOver,
            Self::Lighter => BlendMode::Plus,
            Self::Copy => BlendMode::Source,
            Self::Source => BlendMode::Source,
            Self::Destination => BlendMode::Destination,
            Self::DestinationOver => BlendMode::DestinationOver,
            Self::SourceIn => BlendMode::SourceIn,
            Self::DestinationIn => BlendMode::DestinationIn,
            Self::SourceOut => BlendMode::SourceOut,
            Self::DestinationOut => BlendMode::DestinationOut,
            Self::SourceAtop => BlendMode::SourceAtop,
            Self::DestinationAtop => BlendMode::DestinationAtop,
            Self::Xor => BlendMode::Xor,
        }
    }
}

/// The drawing state, as the HTML spec defines it. `save`/`restore` push and pop these.
#[derive(Debug, Clone)]
pub struct State {
    pub fill_style: FillStyle,
    pub stroke_style: FillStyle,
    pub global_alpha: f32,
    pub line_width: f32,
    pub composite: Composite,
    /// The current transformation matrix.
    pub transform: Transform,
}

impl Default for State {
    fn default() -> Self {
        Self {
            // The spec's initial fillStyle/strokeStyle is "#000000".
            fill_style: FillStyle::Solid(CssColor::BLACK),
            stroke_style: FillStyle::Solid(CssColor::BLACK),
            global_alpha: 1.0,
            line_width: 1.0,
            composite: Composite::SourceOver,
            transform: Transform::identity(),
        }
    }
}

/// The rasterizer: a pixmap plus the state stack and the current path.
pub struct Context {
    pixmap: Pixmap,
    width: u32,
    height: u32,
    state: State,
    stack: Vec<State>,
    /// The current path, in user space.
    path: PathBuilder,
    /// The current path's "is empty" flag, tracked separately because `PathBuilder` has
    /// no way to ask.
    path_has_content: bool,
}

impl Context {
    /// Creates a context with a transparent-black backing store.
    pub fn new(width: u32, height: u32) -> Self {
        let mut pixmap = match Pixmap::new(width.max(1), height.max(1)) {
            Some(pixmap) => pixmap,
            // A size the pixmap cannot hold falls back to 1x1. The stored size follows the
            // pixmap, so the buffer and the size the caller reads always agree.
            None => Pixmap::new(1, 1).expect("a 1x1 pixmap always fits"),
        };
        pixmap.fill(tiny_skia::Color::TRANSPARENT);
        let (width, height) = (pixmap.width(), pixmap.height());
        Self {
            pixmap,
            width,
            height,
            state: State::default(),
            stack: Vec::new(),
            path: PathBuilder::new(),
            path_has_content: false,
        }
    }

    /// The canvas' width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The canvas' height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The RGBA8 pixels, row-major, 4 bytes per pixel.
    pub fn pixels(&self) -> &[u8] {
        self.pixmap.data()
    }

    // ------------------------------------------------------------ state

    pub fn save(&mut self) {
        self.stack.push(self.state.clone());
    }

    pub fn restore(&mut self) {
        if let Some(state) = self.stack.pop() {
            self.state = state;
        }
    }

    pub fn set_fill_style(&mut self, style: FillStyle) {
        self.state.fill_style = style;
    }

    pub fn set_stroke_style(&mut self, style: FillStyle) {
        self.state.stroke_style = style;
    }

    pub fn set_global_alpha(&mut self, alpha: f32) {
        self.state.global_alpha = alpha.clamp(0.0, 1.0);
    }

    pub fn set_line_width(&mut self, width: f32) {
        // A negative or non-finite width is ignored, as in a browser.
        if width.is_finite() && width >= 0.0 {
            self.state.line_width = width;
        }
    }

    pub fn set_composite(&mut self, composite: Composite) {
        self.state.composite = composite;
    }

    /// The current global alpha.
    pub fn global_alpha(&self) -> f32 {
        self.state.global_alpha
    }

    /// The current line width.
    pub fn line_width(&self) -> f32 {
        self.state.line_width
    }

    /// The current composite operation, as its CSS name.
    pub fn composite(&self) -> &'static str {
        match self.state.composite {
            Composite::SourceOver => "source-over",
            Composite::Lighter => "lighter",
            Composite::Copy => "copy",
            Composite::Source => "source",
            Composite::Destination => "destination",
            Composite::DestinationOver => "destination-over",
            Composite::SourceIn => "source-in",
            Composite::DestinationIn => "destination-in",
            Composite::SourceOut => "source-out",
            Composite::DestinationOut => "destination-out",
            Composite::SourceAtop => "source-atop",
            Composite::DestinationAtop => "destination-atop",
            Composite::Xor => "xor",
        }
    }

    pub fn translate(&mut self, x: f32, y: f32) {
        self.state.transform = self.state.transform.post_translate(x, y);
    }

    pub fn scale(&mut self, x: f32, y: f32) {
        self.state.transform = self.state.transform.post_scale(x, y);
    }

    pub fn rotate(&mut self, radians: f32) {
        self.state.transform = self.state.transform.post_rotate(radians);
    }

    /// `setTransform(a, b, c, d, e, f)` replaces the matrix outright.
    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        self.state.transform = Transform::from_row(a, b, c, d, e, f);
    }

    pub fn reset_transform(&mut self) {
        self.state.transform = Transform::identity();
    }

    // ------------------------------------------------------------ path

    pub fn begin_path(&mut self) {
        self.path = PathBuilder::new();
        self.path_has_content = false;
    }

    pub fn close_path(&mut self) {
        if self.path_has_content {
            self.path.close();
        }
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(x, y);
        self.path_has_content = true;
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        // A `lineTo` with no current point starts a new subpath, as in a browser.
        if !self.path_has_content {
            self.path.move_to(x, y);
        } else {
            self.path.line_to(x, y);
        }
        self.path_has_content = true;
    }

    pub fn quadratic_curve_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if !self.path_has_content {
            self.path.move_to(cx, cy);
        }
        self.path.quad_to(cx, cy, x, y);
        self.path_has_content = true;
    }

    pub fn bezier_curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        if !self.path_has_content {
            self.path.move_to(c1x, c1y);
        }
        self.path.cubic_to(c1x, c1y, c2x, c2y, x, y);
        self.path_has_content = true;
    }

    /// `arc(x, y, radius, start, end, counterclockwise)`. The path gets the arc's line
    /// segment; a full circle becomes four cubic segments, as a browser does.
    pub fn arc(
        &mut self,
        x: f32,
        y: f32,
        radius: f32,
        start: f32,
        end: f32,
        counterclockwise: bool,
    ) {
        if !radius.is_finite() || radius < 0.0 {
            return;
        }
        let mut sweep = end - start;
        if counterclockwise {
            if sweep > 0.0 {
                sweep -= std::f32::consts::TAU;
            }
        } else if sweep < 0.0 {
            sweep += std::f32::consts::TAU;
        }
        // A full turn (or more) is drawn as a closed circle.
        let full = sweep.abs() >= std::f32::consts::TAU - 1e-6;

        // The arc starts at its start point: a line joins it to the current point when the
        // path has one (as a browser does), otherwise the path starts there.
        let start_point = Point::from_xy(x + radius * start.cos(), y + radius * start.sin());
        if !self.path_has_content {
            self.path.move_to(start_point.x, start_point.y);
        } else {
            self.path.line_to(start_point.x, start_point.y);
        }
        self.path_has_content = true;

        if full {
            // Four quarter arcs as cubics, with the standard 0.5523 kappa factor.
            const K: f32 = 0.552_284_8;
            let mut angle = start;
            let step = sweep.signum() * std::f32::consts::FRAC_PI_2;
            for _ in 0..4 {
                let next = angle + step;
                let (sin_a, cos_a) = (angle.sin(), angle.cos());
                let (sin_b, cos_b) = (next.sin(), next.cos());
                let c1x = x + radius * (cos_a - K * sin_a);
                let c1y = y + radius * (sin_a + K * cos_a);
                let c2x = x + radius * (cos_b + K * sin_b);
                let c2y = y + radius * (sin_b - K * cos_b);
                let ex = x + radius * cos_b;
                let ey = y + radius * sin_b;
                self.path.cubic_to(c1x, c1y, c2x, c2y, ex, ey);
                angle = next;
            }
            self.path.close();
            return;
        }

        let segments = arc_segment_count(sweep.abs());
        let delta = sweep / segments as f32;
        let (mut sin_prev, mut cos_prev) = (start.sin(), start.cos());
        // One cubic per piece of arc; the control-point factor is the standard
        // arc-to-cubic approximation for a piece of `delta` radians.
        let alpha = (4.0 / 3.0) * (delta / 4.0).tan();
        for i in 1..=segments {
            let angle = start + delta * i as f32;
            let (sin_a, cos_a) = (angle.sin(), angle.cos());
            let c1x = x + radius * (cos_prev - alpha * sin_prev);
            let c1y = y + radius * (sin_prev + alpha * cos_prev);
            let c2x = x + radius * (cos_a + alpha * sin_a);
            let c2y = y + radius * (sin_a - alpha * cos_a);
            let ex = x + radius * cos_a;
            let ey = y + radius * sin_a;
            self.path.cubic_to(c1x, c1y, c2x, c2y, ex, ey);
            sin_prev = sin_a;
            cos_prev = cos_a;
        }
    }

    /// `ellipse(x, y, radiusX, radiusY, rotation, start, end, counterclockwise)`.
    // The arguments are the Canvas 2D `ellipse` signature, one per parameter.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipse(
        &mut self,
        x: f32,
        y: f32,
        radius_x: f32,
        radius_y: f32,
        rotation: f32,
        start: f32,
        end: f32,
        counterclockwise: bool,
    ) {
        if !radius_x.is_finite() || !radius_y.is_finite() || radius_x < 0.0 || radius_y < 0.0 {
            return;
        }
        // Build the ellipse in a unit space and map it with the transform.
        let (sin_r, cos_r) = (rotation.sin(), rotation.cos());
        let map = |ux: f32, uy: f32| -> (f32, f32) {
            let px = ux * radius_x;
            let py = uy * radius_y;
            (x + px * cos_r - py * sin_r, y + px * sin_r + py * cos_r)
        };
        let mut sweep = end - start;
        if counterclockwise {
            if sweep > 0.0 {
                sweep -= std::f32::consts::TAU;
            }
        } else if sweep < 0.0 {
            sweep += std::f32::consts::TAU;
        }
        let full = sweep.abs() >= std::f32::consts::TAU - 1e-6;
        let segments = if full { 4 } else { arc_segment_count(sweep.abs()) };
        let delta = sweep / segments as f32;

        for i in 0..segments {
            let a0 = start + delta * i as f32;
            let a1 = start + delta * (i + 1) as f32;
            let (sin_a, cos_a) = (a0.sin(), a0.cos());
            let (sin_b, cos_b) = (a1.sin(), a1.cos());
            // The cubic control-point factor for this arc segment.
            let alpha = (4.0 / 3.0) * (delta / 4.0).tan();
            let p0 = map(cos_a, sin_a);
            let c1 = map(cos_a - alpha * sin_a, sin_a + alpha * cos_a);
            let c2 = map(cos_b + alpha * sin_b, sin_b - alpha * cos_b);
            let p1 = map(cos_b, sin_b);
            if i == 0 && !self.path_has_content {
                self.path.move_to(p0.0, p0.1);
            }
            self.path.cubic_to(c1.0, c1.1, c2.0, c2.1, p1.0, p1.1);
        }
        self.path_has_content = true;
        if full {
            self.path.close();
        }
    }

    /// Adds a rectangle as a closed subpath.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if !self.path_has_content {
            self.path.move_to(x, y);
        } else {
            self.path.line_to(x, y);
        }
        self.path.line_to(x + w, y);
        self.path.line_to(x + w, y + h);
        self.path.line_to(x, y + h);
        self.path.close();
        self.path_has_content = true;
    }

    // ------------------------------------------------------------ painting

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        // A `fillRect` paints its own rectangle and leaves the current path untouched.
        let Some(rect) = normalized_rect(x, y, w, h) else {
            return;
        };
        let mut pb = PathBuilder::new();
        pb.push_rect(rect);
        let Some(path) = pb.finish() else {
            return;
        };
        let paint = self.make_paint(&self.state.fill_style.clone());
        self.pixmap.fill_path(&path, &paint, FillRule::Winding, self.state.transform, None);
    }

    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        // A stroked rect is four open lines, not a closed path (the corners are not
        // joined), which is why a browser strokes it as a separate path.
        let mut pb = PathBuilder::new();
        pb.move_to(x, y);
        pb.line_to(x + w, y);
        pb.line_to(x + w, y + h);
        pb.line_to(x, y + h);
        pb.line_to(x, y);
        if let Some(path) = pb.finish() {
            let paint = self.make_paint(&self.state.stroke_style.clone());
            let mut stroke = Stroke {
                width: self.state.line_width,
                ..Stroke::default()
            };
            // `Stroke::default` has a hairline width; a zero width means hairline.
            if self.state.line_width <= 0.0 {
                stroke.width = 0.0;
            }
            self.pixmap.stroke_path(&path, &paint, &stroke, self.state.transform, None);
        }
    }

    pub fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        // `clearRect` clears to transparent black regardless of the composite mode.
        let Some(rect) = normalized_rect(x, y, w, h) else {
            return;
        };
        let mut pb = PathBuilder::new();
        pb.push_rect(rect);
        if let Some(path) = pb.finish() {
            let paint = Paint {
                shader: Shader::SolidColor(tiny_skia::Color::TRANSPARENT),
                blend_mode: BlendMode::Source,
                anti_alias: false,
                force_hq_pipeline: false,
            };
            self.pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                self.state.transform,
                None,
            );
        }
    }

    /// Fills the current path.
    pub fn fill(&mut self) {
        // The builder is cloned so the path survives, as a browser's path does.
        let Some(path) = self.path.clone().finish() else {
            return;
        };
        let paint = self.make_paint(&self.state.fill_style.clone());
        self.pixmap.fill_path(&path, &paint, FillRule::Winding, self.state.transform, None);
    }

    /// Strokes the current path.
    pub fn stroke(&mut self) {
        let Some(path) = self.path.clone().finish() else {
            return;
        };
        let paint = self.make_paint(&self.state.stroke_style.clone());
        let mut stroke = Stroke {
            width: self.state.line_width,
            ..Stroke::default()
        };
        if self.state.line_width <= 0.0 {
            stroke.width = 0.0;
        }
        self.pixmap.stroke_path(&path, &paint, &stroke, self.state.transform, None);
    }

    /// Builds the `tiny-skia` paint for a style, honouring `globalAlpha`.
    fn make_paint(&self, style: &FillStyle) -> Paint<'static> {
        let alpha = self.state.global_alpha;
        let shader = match style {
            FillStyle::Solid(c) => Shader::SolidColor(to_skia_color(*c, alpha)),
            FillStyle::Linear(spec) => {
                let stops = gradient_stops(&spec.stops, alpha);
                match LinearGradient::new(
                    Point::from_xy(spec.x0, spec.y0),
                    Point::from_xy(spec.x1, spec.y1),
                    stops,
                    SpreadMode::Pad,
                    Transform::identity(),
                ) {
                    Some(s) => s,
                    None => Shader::SolidColor(tiny_skia::Color::TRANSPARENT),
                }
            }
            FillStyle::Radial(spec) => {
                let stops = gradient_stops(&spec.stops, alpha);
                match RadialGradient::new(
                    Point::from_xy(spec.x0, spec.y0),
                    Point::from_xy(spec.x1, spec.y1),
                    spec.r1,
                    stops,
                    SpreadMode::Pad,
                    Transform::identity(),
                ) {
                    Some(s) => s,
                    None => Shader::SolidColor(tiny_skia::Color::TRANSPARENT),
                }
            }
        };
        Paint {
            shader,
            blend_mode: self.state.composite.to_blend(),
            anti_alias: true,
            force_hq_pipeline: false,
        }
    }

    // ------------------------------------------------------------ image data

    /// `getImageData(x, y, w, h)`: RGBA bytes, straight (not premultiplied) alpha,
    /// row-major, with the canvas origin at (0, 0).
    pub fn get_image_data(&self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        let w = w.max(0) as u32;
        let h = h.max(0) as u32;
        let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
        let pixels = self.pixmap.pixels();
        for row in 0..h {
            let sy = y + row as i32;
            if sy < 0 || sy >= self.height as i32 {
                continue;
            }
            for col in 0..w {
                let sx = x + col as i32;
                if sx < 0 || sx >= self.width as i32 {
                    continue;
                }
                let src = &pixels[(sy as u32 * self.width + sx as u32) as usize];
                // `src` is premultiplied; demultiply to get straight alpha.
                let c = src.demultiply();
                let dst = &mut out[((row * w + col) as usize) * 4..][..4];
                dst[0] = c.red();
                dst[1] = c.green();
                dst[2] = c.blue();
                dst[3] = c.alpha();
            }
        }
        out
    }

    /// `putImageData(data, x, y)`: replaces pixels outright (no blending). The image's
    /// own width is needed to know the stride, so the caller supplies it.
    pub fn put_image_data_with_width(&mut self, data: &[u8], x: i32, y: i32, img_width: u32) {
        let img_width = img_width.max(1);
        let img_height = (data.len() / 4) as u32 / img_width;
        let pixels = self.pixmap.pixels_mut();
        for row in 0..img_height {
            let dy = y + row as i32;
            if dy < 0 || dy >= self.height as i32 {
                continue;
            }
            for col in 0..img_width {
                let dx = x + col as i32;
                if dx < 0 || dx >= self.width as i32 {
                    continue;
                }
                let si = ((row * img_width + col) as usize) * 4;
                if si + 3 >= data.len() {
                    continue;
                }
                // The image is straight alpha; the pixmap stores premultiplied values.
                let a = data[si + 3];
                let dst = &mut pixels[(dy as u32 * self.width + dx as u32) as usize];
                *dst = tiny_skia::PremultipliedColorU8::from_rgba(
                    premultiply(data[si], a),
                    premultiply(data[si + 1], a),
                    premultiply(data[si + 2], a),
                    a,
                )
                .unwrap_or(*dst);
            }
        }
    }
}

/// A rectangle from `(x, y, w, h)`, with a negative size flipped to a positive one as a
/// browser does. A zero-area rectangle is `None`: nothing is painted.
fn normalized_rect(x: f32, y: f32, w: f32, h: f32) -> Option<tiny_skia::Rect> {
    let (left, right) = if w >= 0.0 { (x, x + w) } else { (x + w, x) };
    let (top, bottom) = if h >= 0.0 { (y, y + h) } else { (y + h, y) };
    tiny_skia::Rect::from_ltrb(left, top, right, bottom)
}

/// One straight-alpha channel scaled by its alpha, rounded, for the premultiplied store.
fn premultiply(channel: u8, alpha: u8) -> u8 {
    ((channel as u32 * alpha as u32 + 127) / 255) as u8
}

/// Number of cubic segments used for an arc of `sweep` radians. Browsers use at most
/// four quarter-turns' worth for small arcs.
fn arc_segment_count(sweep: f32) -> u32 {
    let quarter = std::f32::consts::FRAC_PI_2;
    let n = (sweep / quarter).ceil();
    n.clamp(1.0, 8.0) as u32
}

/// Converts a CSS color to a `tiny-skia` color, applying `globalAlpha`.
fn to_skia_color(c: CssColor, alpha: f32) -> tiny_skia::Color {
    let a = (c.a * alpha).clamp(0.0, 1.0);
    tiny_skia::Color::from_rgba(c.r, c.g, c.b, a).unwrap_or(tiny_skia::Color::TRANSPARENT)
}

/// Gradient stops in `tiny-skia` form, with the global alpha folded into each color.
fn gradient_stops(stops: &[(f32, CssColor)], alpha: f32) -> Vec<GradientStop> {
    stops
        .iter()
        .map(|&(pos, c)| {
            let a = (c.a * alpha).clamp(0.0, 1.0);
            GradientStop::new(
                pos.clamp(0.0, 1.0),
                tiny_skia::Color::from_rgba(c.r, c.g, c.b, a)
                    .unwrap_or(tiny_skia::Color::TRANSPARENT),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_rect_makes_transparent() {
        let mut ctx = Context::new(4, 4);
        ctx.fill_rect(0.0, 0.0, 4.0, 4.0);
        assert!(ctx.pixels().iter().any(|&b| b != 0));
        ctx.clear_rect(0.0, 0.0, 4.0, 4.0);
        assert!(ctx.pixels().iter().all(|&b| b == 0));
    }

    #[test]
    fn fill_rect_writes_the_color() {
        let mut ctx = Context::new(4, 4);
        ctx.set_fill_style(FillStyle::Solid(CssColor::from_rgba8(255, 0, 0, 1.0)));
        ctx.fill_rect(0.0, 0.0, 4.0, 4.0);
        let px = ctx.pixels();
        assert_eq!(px[0], 255);
        assert_eq!(px[1], 0);
    }

    #[test]
    fn save_restore_restores_the_state() {
        let mut ctx = Context::new(4, 4);
        ctx.set_line_width(7.0);
        ctx.save();
        ctx.set_line_width(2.0);
        ctx.restore();
        assert_eq!(ctx.state.line_width, 7.0);
    }

    #[test]
    fn transform_translate_moves_the_fill() {
        let mut ctx = Context::new(8, 8);
        ctx.set_fill_style(FillStyle::Solid(CssColor::from_rgba8(255, 0, 0, 1.0)));
        ctx.translate(4.0, 4.0);
        ctx.fill_rect(0.0, 0.0, 2.0, 2.0);
        // The red pixels must not be in the top-left corner.
        let top_left_red = ctx.pixels()[0] == 255;
        assert!(!top_left_red);
    }

    #[test]
    fn get_image_data_returns_straight_alpha() {
        let mut ctx = Context::new(2, 2);
        ctx.set_fill_style(FillStyle::Solid(CssColor::from_rgba8(255, 0, 0, 0.5)));
        ctx.fill_rect(0.0, 0.0, 2.0, 2.0);
        let data = ctx.get_image_data(0, 0, 2, 2);
        assert_eq!(data.len(), 16);
        // Straight alpha: red is 255 even at half opacity.
        assert_eq!(data[0], 255);
        assert!((data[3] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn arc_draws_something() {
        let mut ctx = Context::new(32, 32);
        ctx.set_fill_style(FillStyle::Solid(CssColor::from_rgba8(255, 255, 255, 1.0)));
        ctx.begin_path();
        ctx.arc(16.0, 16.0, 8.0, 0.0, std::f32::consts::TAU, false);
        ctx.fill();
        assert!(ctx.pixels().iter().any(|&b| b != 0));
    }

    #[test]
    fn fill_rect_leaves_the_current_path_alone() {
        // A path with a triangle, then a fillRect elsewhere: the fill must paint only the
        // rectangle, not the triangle too.
        let mut ctx = Context::new(16, 16);
        ctx.set_fill_style(FillStyle::Solid(CssColor::from_rgba8(255, 0, 0, 1.0)));
        ctx.begin_path();
        ctx.move_to(0.0, 0.0);
        ctx.line_to(4.0, 0.0);
        ctx.line_to(0.0, 4.0);
        ctx.fill_rect(10.0, 10.0, 2.0, 2.0);
        // The triangle's interior is untouched.
        let inside_triangle = (16 + 1) * 4;
        assert_eq!(ctx.pixels()[inside_triangle + 3], 0);
        // The rectangle is painted.
        let inside_rect = (10usize * 16 + 10) * 4;
        assert_eq!(ctx.pixels()[inside_rect + 3], 255);
    }

    #[test]
    fn put_image_data_keeps_partial_alpha_as_premultiplied() {
        // A straight-alpha white pixel at alpha 128 must be stored as premultiplied 128.
        let mut ctx = Context::new(2, 2);
        ctx.put_image_data_with_width(&[255, 255, 255, 128], 0, 0, 1);
        let px = &ctx.pixels()[0..4];
        assert_eq!(px[3], 128);
        assert_eq!(px[0], 128);
    }

    #[test]
    fn a_stroked_arc_draws_something() {
        let mut ctx = Context::new(32, 32);
        ctx.set_stroke_style(FillStyle::Solid(CssColor::from_rgba8(255, 0, 0, 1.0)));
        ctx.set_line_width(3.0);
        ctx.begin_path();
        ctx.arc(16.0, 16.0, 8.0, 0.0, std::f32::consts::TAU, false);
        ctx.stroke();
        let painted = ctx.pixels().iter().filter(|&&b| b != 0).count();
        assert!(painted > 20, "only {painted} pixels were painted");
    }
}
