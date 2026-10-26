//! Color helpers: linear RGB, HSV/HSL conversion and small palette utilities.

use canvas_proto::Rgb8;

/// Linear RGB, 0..1 per channel. This is what the canvas stores.
pub type Rgb = [f32; 3];

pub const BLACK: Rgb = [0.0, 0.0, 0.0];
pub const WHITE: Rgb = [1.0, 1.0, 1.0];

/// An 8-bit color as linear floats.
pub fn to_linear(c: Rgb8) -> Rgb {
    c.to_f32()
}

/// Linear float -> 8-bit, clamped.
pub fn to_rgb8(c: Rgb) -> Rgb8 {
    Rgb8::from_f32(c)
}

/// Clamps every channel to 0..1.
pub fn clamp01(c: Rgb) -> Rgb {
    [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)]
}

/// Linear interpolation between two colors.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// HSV (all 0..1, hue 0..1 wrapping) -> linear RGB.
///
/// The canvas is linear, but for these effects a plain sRGB-ish mapping is close enough
/// and keeps the code readable; the same helper is used for every effect so they all
/// agree on what "hue 0.5" looks like.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let h = h.rem_euclid(1.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [r, g, b]
}

/// The value of a color's dominant (max) channel, 0..1.
pub fn max_channel(c: Rgb) -> f32 {
    c[0].max(c[1]).max(c[2])
}

/// Scales a color's saturation around its own luminance.
///
/// `amount` 0 keeps the color, 1 is fully saturated, values above 1 oversaturate.
pub fn saturate(c: Rgb, amount: f32) -> Rgb {
    let m = max_channel(c);
    if m <= f32::EPSILON {
        return c;
    }
    let lum = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        out[i] = (lum + (c[i] - lum) * amount).clamp(0.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_primary_hues() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]);
        let g = hsv_to_rgb(1.0 / 3.0, 1.0, 1.0);
        assert!(g[1] > 0.9 && g[0] < 0.1 && g[2] < 0.1, "{g:?}");
        let b = hsv_to_rgb(2.0 / 3.0, 1.0, 1.0);
        assert!(b[2] > 0.9 && b[0] < 0.1 && b[1] < 0.1, "{b:?}");
    }

    #[test]
    fn hsv_wraps() {
        assert_eq!(hsv_to_rgb(1.0, 1.0, 1.0), hsv_to_rgb(0.0, 1.0, 1.0));
        assert_eq!(hsv_to_rgb(-0.25, 1.0, 1.0), hsv_to_rgb(0.75, 1.0, 1.0));
    }

    #[test]
    fn mix_endpoints() {
        let a = [0.0, 0.0, 0.0];
        let b = [1.0, 0.5, 0.25];
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, 0.5), [0.5, 0.25, 0.125]);
    }

    #[test]
    fn saturate_pushes_toward_pure() {
        let c = [0.5, 0.4, 0.3];
        let s = saturate(c, 2.0);
        assert!(s[0] > c[0] && s[2] < c[2], "{s:?}");
        assert_eq!(saturate(c, 1.0), saturate(c, 1.0));
    }
}
