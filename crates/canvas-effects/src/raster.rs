//! Private raster helpers: alpha compositing, anti-aliased circle strokes, lines and
//! rects, all drawing in *logical* canvas units (320x200) and scaling to the render
//! pixels by [`Canvas::scale`].
//!
//! Compositing is `dst = dst * (1 - a) + src * a` per channel, which matches a browser
//! 2D canvas closely enough. No gamma games, no per-frame allocations.

use crate::color::Rgb;
use canvas_proto::Canvas;

/// Source-over composite of one pixel.
#[inline]
pub fn blend(dst: &mut Rgb, src: Rgb, a: f32) {
    if a <= 0.0 {
        return;
    }
    if a >= 1.0 {
        *dst = src;
        return;
    }
    let inv = 1.0 - a;
    dst[0] = dst[0] * inv + src[0] * a;
    dst[1] = dst[1] * inv + src[1] * a;
    dst[2] = dst[2] * inv + src[2] * a;
}

/// Fills the whole canvas with `color` at alpha `a`.
pub fn fill_alpha(canvas: &mut Canvas, color: Rgb, a: f32) {
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    let (r, g, b) = (color[0] * a, color[1] * a, color[2] * a);
    for p in canvas.pixels.iter_mut() {
        p[0] = p[0] * inv + r;
        p[1] = p[1] * inv + g;
        p[2] = p[2] * inv + b;
    }
}

/// Overwrites the whole canvas with `color` (no blending).
pub fn fill(canvas: &mut Canvas, color: Rgb) {
    canvas.pixels.fill(color);
}

/// Anti-aliased filled circle centered at logical `(cx, cy)` with logical radius `r`.
///
/// Coverage is estimated from the pixel's distance to the center against the pixel
/// radius, which gives a one-pixel smooth edge.
pub fn fill_circle(canvas: &mut Canvas, cx: f32, cy: f32, r: f32, color: Rgb, alpha: f32) {
    if r <= 0.0 || alpha <= 0.0 {
        return;
    }
    let s = canvas.scale();
    let px = cx * s;
    let py = cy * s;
    let pr = r * s;
    if pr <= 0.0 {
        return;
    }
    // Pixel radius for coverage: half the diagonal, so the edge is smooth at any scale.
    let edge = std::f32::consts::FRAC_1_SQRT_2 * 0.5;
    // The clip bounds are computed as i64 and clipped to the canvas before the cast,
    // so an off-canvas center can never wrap around into a huge unsigned range.
    let x0 = ((px - pr).floor() as i64).clamp(0, canvas.width as i64);
    let y0 = ((py - pr).floor() as i64).clamp(0, canvas.height as i64);
    let x1 = ((px + pr).ceil() as i64 + 1).clamp(0, canvas.width as i64);
    let y1 = ((py + pr).ceil() as i64 + 1).clamp(0, canvas.height as i64);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        let fy = (y as f32 + 0.5 - py) / pr;
        for x in x0..x1 {
            let fx = (x as f32 + 0.5 - px) / pr;
            let d = (fx * fx + fy * fy).sqrt();
            let cov = (1.0 - (d - (1.0 - edge)) / edge).clamp(0.0, 1.0);
            if cov > 0.0 {
                let i = (y as usize) * canvas.width as usize + x as usize;
                blend(&mut canvas.pixels[i], color, cov * alpha);
            }
        }
    }
}

/// Anti-aliased circle outline (a stroke) centered at logical `(cx, cy)` with logical
/// radius `r` and logical line width `w`.
///
/// The stroke covers distances from `r - w/2` to `r + w/2` from the center, clamped at
/// zero inside, which is what a browser's `ctx.arc` + `ctx.stroke` does.
///
/// Each row only visits the columns that actually intersect the annulus, so the cost
/// scales with the ring's circumference rather than its bounding box.
pub fn stroke_circle(canvas: &mut Canvas, cx: f32, cy: f32, r: f32, w: f32, color: Rgb, alpha: f32) {
    if w <= 0.0 || alpha <= 0.0 {
        return;
    }
    let s = canvas.scale();
    let px = cx * s;
    let py = cy * s;
    let pr = r * s;
    let pw = w * s;
    if pr < 0.0 || pw <= 0.0 {
        return;
    }
    let half = pw * 0.5;
    // The stroke spans distances [inner, outer] from the center.
    let outer = pr + half;
    let inner = (pr - half).max(0.0);
    // The half-diagonal of a pixel: how far a pixel reaches in the radial direction.
    let half_diag = std::f32::consts::FRAC_1_SQRT_2 * 0.5;

    let y0 = ((py - outer).floor() as i64).max(0);
    let y1 = ((py + outer).ceil() as i64 + 1).min(canvas.height as i64);
    if y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        let fy = y as f32 + 0.5 - py;
        let dy2 = fy * fy;
        // The row misses the ring entirely when |dy| is beyond the outer radius.
        if dy2 > outer * outer {
            continue;
        }
        // The x distances where the row crosses the outer and inner circles.
        let x_outer = (outer * outer - dy2).max(0.0).sqrt();
        let x_inner = if dy2 < inner * inner {
            (inner * inner - dy2).max(0.0).sqrt()
        } else {
            0.0
        };
        // Two spans per row: [px - x_outer, px - x_inner] and [px + x_inner, px + x_outer].
        let span_a = (px - x_outer, px - x_inner);
        let span_b = (px + x_inner, px + x_outer);
        for (lo, hi) in [span_a, span_b] {
            if hi < lo {
                continue;
            }
            let x_start = (lo - half_diag).floor() as i64;
            let x_end = (hi + half_diag).ceil() as i64 + 1;
            let xs = x_start.max(0);
            let xe = x_end.min(canvas.width as i64);
            for x in xs..xe {
                let fx = x as f32 + 0.5 - px;
                let d = (fx * fx + dy2).sqrt();
                // Coverage of the annulus [inner, outer].
                //
                // A pixel spans roughly `half_diag` in the radial direction, so the
                // part of it inside the band is the overlap of the pixel's radial
                // extent with the band. That gives a smooth edge and keeps the
                // stroke's total coverage close to a browser's, which computes the
                // same overlap exactly.
                let lo = (d - half_diag).max(inner);
                let hi = (d + half_diag).min(outer);
                let cov = if hi <= lo {
                    0.0
                } else {
                    ((hi - lo) / (2.0 * half_diag)).clamp(0.0, 1.0)
                };
                if cov > 0.0 {
                    let i = (y as usize) * canvas.width as usize + x as usize;
                    blend(&mut canvas.pixels[i], color, cov * alpha);
                }
            }
        }
    }
}

/// A stroke's colour and alpha.
#[derive(Debug, Clone, Copy)]
pub struct Paint {
    pub color: Rgb,
    pub alpha: f32,
}

impl Paint {
    pub fn new(color: Rgb, alpha: f32) -> Self {
        Self { color, alpha }
    }
}

/// Anti-aliased line from logical `(x0, y0)` to `(x1, y1)` with logical width `w`.
///
/// The line is walked pixel by pixel along its length, so the cost scales with the
/// line's length rather than its bounding box.
pub fn stroke_line(
    canvas: &mut Canvas,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    w: f32,
    paint: Paint,
) {
    if w <= 0.0 || paint.alpha <= 0.0 {
        return;
    }
    let color = paint.color;
    let alpha = paint.alpha;
    let s = canvas.scale();
    let (ax, ay) = (x0 * s, y0 * s);
    let (bx, by) = (x1 * s, y1 * s);
    let pw = w * s;
    let half = pw * 0.5;
    if !ax.is_finite() || !ay.is_finite() || !bx.is_finite() || !by.is_finite() {
        return;
    }
    let dx = bx - ax;
    let dy = by - ay;
    let len = (dx * dx + dy * dy).sqrt();
    if !len.is_finite() {
        return;
    }
    let edge = std::f32::consts::FRAC_1_SQRT_2 * 0.5;
    // One pixel of softening around the line.
    let half_soft = half + edge;
    // The number of pixels along the line.
    let steps = (len + half_soft * 2.0).ceil().max(1.0) as usize;
    if steps > 8192 {
        // A pathological line: clip it rather than stall the render loop.
        return;
    }
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let cx = ax + dx * t;
        let cy = ay + dy * t;
        let x_start = (cx - half_soft).floor() as i64;
        let x_end = (cx + half_soft).ceil() as i64 + 1;
        let y_start = (cy - half_soft).floor() as i64;
        let y_end = (cy + half_soft).ceil() as i64 + 1;
        let xs = x_start.max(0);
        let xe = x_end.min(canvas.width as i64);
        let ys = y_start.max(0);
        let ye = y_end.min(canvas.height as i64);
        for y in ys..ye {
            for x in xs..xe {
                let px = x as f32 + 0.5 - cx;
                let py = y as f32 + 0.5 - cy;
                let d = (px * px + py * py).sqrt();
                let cov = (1.0 - (d - half) / edge).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let i = (y as usize) * canvas.width as usize + x as usize;
                    blend(&mut canvas.pixels[i], color, cov * alpha);
                }
            }
        }
    }
}

/// Anti-aliased filled rectangle in logical coordinates.
pub fn fill_rect(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32, color: Rgb, alpha: f32) {
    if w <= 0.0 || h <= 0.0 || alpha <= 0.0 {
        return;
    }
    let s = canvas.scale();
    let (x0, y0) = (x * s, y * s);
    let (x1, y1) = (x0 + w * s, y0 + h * s);
    let ix0 = (x0.floor() as i64).clamp(0, canvas.width as i64);
    let iy0 = (y0.floor() as i64).clamp(0, canvas.height as i64);
    let ix1 = (x1.ceil() as i64 + 1).clamp(0, canvas.width as i64);
    let iy1 = (y1.ceil() as i64 + 1).clamp(0, canvas.height as i64);
    if ix1 <= ix0 || iy1 <= iy0 {
        return;
    }
    for y in iy0..iy1 {
        for x in ix0..ix1 {
            // Coverage of the pixel by the rect.
            let cx = (x as f32 + 0.5).clamp(x0, x1);
            let cy = (y as f32 + 0.5).clamp(y0, y1);
            let cov_x = (cx - x as f32).clamp(0.0, 1.0);
            let cov_y = (cy - y as f32).clamp(0.0, 1.0);
            let cov = cov_x * cov_y;
            if cov > 0.0 {
                let i = (y as usize) * canvas.width as usize + x as usize;
                blend(&mut canvas.pixels[i], color, cov * alpha);
            }
        }
    }
}

/// The source-to-target mapping for an area-average downscale.
///
/// Built once per (source size, target size) pair and reused every frame, so the
/// hot path does not allocate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaMap {
    /// The first source column of each target column.
    col_start: Vec<u32>,
    /// The first source row of each target row.
    row_start: Vec<u32>,
    src_w: u32,
    src_h: u32,
    target_w: u32,
    target_h: u32,
}

impl AreaMap {
    /// Builds the mapping from a `src_w` by `src_h` source to a `target_w` by
    /// `target_h` target.
    pub fn new(src_w: u32, src_h: u32, target_w: u32, target_h: u32) -> Self {
        let col_start = (0..target_w)
            .map(|x| (x as u64 * src_w as u64 / target_w.max(1) as u64) as u32)
            .collect();
        let row_start = (0..target_h)
            .map(|y| (y as u64 * src_h as u64 / target_h.max(1) as u64) as u32)
            .collect();
        Self { col_start, row_start, src_w, src_h, target_w, target_h }
    }

    /// True when this map still fits the given sizes.
    pub fn fits(&self, src_w: u32, src_h: u32, target_w: u32, target_h: u32) -> bool {
        self.src_w == src_w && self.src_h == src_h && self.target_w == target_w && self.target_h == target_h
    }
}

/// Area-average downscale of an RGB8 image into a canvas-sized linear RGB buffer.
///
/// The source is assumed to cover the whole target: each target pixel averages the
/// source pixels whose centers fall inside it.
pub fn blit_area_average(
    canvas: &mut Canvas,
    src_w: u32,
    src_h: u32,
    src: &[canvas_proto::Rgb8],
) {
    let map = AreaMap::new(src_w, src_h, canvas.width, canvas.height);
    blit_area_average_with(canvas, src_w, src_h, src, &map);
}

/// Area-average downscale using a mapping the caller built and caches.
pub fn blit_area_average_with(
    canvas: &mut Canvas,
    src_w: u32,
    src_h: u32,
    src: &[canvas_proto::Rgb8],
    map: &AreaMap,
) {
    if src_w == 0 || src_h == 0 || src.len() < (src_w as usize) * (src_h as usize) {
        return;
    }
    let tw = canvas.width;
    let th = canvas.height;
    if tw == 0 || th == 0 {
        return;
    }
    for y in 0..th {
        let ys = map.row_start[y as usize] as usize;
        // The end of the row's span.
        let ye = ((y as u64 + 1) * src_h as u64).div_ceil(th as u64) as usize;
        let ye = ye.min(src_h as usize).max(ys + 1);
        for x in 0..tw {
            let xs = map.col_start[x as usize] as usize;
            let xe = ((x as u64 + 1) * src_w as u64).div_ceil(tw as u64) as usize;
            let xe = xe.min(src_w as usize).max(xs + 1);
            let mut r = 0.0f32;
            let mut g = 0.0f32;
            let mut b = 0.0f32;
            let mut n = 0u32;
            for sy in ys..ye {
                let base = sy * src_w as usize;
                for sx in xs..xe {
                    let c = src[base + sx];
                    r += c.r as f32;
                    g += c.g as f32;
                    b += c.b as f32;
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let inv = 1.0 / (n as f32 * 255.0);
            let i = (y as usize) * tw as usize + x as usize;
            canvas.pixels[i] = [r * inv, g * inv, b * inv];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::Canvas;

    #[test]
    fn blend_is_source_over() {
        let mut d = [0.0, 0.0, 0.0];
        blend(&mut d, [1.0, 0.0, 0.0], 0.5);
        assert_eq!(d, [0.5, 0.0, 0.0]);
        blend(&mut d, [1.0, 1.0, 1.0], 0.5);
        assert_eq!(d, [0.75, 0.5, 0.5]);
    }

    #[test]
    fn fill_alpha_covers_everything() {
        let mut c = Canvas::new(8, 4);
        fill_alpha(&mut c, [1.0, 0.0, 0.0], 1.0);
        assert!(c.pixels.iter().all(|p| p == &[1.0, 0.0, 0.0]));
    }

    #[test]
    fn stroke_circle_draws_an_annulus() {
        // A 320x200 canvas, so the logical coordinates map one to one onto the pixels.
        let mut c = Canvas::new(320, 200);
        stroke_circle(&mut c, 160.0, 100.0, 40.0, 6.0, [1.0, 1.0, 1.0], 1.0);
        // On the ring (radius 40), the pixel is lit.
        let on_ring = c.get(200, 100);
        assert!(on_ring[0] > 0.5, "ring pixel not lit: {on_ring:?}");
        // Inside the ring is untouched.
        let inside = c.get(160, 100);
        assert!(inside[0] < 0.1, "inside should be empty: {inside:?}");
        // Outside the ring is untouched.
        let outside = c.get(20, 100);
        assert!(outside[0] < 0.1, "outside should be empty: {outside:?}");
    }

    #[test]
    fn stroke_circle_scales_with_the_canvas() {
        let mut big = Canvas::with_scale(1.0);
        let mut small = Canvas::with_scale(0.5);
        stroke_circle(&mut big, 160.0, 100.0, 40.0, 6.0, [1.0, 1.0, 1.0], 1.0);
        stroke_circle(&mut small, 160.0, 100.0, 40.0, 6.0, [1.0, 1.0, 1.0], 1.0);
        let lit = |c: &Canvas| c.pixels.iter().filter(|p| p[0] > 0.5).count();
        // The small canvas is a quarter of the area, so it holds roughly a quarter of
        // the lit pixels (a little more, because the anti-aliased edge pixels are
        // counted in both).
        let (b, s) = (lit(&big), lit(&small));
        assert!(s * 2 <= b && s * 6 >= b, "lit pixels: big={b} small={s}");
    }

    #[test]
    fn area_average_of_a_uniform_image_keeps_the_color() {
        let mut c = Canvas::new(10, 5);
        let src = vec![canvas_proto::Rgb8::new(255, 0, 0); 40 * 20];
        blit_area_average(&mut c, 40, 20, &src);
        for p in c.pixels.iter() {
            assert!(p[0] > 0.99 && p[1] < 0.01 && p[2] < 0.01, "{p:?}");
        }
    }

    #[test]
    fn area_average_mixes_two_halves() {
        let mut c = Canvas::new(2, 1);
        let src = vec![canvas_proto::Rgb8::new(255, 0, 0), canvas_proto::Rgb8::new(0, 0, 255)];
        blit_area_average(&mut c, 2, 1, &src);
        assert!(c.get(0, 0)[0] > 0.9, "left: {:?}", c.get(0, 0));
        assert!(c.get(1, 0)[2] > 0.9, "right: {:?}", c.get(1, 0));
    }

    #[test]
    fn area_average_scales_a_half_red_frame() {
        // A 320x200 frame whose left half is red, drawn onto a 64x40 canvas.
        let mut c = Canvas::new(64, 40);
        let mut src = Vec::with_capacity(320 * 200);
        for _ in 0..200 {
            for x in 0..320 {
                src.push(if x < 160 {
                    canvas_proto::Rgb8::new(255, 0, 0)
                } else {
                    canvas_proto::Rgb8::new(0, 0, 255)
                });
            }
        }
        blit_area_average(&mut c, 320, 200, &src);
        let left = c.get(4, 20);
        let right = c.get(60, 20);
        assert!(left[0] > 0.9 && left[2] < 0.1, "left: {left:?}");
        assert!(right[2] > 0.9 && right[0] < 0.1, "right: {right:?}");
    }
}
