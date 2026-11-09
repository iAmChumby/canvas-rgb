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

/// The warmest colour temperature [`kelvin_to_rgb`] draws, in kelvin.
pub const KELVIN_MIN: f32 = 1800.0;
/// The coolest colour temperature [`kelvin_to_rgb`] draws, in kelvin.
pub const KELVIN_MAX: f32 = 6500.0;
/// The least blue a white ever gets, 0..1.
///
/// A blackbody below about 2100 K has almost no blue, and an RGB LED with its blue off
/// reads as orange, not as a warm white, the more so when it is dimmed. Holding blue at
/// this floor keeps the warm end a white.
pub const KELVIN_BLUE_FLOOR: f32 = 0.12;

/// A white of colour temperature `kelvin`, as RGB in 0..1.
///
/// The curve is Tanner Helland's fit to the blackbody colours, the one most LED tools use.
/// `kelvin` is clamped to [`KELVIN_MIN`]..=[`KELVIN_MAX`]; a non-finite value draws the
/// warmest white. Over that range red stays full and green and blue rise with the
/// temperature, so 6500 K is close to a neutral white.
pub fn kelvin_to_rgb(kelvin: f32) -> Rgb {
    let k = if kelvin.is_finite() { kelvin.clamp(KELVIN_MIN, KELVIN_MAX) } else { KELVIN_MIN };
    // The fit works in hundreds of kelvin and gives 0..255 per channel.
    let t = (k / 100.0) as f64;
    let red = if t <= 66.0 { 255.0 } else { 329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2) };
    let green = if t <= 66.0 {
        99.470_802_586_1 * t.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
    };
    let blue = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
    };
    let unit = |v: f64| (v / 255.0).clamp(0.0, 1.0) as f32;
    [unit(red), unit(green), unit(blue).max(KELVIN_BLUE_FLOOR)]
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

    /// Red's part of the white, which is what "warmer" means for a white.
    fn red_share(c: Rgb) -> f32 {
        c[0] / (c[0] + c[1] + c[2])
    }

    #[test]
    fn a_higher_temperature_is_a_cooler_white_across_the_whole_range() {
        let mut last = kelvin_to_rgb(KELVIN_MIN);
        let mut k = KELVIN_MIN + 100.0;
        while k <= KELVIN_MAX {
            let c = kelvin_to_rgb(k);
            assert!(red_share(c) < red_share(last), "red's share did not fall from {} K to {k} K: {last:?} -> {c:?}", k - 100.0);
            assert!(c[0] <= last[0], "red rose at {k} K: {last:?} -> {c:?}");
            assert!(c[1] > last[1], "green did not rise at {k} K: {last:?} -> {c:?}");
            assert!(c[2] >= last[2], "blue fell at {k} K: {last:?} -> {c:?}");
            last = c;
            k += 100.0;
        }
        // Blue climbs well clear of its floor by the cool end.
        assert!(kelvin_to_rgb(KELVIN_MAX)[2] > kelvin_to_rgb(2700.0)[2] + 0.5);
    }

    #[test]
    fn six_thousand_five_hundred_kelvin_is_close_to_neutral() {
        let c = kelvin_to_rgb(6500.0);
        for v in c {
            assert!(v >= 0.97, "6500 K is not near neutral: {c:?}");
        }
    }

    #[test]
    fn the_temperature_is_clamped_to_its_range() {
        assert_eq!(kelvin_to_rgb(1000.0), kelvin_to_rgb(KELVIN_MIN));
        assert_eq!(kelvin_to_rgb(0.0), kelvin_to_rgb(KELVIN_MIN));
        assert_eq!(kelvin_to_rgb(-5.0), kelvin_to_rgb(KELVIN_MIN));
        assert_eq!(kelvin_to_rgb(20_000.0), kelvin_to_rgb(KELVIN_MAX));
        assert_eq!(kelvin_to_rgb(f32::NAN), kelvin_to_rgb(KELVIN_MIN));
        assert_eq!(kelvin_to_rgb(f32::INFINITY), kelvin_to_rgb(KELVIN_MIN));
        for k in [KELVIN_MIN, 2700.0, 4000.0, KELVIN_MAX] {
            for v in kelvin_to_rgb(k) {
                assert!(v.is_finite() && (0.0..=1.0).contains(&v), "{k} K gave {v}");
            }
        }
    }

    #[test]
    fn the_warmest_white_keeps_its_blue_floor() {
        let c = kelvin_to_rgb(KELVIN_MIN);
        assert_eq!(c[2], KELVIN_BLUE_FLOOR, "the warm end lost its blue: {c:?}");
        assert_eq!(c[0], 1.0);
        assert!(c[1] > 0.4 && c[1] < 0.6, "1800 K green is off the curve: {c:?}");
    }
}
