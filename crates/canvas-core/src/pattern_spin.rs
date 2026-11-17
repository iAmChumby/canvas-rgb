//! Spin (#468): a pattern that turns along its LEDs, and the OKLab blend it shares with the editor.
//!
//! A pattern with `Motion::spin(turns_per_s)` is fitted once, like any pattern, and then every
//! tick the fitted colours are *rotated* by a pure function of the effect's clock: no state, no
//! frame step, so a dropped frame never makes it lurch and two lights playing the same pattern
//! agree to the bit.
//!
//! The turn is `phase = frac(turns_per_s * t)`, the shift is `phase * m` LEDs (`m` is the light's
//! own LED count), so one lap takes the same time on a ring of 8 and a ring of 40: the speed is in
//! laps, never in LEDs. A positive speed moves the picture towards higher LED numbers (clockwise on
//! a ring, which runs clockwise from the top); a negative one the other way. The part of the shift
//! between two LEDs is a blend of the two neighbours **in OKLab**, so a fraction of a step is an
//! even step of lightness and never a muddy RGB average. Rings and lines both wrap: a line's end
//! feeds its start (a chase).
//!
//! The editor's preview runs the same maths in TypeScript (`app/web/src/lib/spin.ts`). Both are
//! checked against one JSON fixture, `app/web/src/lib/__tests__/spin.vectors.json`, which
//! `tests/spin_vectors.rs` reads here and `spin.test.ts` reads there.
//!
//! All the arithmetic is `f64`, as JavaScript's is, so the two agree without a tolerance on the
//! 8-bit result.

use canvas_proto::color::Rgb8;

/// The largest fraction of a step that is still "on the LED": below it the shift is a whole number
/// of LEDs and no blend is made.
const WHOLE: f64 = 1e-9;

/// An sRGB channel (0..=255) as linear light.
fn to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light as an sRGB channel, clamped into gamut and rounded to 8 bits.
fn from_linear(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let encoded = if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

/// A colour in OKLab (Björn Ottosson, 2020): `[L, a, b]`, with L in 0..=1 for in-gamut sRGB.
pub fn to_oklab(c: Rgb8) -> [f64; 3] {
    let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// An OKLab colour as sRGB, clamped into gamut.
pub fn from_oklab(lab: [f64; 3]) -> Rgb8 {
    let [big_l, a, b] = lab;
    let l = (big_l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (big_l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (big_l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    Rgb8::new(
        from_linear(4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s),
        from_linear(-1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s),
        from_linear(-0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s),
    )
}

/// `from` to `to` at `t` (0 is `from`, 1 is `to`), blended in OKLab. A `t` outside 0..=1 is clamped.
pub fn mix(from: Rgb8, to: Rgb8, t: f64) -> Rgb8 {
    mix_lab(from, to_oklab(from), to, to_oklab(to), t)
}

/// [`mix`] with both colours' OKLab already known: the per-tick path converts each colour once.
fn mix_lab(from: Rgb8, from_lab: [f64; 3], to: Rgb8, to_lab: [f64; 3], t: f64) -> Rgb8 {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    // The ends and two alike colours are exact: no round trip through OKLab to lose a level on.
    if from == to || t <= 0.0 {
        return from;
    }
    if t >= 1.0 {
        return to;
    }
    from_oklab([
        from_lab[0] + (to_lab[0] - from_lab[0]) * t,
        from_lab[1] + (to_lab[1] - from_lab[1]) * t,
        from_lab[2] + (to_lab[2] - from_lab[2]) * t,
    ])
}

/// How far round a pattern spinning at `turns_per_s` has turned `t_secs` seconds in: 0.0..1.0.
/// A negative speed counts down, so the phase is still in range.
pub fn phase(turns_per_s: f64, t_secs: f64) -> f64 {
    let turns = turns_per_s * t_secs;
    if !turns.is_finite() {
        return 0.0;
    }
    let phase = turns.rem_euclid(1.0);
    // `rem_euclid` of a tiny negative number is 1.0 after rounding: that is a whole lap, i.e. 0.
    if phase >= 1.0 {
        0.0
    } else {
        phase
    }
}

/// One light's fitted colours, ready to be turned every tick: the colours and their OKLab values,
/// both in position order (the light's own `reversed` is applied after the turn, by the caller).
#[derive(Debug, Clone, PartialEq)]
pub struct SpinSource {
    rgb: Vec<Rgb8>,
    lab: Vec<[f64; 3]>,
}

impl SpinSource {
    /// Converts every colour to OKLab once, so a tick only blends.
    pub fn new(rgb: Vec<Rgb8>) -> Self {
        let lab = rgb.iter().map(|c| to_oklab(*c)).collect();
        Self { rgb, lab }
    }

    /// How many LEDs the source has.
    pub fn len(&self) -> usize {
        self.rgb.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rgb.is_empty()
    }

    /// The colours `t_secs` seconds into a spin of `turns_per_s`, written into `out` (cleared
    /// first): `out[i]` is the source at `i - phase * m`, the fraction blended in OKLab.
    pub fn at(&self, turns_per_s: f64, t_secs: f64, out: &mut Vec<Rgb8>) {
        out.clear();
        let m = self.rgb.len();
        if m < 2 {
            out.extend_from_slice(&self.rgb);
            return;
        }
        let shift = phase(turns_per_s, t_secs) * m as f64;
        let whole = (shift.floor() as usize).min(m - 1);
        let fraction = shift - whole as f64;
        let blend = (WHOLE..1.0 - WHOLE).contains(&fraction);
        // At the other end of the fraction the shift is the next whole LED.
        let whole = if fraction >= 1.0 - WHOLE { (whole + 1) % m } else { whole };
        out.extend((0..m).map(|i| {
            let here = (i + m - whole) % m;
            if !blend {
                return self.rgb[here];
            }
            // One LED further along the way the picture is moving from.
            let behind = (here + m - 1) % m;
            mix_lab(self.rgb[here], self.lab[here], self.rgb[behind], self.lab[behind], fraction)
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgb8 = Rgb8::new(255, 0, 0);
    const BLUE: Rgb8 = Rgb8::new(0, 0, 255);

    /// Eight LEDs, each told apart by its red channel.
    fn eight() -> Vec<Rgb8> {
        (0..8).map(|i| Rgb8::new(10 + 20 * i as u8, 0, 0)).collect()
    }

    #[test]
    fn white_is_l_one_and_red_is_ottossons_published_value() {
        let white = to_oklab(Rgb8::WHITE);
        assert!((white[0] - 1.0).abs() < 1e-6 && white[1].abs() < 1e-6 && white[2].abs() < 1e-6);
        // https://bottosson.github.io/posts/oklab/ lists sRGB red as (0.628, 0.225, 0.126).
        let red = to_oklab(RED);
        assert!((red[0] - 0.628).abs() < 1e-3, "{red:?}");
        assert!((red[1] - 0.225).abs() < 1e-3, "{red:?}");
        assert!((red[2] - 0.126).abs() < 1e-3, "{red:?}");
    }

    #[test]
    fn every_colour_survives_the_round_trip() {
        for r in (0..=255u16).step_by(15) {
            for g in (0..=255u16).step_by(15) {
                for b in (0..=255u16).step_by(15) {
                    let c = Rgb8::new(r as u8, g as u8, b as u8);
                    assert_eq!(from_oklab(to_oklab(c)), c);
                }
            }
        }
    }

    #[test]
    fn the_midpoint_of_red_and_blue_is_a_purple_not_rgb_128_0_128() {
        let mid = mix(RED, BLUE, 0.5);
        // RGB would give (128, 0, 128). OKLab holds the lightness steady between the two, which
        // lands on a lighter, bluer purple.
        assert_ne!(mid, Rgb8::new(128, 0, 128));
        assert!(mid.r > 100 && mid.b > 150, "{mid:?}");
        assert_eq!(mix(RED, BLUE, 0.0), RED);
        assert_eq!(mix(RED, BLUE, 1.0), BLUE);
    }

    #[test]
    fn a_whole_number_of_leds_moves_the_picture_exactly() {
        let source = SpinSource::new(eight());
        let mut out = Vec::new();
        // 0.25 laps a second on 8 LEDs: 2 LEDs a second.
        source.at(0.25, 1.0, &mut out);
        let want: Vec<Rgb8> = (0..8).map(|i| eight()[(i + 8 - 2) % 8]).collect();
        assert_eq!(out, want);
        source.at(0.25, 4.0, &mut out);
        assert_eq!(out, eight(), "a lap later it is back where it started");
        source.at(0.25, 0.0, &mut out);
        assert_eq!(out, eight());
    }

    #[test]
    fn a_negative_speed_turns_the_other_way() {
        let source = SpinSource::new(eight());
        let mut out = Vec::new();
        source.at(-0.25, 1.0, &mut out);
        let want: Vec<Rgb8> = (0..8).map(|i| eight()[(i + 2) % 8]).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn the_speed_is_in_laps_so_any_led_count_turns_at_the_same_rate() {
        // A quarter lap is a quarter of the ring, on 8 LEDs and on 40.
        for m in [8usize, 40, 12, 100] {
            let source = SpinSource::new((0..m).map(|i| Rgb8::new((i % 250) as u8, 1, 1)).collect());
            let mut out = Vec::new();
            source.at(0.25, 1.0, &mut out);
            let want_shift = m / 4;
            assert_eq!(out[want_shift], source.rgb[0], "{m} LEDs: LED 0 is a quarter of the way round");
        }
    }

    #[test]
    fn half_a_step_blends_the_two_neighbours_in_oklab() {
        // 2 LEDs: red then blue. 0.25 laps/s for 1 s on m=2 is a shift of 0.5.
        let source = SpinSource::new(vec![RED, BLUE]);
        let mut out = Vec::new();
        source.at(0.25, 1.0, &mut out);
        let mid = mix(RED, BLUE, 0.5);
        // out[0] reads LED 0 and the one behind it (LED 1), out[1] reads LED 1 and LED 0.
        assert_eq!(out[0], mid);
        assert_eq!(out[1], mid);
    }

    #[test]
    fn a_fraction_at_the_end_of_a_step_is_the_next_whole_led() {
        let source = SpinSource::new(eight());
        let mut out = Vec::new();
        // A hair under 2 LEDs of shift.
        source.at(0.25, 1.0 - 1e-12, &mut out);
        let mut exact = Vec::new();
        source.at(0.25, 1.0, &mut exact);
        assert_eq!(out, exact);
    }

    #[test]
    fn the_phase_stays_in_range_for_any_time_and_speed() {
        for turns in [-5.0, -0.25, 0.0, 0.25, 5.0] {
            for t in [0.0, 1e-9, 0.3, 1.0, 17.77, 3600.0, 1.0e7] {
                let p = phase(turns, t);
                assert!((0.0..1.0).contains(&p), "{turns} t/s at {t}s gave {p}");
            }
        }
        assert_eq!(phase(f64::NAN, 1.0), 0.0);
    }

    #[test]
    fn a_single_led_and_an_empty_source_are_left_alone() {
        let mut out = Vec::new();
        SpinSource::new(vec![RED]).at(1.0, 0.3, &mut out);
        assert_eq!(out, vec![RED]);
        SpinSource::new(Vec::new()).at(1.0, 0.3, &mut out);
        assert!(out.is_empty());
    }
}
