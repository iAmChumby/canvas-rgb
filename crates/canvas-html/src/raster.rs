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
    PremultipliedColorU8, RadialGradient, Shader, SpreadMode, Stroke, Transform,
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
    /// The device pixmap's size. It follows the render scale; see `set_device_size`.
    width: u32,
    height: u32,
    /// The canvas size the script sees (the file's `<canvas>` size). Coordinates the script
    /// uses, and the image-data calls, are in these units.
    logical_width: u32,
    logical_height: u32,
    state: State,
    stack: Vec<State>,
    /// The current path, in user space.
    path: PathBuilder,
    /// The current path's "is empty" flag, tracked separately because `PathBuilder` has
    /// no way to ask.
    path_has_content: bool,
    /// The centre and radius of the current path when it is exactly one full-turn `arc`
    /// (in user space). A stroke of such a path takes the annulus fast path.
    circle: Option<(f32, f32, f32)>,
    /// Whether a stroke of a full circle may take the annulus fast path. On by default;
    /// the tests turn it off to compare against tiny-skia's stroker.
    pub fast_annulus: bool,
    /// How many strokes took the annulus fast path.
    pub fast_strokes: u64,
}

impl Context {
    /// Creates a context with a transparent-black backing store, one device pixel per
    /// logical pixel.
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_logical(width, height, width, height)
    }

    /// Creates a context whose script-visible canvas is `logical_width` x `logical_height`,
    /// backed by a device pixmap of `device_width` x `device_height`. Every user transform
    /// sits on top of the scale between the two.
    pub fn with_logical(
        logical_width: u32,
        logical_height: u32,
        device_width: u32,
        device_height: u32,
    ) -> Self {
        let mut pixmap = match Pixmap::new(device_width.max(1), device_height.max(1)) {
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
            logical_width: logical_width.max(1),
            logical_height: logical_height.max(1),
            state: State::default(),
            stack: Vec::new(),
            path: PathBuilder::new(),
            path_has_content: false,
            circle: None,
            fast_annulus: true,
            fast_strokes: 0,
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

    /// Device pixels per logical pixel, horizontally.
    pub fn scale_x(&self) -> f32 {
        self.width as f32 / self.logical_width as f32
    }

    /// Device pixels per logical pixel, vertically.
    pub fn scale_y(&self) -> f32 {
        self.height as f32 / self.logical_height as f32
    }

    /// Resizes the device pixmap, keeping the drawing: the old pixels are resampled
    /// (nearest) into the new size, so a trail survives a change of render scale. A size
    /// the pixmap cannot hold leaves the context as it was.
    pub fn set_device_size(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        let Some(mut next) = Pixmap::new(width, height) else {
            return;
        };
        let (old_w, old_h) = (self.width as usize, self.height as usize);
        let (new_w, new_h) = (width as usize, height as usize);
        let old = self.pixmap.pixels();
        let new = next.pixels_mut();
        for y in 0..new_h {
            let sy = (y * old_h / new_h).min(old_h - 1);
            for x in 0..new_w {
                let sx = (x * old_w / new_w).min(old_w - 1);
                new[y * new_w + x] = old[sy * old_w + sx];
            }
        }
        self.pixmap = next;
        self.width = width;
        self.height = height;
    }

    /// The transform from the script's user space to device pixels: the current user
    /// transform, then the render scale. The scale sits under every user transform, so
    /// `setTransform` and `resetTransform` keep it.
    fn device_transform(&self) -> Transform {
        let (sx, sy) = (self.scale_x(), self.scale_y());
        let t = self.state.transform;
        Transform::from_row(sx * t.sx, sy * t.ky, sx * t.kx, sy * t.sy, sx * t.tx, sy * t.ty)
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
        self.circle = None;
    }

    pub fn close_path(&mut self) {
        if self.path_has_content {
            self.path.close();
        }
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        self.circle = None;
        self.path.move_to(x, y);
        self.path_has_content = true;
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        self.circle = None;
        // A `lineTo` with no current point starts a new subpath, as in a browser.
        if !self.path_has_content {
            self.path.move_to(x, y);
        } else {
            self.path.line_to(x, y);
        }
        self.path_has_content = true;
    }

    pub fn quadratic_curve_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.circle = None;
        if !self.path_has_content {
            self.path.move_to(cx, cy);
        }
        self.path.quad_to(cx, cy, x, y);
        self.path_has_content = true;
    }

    pub fn bezier_curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.circle = None;
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
        // The path is exactly this circle only when the arc is a full turn on an empty path.
        self.circle = if full && !self.path_has_content {
            Some((x, y, radius))
        } else {
            None
        };

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
        self.circle = None;
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
        self.circle = None;
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
        // A solid fill of whole device pixels is fully covered inside, so it is written pixel
        // by pixel in the browser's arithmetic (this is the full-canvas background every frame).
        if let FillStyle::Solid(c) = self.state.fill_style {
            if self.state.composite == Composite::SourceOver {
                if let Some((x0, y0, x1, y1)) = self.device_pixel_rect(&rect) {
                    let src = SolidSource::new(c, self.state.global_alpha);
                    let (width, height) = (self.width as i32, self.height as i32);
                    let pixels = self.pixmap.pixels_mut();
                    for py in y0.max(0)..y1.min(height) {
                        for px in x0.max(0)..x1.min(width) {
                            blend_skia(&mut pixels[(py * width + px) as usize], src, 255);
                        }
                    }
                    return;
                }
            }
        }
        let mut pb = PathBuilder::new();
        pb.push_rect(rect);
        let Some(path) = pb.finish() else {
            return;
        };
        let paint = self.make_paint(&self.state.fill_style.clone());
        { let t = self.device_transform(); self.pixmap.fill_path(&path, &paint, FillRule::Winding, t, None); }
    }

    /// The device pixel rectangle `[x0, x1) x [y0, y1)` a user-space rectangle covers, when
    /// its edges land exactly on whole device pixels under the device transform (no skew).
    fn device_pixel_rect(&self, rect: &tiny_skia::Rect) -> Option<(i32, i32, i32, i32)> {
        let t = self.device_transform();
        if t.kx != 0.0 || t.ky != 0.0 {
            return None;
        }
        let (a, b) = (t.sx * rect.left() + t.tx, t.sx * rect.right() + t.tx);
        let (c, d) = (t.sy * rect.top() + t.ty, t.sy * rect.bottom() + t.ty);
        let whole = |v: f32| {
            let r = v.round();
            ((v - r).abs() < 1e-3).then_some(r as i32)
        };
        Some((
            whole(a.min(b))?,
            whole(c.min(d))?,
            whole(a.max(b))?,
            whole(c.max(d))?,
        ))
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
            { let t = self.device_transform(); self.pixmap.stroke_path(&path, &paint, &stroke, t, None); }
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
            let t = self.device_transform();
            self.pixmap.fill_path(&path, &paint, FillRule::Winding, t, None);
        }
    }

    /// Fills the current path.
    pub fn fill(&mut self) {
        // The builder is cloned so the path survives, as a browser's path does.
        let Some(path) = self.path.clone().finish() else {
            return;
        };
        let paint = self.make_paint(&self.state.fill_style.clone());
        { let t = self.device_transform(); self.pixmap.fill_path(&path, &paint, FillRule::Winding, t, None); }
    }

    /// Strokes the current path.
    pub fn stroke(&mut self) {
        if let Some((cx, cy, r)) = self.circle {
            if self.stroke_circle_fast(cx, cy, r) {
                return;
            }
        }
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
        { let t = self.device_transform(); self.pixmap.stroke_path(&path, &paint, &stroke, t, None); }
    }

    /// Strokes the current full circle as an anti-aliased annulus, without tiny-skia's
    /// stroker. Returns `false`, painting nothing, when the stroke needs the general path:
    /// the fast path is off, the composite is not source-over, the style is a gradient,
    /// the device transform is not a uniform scale plus translation (a skew or two
    /// different axis scales turns the circle into an ellipse), or the width or radius is
    /// zero.
    fn stroke_circle_fast(&mut self, cx: f32, cy: f32, r: f32) -> bool {
        if !self.fast_annulus
            || self.state.composite != Composite::SourceOver
            || self.state.line_width.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            || r.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            || !r.is_finite()
        {
            return false;
        }
        let t = self.device_transform();
        if t.kx != 0.0 || t.ky != 0.0 || t.sx != t.sy || t.sx.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return false;
        }
        // A circle whose radius is under half the line width is stroked with a hole in the
        // middle. The browser draws that hole (its stroker and tiny-skia's agree), so such
        // circles take the general stroker rather than the annulus.
        if r * t.sx < self.state.line_width * t.sx * 0.5 {
            return false;
        }
        let FillStyle::Solid(c) = &self.state.stroke_style else {
            return false;
        };
        let c = *c;
        self.fast_strokes += 1;
        let src = SolidSource::new(c, self.state.global_alpha);
        // With a uniform scale the centre maps by the translation part, the radius and the
        // width by the scale.
        let scale = t.sx;
        let half = self.state.line_width * scale * 0.5;
        paint_annulus(
            &mut self.pixmap,
            cx * scale + t.tx,
            cy * scale + t.ty,
            (r * scale - half).max(0.0),
            r * scale + half,
            src,
        );
        true
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

    /// `getImageData(x, y, w, h)` in the script's (logical) pixels: RGBA bytes, straight
    /// (not premultiplied) alpha, row-major. Each logical pixel reads the device pixel
    /// under its centre, so the call works at any render scale.
    pub fn get_image_data(&self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        let w = w.max(0) as u32;
        let h = h.max(0) as u32;
        let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
        let pixels = self.pixmap.pixels();
        let (sx, sy) = (self.scale_x(), self.scale_y());
        for row in 0..h {
            let dy = ((y as f32 + row as f32 + 0.5) * sy).floor() as i32;
            if dy < 0 || dy >= self.height as i32 {
                continue;
            }
            for col in 0..w {
                let dx = ((x as f32 + col as f32 + 0.5) * sx).floor() as i32;
                if dx < 0 || dx >= self.width as i32 {
                    continue;
                }
                let src = &pixels[(dy as u32 * self.width + dx as u32) as usize];
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

    /// `putImageData(data, x, y)` in the script's (logical) pixels: replaces the pixels
    /// outright (no blending). Each image pixel fills every device pixel its logical pixel
    /// covers, so a canvas rendered at 2x or 4x is filled whole, not dotted. The image's own
    /// width is needed to know the stride, so the caller supplies it.
    pub fn put_image_data_with_width(&mut self, data: &[u8], x: i32, y: i32, img_width: u32) {
        let img_width = img_width.max(1);
        let img_height = (data.len() / 4) as u32 / img_width;
        let (sx, sy) = (self.scale_x(), self.scale_y());
        let (dev_w, dev_h) = (self.width as i32, self.height as i32);
        let width = self.width;
        let pixels = self.pixmap.pixels_mut();
        // The device span of logical pixel `i` is [floor(i * s), floor((i + 1) * s)), at least one.
        let span = |i: i32, s: f32| -> (i32, i32) {
            let lo = (i as f32 * s).floor() as i32;
            let hi = (((i + 1) as f32 * s).floor() as i32).max(lo + 1);
            (lo, hi)
        };
        for row in 0..img_height {
            let (y0, y1) = span(y + row as i32, sy);
            let (y0, y1) = (y0.max(0), y1.min(dev_h));
            if y0 >= y1 {
                continue;
            }
            for col in 0..img_width {
                let (x0, x1) = span(x + col as i32, sx);
                let (x0, x1) = (x0.max(0), x1.min(dev_w));
                if x0 >= x1 {
                    continue;
                }
                let si = ((row * img_width + col) as usize) * 4;
                if si + 3 >= data.len() {
                    continue;
                }
                // The image is straight alpha; the pixmap stores premultiplied values.
                let a = data[si + 3];
                let Some(value) = tiny_skia::PremultipliedColorU8::from_rgba(
                    premultiply(data[si], a),
                    premultiply(data[si + 1], a),
                    premultiply(data[si + 2], a),
                    a,
                ) else {
                    continue;
                };
                for dy in y0..y1 {
                    let base = (dy as u32 * width) as usize;
                    for dx in x0..x1 {
                        pixels[base + dx as usize] = value;
                    }
                }
            }
        }
    }
}

/// How far the band's coverage can reach past an edge: half a pixel diagonal.
const EDGE_MARGIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Source-over paint of an anti-aliased annulus into a premultiplied pixmap.
///
/// Coverage is the band's overlap with each pixel, approximated as a linear ramp one pixel
/// wide at each edge: 1 inside the band, 0 outside. Pixels whose centres sit at least half
/// a pixel inside both edges are full coverage, and that test uses squared distances, so
/// only edge pixels take a square root. Each row visits only the columns the band can
/// touch, and skips the hole the band surrounds.
fn paint_annulus(
    pixmap: &mut Pixmap,
    cx: f32,
    cy: f32,
    inner: f32,
    outer: f32,
    src: SolidSource,
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let pixels = pixmap.pixels_mut();

    // Pixel centres within half a pixel of an edge can still have coverage.
    let r_out = outer + EDGE_MARGIN;
    let r_in = inner - EDGE_MARGIN;
    let hole = r_in > 0.0;
    let ro2 = r_out * r_out;
    let ri2 = r_in * r_in;

    let y0 = ((cy - r_out).floor() as i32).max(0);
    let y1 = ((cy + r_out).ceil() as i32).min(height - 1);
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        let dy2 = dy * dy;
        if dy2 >= ro2 {
            continue;
        }
        let xo = (ro2 - dy2).sqrt();
        // The spans of this row that the band can touch: the whole outer span, or the two
        // sides of the hole when the row crosses it.
        let mut spans = [(0.0f32, 0.0f32); 2];
        let mut count = 1;
        spans[0] = (cx - xo, cx + xo);
        if hole && dy2 < ri2 {
            let xi = (ri2 - dy2).sqrt();
            spans[0] = (cx - xo, cx - xi);
            spans[1] = (cx + xi, cx + xo);
            count = 2;
        }
        // The margins of the two spans can overlap when the hole is narrow; a pixel is
        // visited once, so each span starts after the columns already visited.
        let mut next_free = 0i32;
        for &(lo, hi) in &spans[..count] {
            let x0 = (((lo - 1.0).floor() as i32).max(0)).max(next_free);
            let x1 = ((hi + 1.0).ceil() as i32).min(width - 1);
            next_free = x1 + 1;
            for px in x0..=x1 {
                let dx = px as f32 + 0.5 - cx;
                let d2 = dx * dx + dy2;
                if d2 >= ro2 || (hole && d2 <= ri2) {
                    continue;
                }
                let coverage = pixel_band_coverage(px as f32, py as f32, cx, cy, inner, outer);
                let cov8 = (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
                if cov8 == 0 {
                    continue;
                }
                blend_skia(&mut pixels[(py * width + px) as usize], src, cov8);
            }
        }
    }
}

/// Slices per edge pixel for the coverage integral.
const COVERAGE_SLICES: usize = 8;

/// The exact area of the pixel `[x0, x0+1] x [y0, y0+1]` covered by the annulus of radii
/// `[inner, outer]` centred on `(cx, cy)`, as a fraction in 0..1.
///
/// A pixel whose nearest corner is outside the band, or whose farthest corner is inside the
/// hole, is decided with squared distances. Any other pixel is integrated over x in
/// `COVERAGE_SLICES` slices: within a slice the band is two y-intervals, measured exactly.
fn pixel_band_coverage(x0: f32, y0: f32, cx: f32, cy: f32, inner: f32, outer: f32) -> f32 {
    let x1 = x0 + 1.0;
    let y1 = y0 + 1.0;
    let ro2 = outer * outer;
    let ri2 = inner * inner;
    let nx = cx.clamp(x0, x1);
    let ny = cy.clamp(y0, y1);
    let near2 = (nx - cx) * (nx - cx) + (ny - cy) * (ny - cy);
    let fx = if cx >= (x0 + x1) * 0.5 { x0 } else { x1 };
    let fy = if cy >= (y0 + y1) * 0.5 { y0 } else { y1 };
    let far2 = (fx - cx) * (fx - cx) + (fy - cy) * (fy - cy);
    if near2 >= ro2 || far2 <= ri2 {
        return 0.0;
    }
    if far2 <= ro2 && near2 >= ri2 {
        return 1.0;
    }
    let mut total = 0.0;
    for k in 0..COVERAGE_SLICES {
        let x = x0 + (k as f32 + 0.5) / COVERAGE_SLICES as f32;
        let u = (x - cx) * (x - cx);
        if u >= ro2 {
            continue;
        }
        let yo = (ro2 - u).sqrt();
        let yi = if u < ri2 { (ri2 - u).sqrt() } else { 0.0 };
        total += interval_overlap(cy + yi, cy + yo, y0, y1) + interval_overlap(cy - yo, cy - yi, y0, y1);
    }
    total / COVERAGE_SLICES as f32
}

/// The length of `[a, b]` inside `[c, d]`.
fn interval_overlap(a: f32, b: f32, c: f32, d: f32) -> f32 {
    (b.min(d) - a.max(c)).max(0.0)
}

/// Skia's 8-bit `SkMulDiv255Round`: `a * b / 255`, rounded.
fn mul_div255_round(a: u32, b: u32) -> u32 {
    let prod = a * b + 128;
    (prod + (prod >> 8)) >> 8
}

/// A solid colour as the browser's canvas stores it: its alpha byte (with `globalAlpha`
/// applied) and its colour premultiplied by that alpha, both rounded to bytes.
#[derive(Debug, Clone, Copy)]
struct SolidSource {
    alpha: u8,
    pm: [u8; 3],
}

impl SolidSource {
    fn new(c: CssColor, global_alpha: f32) -> Self {
        let alpha = (c.a * global_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u32;
        let pm = [byte(c.r), byte(c.g), byte(c.b)]
            .map(|v| mul_div255_round(v, alpha as u32).min(255) as u8);
        Self { alpha, pm }
    }
}

/// Source-over of a solid colour onto one premultiplied pixel, with coverage `cov` (0..=255),
/// in Skia's 8-bit arithmetic: the source is scaled by its coverage as Skia's blitter scales
/// it (`v * (cov + 1) >> 8`), then `dst' = src + (dst * (256 - src_a)) >> 8`. A browser's
/// canvas stores pixels this way, so a repeated fill settles where the browser's does.
fn blend_skia(dst: &mut PremultipliedColorU8, src: SolidSource, cov: u8) {
    let scale = cov as u32 + 1;
    let src_a = (src.alpha as u32 * scale) >> 8;
    if src_a == 0 {
        return;
    }
    let keep = 256 - src_a;
    let mix = |s: u8, d: u8| (((s as u32 * scale) >> 8) + ((d as u32 * keep) >> 8)).min(255) as u8;
    let a = (src_a + ((dst.alpha() as u32 * keep) >> 8)).min(255) as u8;
    let r = mix(src.pm[0], dst.red());
    let g = mix(src.pm[1], dst.green());
    let b = mix(src.pm[2], dst.blue());
    if let Some(pixel) = PremultipliedColorU8::from_rgba(r.min(a), g.min(a), b.min(a), a) {
        *dst = pixel;
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
    fn put_image_data_fills_the_whole_device_block_at_a_larger_render_scale() {
        // A 2x2 logical canvas at 8x8 device pixels: each image pixel covers a 4x4 block.
        let mut ctx = Context::new(2, 2);
        ctx.set_device_size(8, 8);
        ctx.put_image_data_with_width(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255], 0, 0, 2);
        let px = ctx.pixels();
        // The byte of channel `c` at device pixel (x, y).
        let at = |x: usize, y: usize, c: usize| px[(y * 8 + x) * 4 + c];
        assert_eq!((at(0, 0, 0), at(3, 3, 0)), (255, 255), "the red block is whole");
        assert_eq!((at(4, 0, 1), at(7, 3, 1)), (255, 255), "the green block is whole");
        assert_eq!((at(0, 4, 2), at(3, 7, 2)), (255, 255), "the blue block is whole");
        assert_eq!(at(5, 5, 0), 255, "the white block is whole");
        assert_eq!(at(4, 0, 0), 0, "the green block holds no red");
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

    /// The exact area fraction of pixel `(x0, y0)` inside the annulus, by dense point
    /// sampling. It is the reference the browser-like output is measured against.
    fn exact_area(x0: usize, y0: usize, cx: f32, cy: f32, inner: f32, outer: f32) -> f32 {
        const N: usize = 64;
        let (ri2, ro2) = (inner * inner, outer * outer);
        let mut hit = 0usize;
        for j in 0..N {
            let y = y0 as f32 + (j as f32 + 0.5) / N as f32;
            for i in 0..N {
                let x = x0 as f32 + (i as f32 + 0.5) / N as f32;
                let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
                if d2 >= ri2 && d2 < ro2 {
                    hit += 1;
                }
            }
        }
        hit as f32 / (N * N) as f32
    }

    /// The annulus fast path against an exact-area reference and against tiny-skia's
    /// stroker. Full circles with several radii, widths 1..11, colour alpha 0x88, centres
    /// partly off the canvas. Each circle is drawn on a fresh canvas by each path.
    /// Circles with radius under half the width are the whole disc (every point within
    /// w/2 of the path), checked separately.
    #[test]
    fn annulus_fast_path_matches_exact_coverage() {
        const W: usize = 48;
        let centres = [(10.3f32, 12.7f32), (-4.0, 20.0), (44.5, 46.0), (24.0, 0.0)];
        let radii = [1.5f32, 3.0, 5.0, 8.5, 12.0, 16.0, 21.0];
        let widths = [1.0f32, 2.0, 3.0, 4.0, 5.0, 7.0, 9.0, 11.0];
        let color = [255.0f32, 64.0, 128.0];
        let alpha = 136.0f32 / 255.0;

        let mut circles = 0u64;
        let mut discs = 0u64;
        // Per comparison: bytes within 2 levels, total bytes, worst difference.
        let mut fast_vs_exact = (0usize, 0usize, 0i32);
        let mut tiny_vs_exact = (0usize, 0usize, 0i32);
        let mut fast_vs_tiny = (0usize, 0usize, 0i32);
        let tally = |acc: &mut (usize, usize, i32), a: u8, b: u8| {
            let d = (a as i32 - b as i32).abs();
            acc.1 += 1;
            if d <= 2 {
                acc.0 += 1;
            }
            acc.2 = acc.2.max(d);
        };

        for &(cx, cy) in &centres {
            for &r in &radii {
                for &w in &widths {
                    let mut fast = Context::new(W as u32, W as u32);
                    let mut slow = Context::new(W as u32, W as u32);
                    slow.fast_annulus = false;
                    for ctx in [&mut fast, &mut slow] {
                        ctx.set_stroke_style(FillStyle::Solid(CssColor::from_rgba8(
                            255, 64, 128, alpha,
                        )));
                        ctx.set_line_width(w);
                        ctx.begin_path();
                        ctx.arc(cx, cy, r, 0.0, std::f32::consts::TAU, false);
                        ctx.stroke();
                    }
                    circles += 1;
                    assert_eq!(slow.fast_strokes, 0);
                    // A radius under half the width leaves a hole, which the general stroker
                    // draws (as the browser does), so the fast path declines those circles.
                    if r < w / 2.0 {
                        assert_eq!(fast.fast_strokes, 0, "a holed circle must not take the fast path");
                        discs += 1;
                        continue;
                    }
                    assert_eq!(fast.fast_strokes, 1, "the fast path must run for a full circle");

                    let (inner, outer) = ((r - w / 2.0).max(0.0), r + w / 2.0);
                    for y in 0..W {
                        for x in 0..W {
                            let cov = exact_area(x, y, cx, cy, inner, outer);
                            let i = (y * W + x) * 4;
                            let reference = [
                                (color[0] * alpha * cov).round() as u8,
                                (color[1] * alpha * cov).round() as u8,
                                (color[2] * alpha * cov).round() as u8,
                                (alpha * 255.0 * cov).round() as u8,
                            ];
                            for (k, &expected) in reference.iter().enumerate() {
                                let f = fast.pixels()[i + k];
                                let s = slow.pixels()[i + k];
                                tally(&mut fast_vs_exact, f, expected);
                                tally(&mut tiny_vs_exact, s, expected);
                                tally(&mut fast_vs_tiny, f, s);
                            }
                        }
                    }
                }
            }
        }

        let report = |name: &str, acc: (usize, usize, i32)| {
            let pct = acc.0 as f64 / acc.1 as f64 * 100.0;
            eprintln!("{name}: {:.3}% of {} bytes within 2 levels, worst {}", pct, acc.1, acc.2);
            pct
        };
        eprintln!("annulus: {circles} circles, {discs} disc cases checked separately");
        let fe = report("fast vs exact area", fast_vs_exact);
        report("tiny-skia vs exact area", tiny_vs_exact);
        report("fast vs tiny-skia", fast_vs_tiny);

        assert!(fe >= 99.0, "fast vs exact: only {fe:.3}% of bytes within 2 levels");
        assert!(fast_vs_exact.2 <= 16, "fast vs exact: worst byte differs by {}", fast_vs_exact.2);
    }

    #[test]
    fn the_render_scale_sits_under_the_user_transform() {
        // An 8x8 logical canvas at 4x4 device pixels: a logical fillRect(0,0,4,4) covers
        // device pixels (0,0) and (1,1), and nothing beyond.
        let mut ctx = Context::with_logical(8, 8, 4, 4);
        ctx.set_fill_style(FillStyle::Solid(CssColor::from_rgba8(255, 0, 0, 1.0)));
        ctx.fill_rect(0.0, 0.0, 4.0, 4.0);
        let alpha = |ctx: &Context, x: usize, y: usize| ctx.pixels()[(y * ctx.width() as usize + x) * 4 + 3];
        assert_eq!(alpha(&ctx, 1, 1), 255);
        assert_eq!(alpha(&ctx, 2, 2), 0);

        // getImageData takes logical coordinates: logical (0,0) is opaque, (4,4) is clear.
        let data = ctx.get_image_data(0, 0, 8, 8);
        assert_eq!(data[3], 255);
        assert_eq!(data[((4 * 8 + 4) * 4) + 3], 0);

        // A device resize keeps the drawing: 4 -> 8 doubles each device pixel, so the old
        // (1,1) block now covers device (2..3, 2..3).
        ctx.set_device_size(8, 8);
        assert_eq!(ctx.width(), 8);
        assert_eq!(alpha(&ctx, 3, 3), 255);
        assert_eq!(alpha(&ctx, 4, 4), 0);
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
