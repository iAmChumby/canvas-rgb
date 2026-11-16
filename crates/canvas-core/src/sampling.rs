//! Sampling the canvas at precomputed LED regions.
//!
//! Three modes, chosen per layout:
//! - **Nearest**: the pixel at the LED's center.
//! - **Bilinear**: the pixel at the LED's center, interpolated.
//! - **Area average**: the unweighted mean of the render pixels whose centers fall in
//!   the LED's region (at least the nearest pixel).
//!
//! A LED with no owner (the LEDs of a split zone that no placement claims) is black:
//! it carries no region and is never sampled.
//!
//! The hot path allocates nothing: it writes into a caller-owned `Vec<Rgb8>`.

use canvas_proto::color::Rgb8;
use canvas_proto::layout::SamplingMode;

use crate::geometry::LedSample;

/// Samples one LED's region out of `canvas` into `out`.
///
/// `out` is resized to `leds.len()`; the caller owns the buffer, so no allocation
/// happens once the buffer has grown.
pub fn sample_leds(
    canvas: &[[f32; 3]],
    width: u32,
    height: u32,
    leds: &[LedSample],
    mode: SamplingMode,
    out: &mut Vec<Rgb8>,
) {
    out.clear();
    out.reserve(leds.len());
    for led in leds {
        out.push(read_led(canvas, width, height, led, mode, false));
    }
}

/// Samples each LED's cell width over the **full height** of `canvas`: the read an effect asks
/// for with `StripRead::Column` (#400).
///
/// The x-range is the LED's own cell, the range [`sample_leds`] would average over; the y-range
/// is the whole canvas. An effect whose picture grows upward from the bottom (a flame, a
/// spectrum bar) says its meaning is how high it reaches, and a cell at mid-height would read
/// the empty picture above it. The mean stays the mean: a flat picture reads exactly what the
/// cell read gives it, and a column of one colour, or of none, is that colour at full height.
pub fn sample_led_columns(
    canvas: &[[f32; 3]],
    width: u32,
    height: u32,
    leds: &[LedSample],
    out: &mut Vec<Rgb8>,
) {
    out.clear();
    out.reserve(leds.len());
    for led in leds {
        out.push(read_led(canvas, width, height, led, SamplingMode::AreaAverage, true));
    }
}

/// One LED's colour: nearest, bilinear or the area mean of its region.
///
/// `full_height` replaces the region's y-range with the whole canvas and keeps its x-range: the
/// column read of #400. Only the area mean can read a column; the other two modes are a pixel
/// at the LED's centre, which no column can change.
fn read_led(
    canvas: &[[f32; 3]],
    width: u32,
    height: u32,
    led: &LedSample,
    mode: SamplingMode,
    full_height: bool,
) -> Rgb8 {
    if !led.sampled {
        // A LED with no owner is black: it is never sampled.
        return Rgb8::BLACK;
    }
    match mode {
        SamplingMode::Nearest => sample_nearest(canvas, width, height, led.center_x, led.center_y),
        SamplingMode::Bilinear => sample_bilinear(canvas, width, height, led.center_x, led.center_y),
        SamplingMode::AreaAverage => {
            let bounds = if full_height {
                [led.bounds[0], 0.0, led.bounds[2], height as f32]
            } else {
                led.bounds
            };
            sample_area(canvas, width, height, led.center_x, led.center_y, bounds)
        }
    }
}

/// The pixel at the LED's center, rounded to the nearest pixel center.
fn sample_nearest(
    canvas: &[[f32; 3]],
    width: u32,
    height: u32,
    center_x: f32,
    center_y: f32,
) -> Rgb8 {
    let (x, y) = nearest_pixel(center_x, center_y, width, height);
    Rgb8::from_f32(canvas[(y * width + x) as usize])
}

/// The pixel at the LED's center, interpolated from the four neighbours.
fn sample_bilinear(
    canvas: &[[f32; 3]],
    width: u32,
    height: u32,
    center_x: f32,
    center_y: f32,
) -> Rgb8 {
    if width == 0 || height == 0 {
        return Rgb8::BLACK;
    }
    let max_x = width - 1;
    let max_y = height - 1;
    // The fractional position inside the pixel grid.
    let fx = center_x.clamp(0.0, max_x as f32);
    let fy = center_y.clamp(0.0, max_y as f32);
    let x0 = fx.floor() as u32;
    let y0 = fy.floor() as u32;
    let x1 = (x0 + 1).min(max_x);
    let y1 = (y0 + 1).min(max_y);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;

    let p00 = canvas[(y0 * width + x0) as usize];
    let p10 = canvas[(y0 * width + x1) as usize];
    let p01 = canvas[(y1 * width + x0) as usize];
    let p11 = canvas[(y1 * width + x1) as usize];

    let mut c = [0.0f32; 3];
    for ch in 0..3 {
        let top = p00[ch] + (p10[ch] - p00[ch]) * tx;
        let bottom = p01[ch] + (p11[ch] - p01[ch]) * tx;
        c[ch] = top + (bottom - top) * ty;
    }
    Rgb8::from_f32(c)
}

/// The mean of the render pixels whose centers fall inside the LED's region.
///
/// At least the nearest pixel is always included, so a region smaller than one pixel
/// still yields that pixel rather than nothing.
fn sample_area(
    canvas: &[[f32; 3]],
    width: u32,
    height: u32,
    center_x: f32,
    center_y: f32,
    bounds: [f32; 4],
) -> Rgb8 {
    if width == 0 || height == 0 {
        return Rgb8::BLACK;
    }
    let [bx0, by0, bx1, by1] = bounds;
    let max_x = width - 1;
    let max_y = height - 1;

    // Pixel (x, y) has its center at (x + 0.5, y + 0.5).
    let x0 = ((bx0 - 0.5).ceil() as i64).clamp(0, i64::from(max_x)) as u32;
    let y0 = ((by0 - 0.5).ceil() as i64).clamp(0, i64::from(max_y)) as u32;
    let mut x1 = ((bx1 - 0.5).floor() as i64).clamp(0, i64::from(max_x)) as u32;
    let mut y1 = ((by1 - 0.5).floor() as i64).clamp(0, i64::from(max_y)) as u32;
    if x1 < x0 {
        x1 = x0;
    }
    if y1 < y0 {
        y1 = y0;
    }
    // An empty region still samples the nearest pixel.
    if x0 > x1 || y0 > y1 {
        let (x, y) = nearest_pixel(center_x, center_y, width, height);
        return Rgb8::from_f32(canvas[(y * width + x) as usize]);
    }

    let mut sum = [0.0f64; 3];
    let mut count = 0u32;
    for y in y0..=y1 {
        let row = y * width;
        for x in x0..=x1 {
            let p = canvas[(row + x) as usize];
            sum[0] += f64::from(p[0]);
            sum[1] += f64::from(p[1]);
            sum[2] += f64::from(p[2]);
            count += 1;
        }
    }
    if count == 0 {
        let (x, y) = nearest_pixel(center_x, center_y, width, height);
        return Rgb8::from_f32(canvas[(y * width + x) as usize]);
    }
    let n = count as f64;
    Rgb8::from_f32([
        (sum[0] / n) as f32,
        (sum[1] / n) as f32,
        (sum[2] / n) as f32,
    ])
}

/// The pixel index nearest to `(cx, cy)`.
fn nearest_pixel(cx: f32, cy: f32, width: u32, height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let x = (cx.round() as i64).clamp(0, i64::from(width - 1)) as u32;
    let y = (cy.round() as i64).clamp(0, i64::from(height - 1)) as u32;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::LedSample;

    fn canvas_2x2() -> (Vec<[f32; 3]>, u32, u32) {
        // [red, green] / [blue, white]
        (vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0]], 2, 2)
    }

    /// A LED that samples the given region.
    fn region(center_x: f32, center_y: f32, bounds: [f32; 4]) -> LedSample {
        LedSample { center_x, center_y, bounds, sampled: true }
    }

    #[test]
    fn nearest_takes_the_pixel_at_the_center() {
        let (c, w, h) = canvas_2x2();
        let led = region(0.0, 0.0, [0.0, 0.0, 1.0, 1.0]);
        let mut out = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::Nearest, &mut out);
        assert_eq!(out, vec![Rgb8::new(255, 0, 0)]);
    }

    #[test]
    fn nearest_clamps_out_of_range_centers() {
        let (c, w, h) = canvas_2x2();
        // Clamped to the right edge (x = 1) and the top edge (y = 0): green.
        let led = region(99.0, -5.0, [0.0, 0.0, 0.0, 0.0]);
        let mut out = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::Nearest, &mut out);
        assert_eq!(out, vec![Rgb8::new(0, 255, 0)]);
    }

    #[test]
    fn nearest_clamps_below_the_origin() {
        let (c, w, h) = canvas_2x2();
        // Clamped to (0, 0): red.
        let led = region(-5.0, -5.0, [0.0, 0.0, 0.0, 0.0]);
        let mut out = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::Nearest, &mut out);
        assert_eq!(out, vec![Rgb8::new(255, 0, 0)]);
    }

    #[test]
    fn bilinear_interpolates_between_pixels() {
        let (c, w, h) = canvas_2x2();
        // Halfway between pixel (0,0) red and pixel (1,0) green.
        let led = region(0.5, 0.0, [0.0, 0.0, 1.0, 1.0]);
        let mut out = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::Bilinear, &mut out);
        assert_eq!(out, vec![Rgb8::new(128, 128, 0)]);
    }

    #[test]
    fn bilinear_at_a_pixel_center_is_that_pixel() {
        let (c, w, h) = canvas_2x2();
        let led = region(1.0, 1.0, [0.0, 0.0, 1.0, 1.0]);
        let mut out = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::Bilinear, &mut out);
        assert_eq!(out, vec![Rgb8::new(255, 255, 255)]);
    }

    #[test]
    fn area_average_means_the_pixels_whose_centers_fall_in_the_region() {
        let (c, w, h) = canvas_2x2();
        // The whole 2x2 canvas.
        let led = region(1.0, 1.0, [0.0, 0.0, 2.0, 2.0]);
        let mut out = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::AreaAverage, &mut out);
        // Red + green + blue + white, averaged.
        assert_eq!(out, vec![Rgb8::new(128, 128, 128)]);
    }

    #[test]
    fn area_average_of_one_pixel_is_that_pixel() {
        let (c, w, h) = canvas_2x2();
        let led = region(0.0, 0.0, [0.0, 0.0, 1.0, 1.0]);
        let mut out = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::AreaAverage, &mut out);
        assert_eq!(out, vec![Rgb8::new(255, 0, 0)]);
    }

    #[test]
    fn area_average_of_an_empty_region_falls_back_to_the_nearest_pixel() {
        let (c, w, h) = canvas_2x2();
        // A degenerate region far from any pixel center.
        let led = region(0.4, 0.4, [0.2, 0.2, 0.2, 0.2]);
        let mut out = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::AreaAverage, &mut out);
        assert_eq!(out, vec![Rgb8::new(255, 0, 0)]);
    }

    #[test]
    fn area_average_of_a_two_pixel_region_averages_those_two() {
        let (c, w, h) = canvas_2x2();
        // Covers the centers of pixel (0,0) and (1,0): red and green.
        let led = region(0.5, 0.0, [0.0, 0.0, 1.5, 1.0]);
        let mut out = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::AreaAverage, &mut out);
        assert_eq!(out, vec![Rgb8::new(128, 128, 0)]);
    }

    /// The column read (#400) averages the LED's x-range over the whole height, not over the
    /// cell's height: the left pixel column of the 2x2 canvas is red over blue.
    #[test]
    fn a_column_read_averages_its_x_range_over_the_whole_height() {
        let (c, w, h) = canvas_2x2();
        let led = region(0.0, 0.0, [0.0, 0.0, 1.0, 1.0]);
        let mut out = Vec::new();
        sample_led_columns(&c, w, h, std::slice::from_ref(&led), &mut out);
        assert_eq!(out, vec![Rgb8::new(128, 0, 128)]);
        // The cell read of the same LED sees its own row only: red, and never the blue under it.
        let mut cell = Vec::new();
        sample_leds(&c, w, h, std::slice::from_ref(&led), SamplingMode::AreaAverage, &mut cell);
        assert_eq!(cell, vec![Rgb8::new(255, 0, 0)]);
    }

    /// The x-range the column reads is the LED's own cell: a cell over the right pixel column
    /// takes that column, green over white.
    #[test]
    fn a_column_read_keeps_the_leds_own_x_range() {
        let (c, w, h) = canvas_2x2();
        let led = region(1.5, 0.0, [1.0, 0.0, 2.0, 1.0]);
        let mut out = Vec::new();
        sample_led_columns(&c, w, h, std::slice::from_ref(&led), &mut out);
        assert_eq!(out, vec![Rgb8::new(128, 255, 128)]);
    }

    /// A flat canvas reads exactly what the cell read reads on every LED.
    #[test]
    fn a_column_read_of_a_flat_canvas_reads_that_colour_exactly() {
        let flat = vec![[0.2f32, 0.4, 0.6]; 4];
        let mut out = Vec::new();
        sample_led_columns(&flat, 2, 2, &[region(1.0, 1.0, [0.5, 0.0, 1.5, 2.0])], &mut out);
        assert_eq!(out, vec![Rgb8::from_f32([0.2, 0.4, 0.6])]);
    }

    #[test]
    fn a_led_with_no_owner_is_black_in_the_column_read_too() {
        let (c, w, h) = canvas_2x2();
        let mut out = Vec::new();
        sample_led_columns(&c, w, h, &[LedSample::default()], &mut out);
        assert_eq!(out, vec![Rgb8::BLACK]);
    }

    #[test]
    fn a_led_with_no_owner_is_black_in_every_mode() {
        let (c, w, h) = canvas_2x2();
        for mode in [SamplingMode::Nearest, SamplingMode::Bilinear, SamplingMode::AreaAverage] {
            let mut out = Vec::new();
            sample_leds(&c, w, h, &[LedSample::default()], mode, &mut out);
            assert_eq!(out, vec![Rgb8::BLACK], "{mode:?} sampled a LED with no owner");
        }
    }

    #[test]
    fn a_led_with_no_owner_is_black_even_next_to_a_lit_one() {
        let (c, w, h) = canvas_2x2();
        let leds = [LedSample::default(), region(0.0, 0.0, [0.0, 0.0, 1.0, 1.0])];
        for mode in [SamplingMode::Nearest, SamplingMode::Bilinear, SamplingMode::AreaAverage] {
            let mut out = Vec::new();
            sample_leds(&c, w, h, &leds, mode, &mut out);
            assert_eq!(out, vec![Rgb8::BLACK, Rgb8::new(255, 0, 0)], "{mode:?}");
        }
    }
}
