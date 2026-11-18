//! Patterns (0.16.0, #339): the colours a pattern gives one light.
//!
//! A pattern is built on one kind of device and every light of a group may be another shape: a
//! ring of 24 on a pattern built on a ring of 12, a strip where the pattern was built on a ring, a
//! light of one LED. [`fit_colors`] works out the colour of every LED for one light; the engine
//! calls it once per change (a new pattern, a new plan, a resized zone), never per frame.
//!
//! The rules are the spec's "Fit" (`docs/superpowers/specs/2026-11-12-patterns-design.md`), with
//! the rounding the plan pins:
//! - the same count and arrangement is a copy;
//! - a light of one LED takes the pattern's first colour that is not off, or off if every colour
//!   is off;
//! - a ring stretched to another count reads source index `round(j * n / m) % n`, ties up, so
//!   LED 0 stays at the top and a quarter round stays a quarter round;
//! - a strip, a single or a grid stretched to another count reads `round(j * (n - 1) / (m - 1))`,
//!   so the first LED reads the first colour and the last reads the last (a grid is stretched by
//!   LED order: a two-dimensional stretch needs the zone's cell map for both devices, which
//!   0.16.0 does not have);
//! - a different arrangement is stretched the same way, by LED order; equal counts copy.
//!
//! The index maths is in `u64` so a zone of [`PATTERN_LEDS_MAX`] LEDs cannot overflow it.

use canvas_proto::color::Rgb8;
use canvas_proto::pattern::{fit_kind, BuiltOn, FitKind, Pattern};
use crate::pattern_play::Timeline;
use canvas_proto::scene::Arrangement;

/// The most fitted LED-frames the patterns of one scene keep ready at once (#481). A pattern that
/// plays through its frames keeps every frame fitted to every distinct light shape, 27 bytes an
/// LED (the colour and its OKLab values): measured on the fakes in a release build, a million
/// LED-frames is 28 MB and about 124 ms of fitting on the render thread, so this is about 108 MB
/// and half a second. A scene of thirty different 300-LED lights and 256 frames is 2.3 million,
/// well under it; four 4096-LED lights of 256 frames are over.
pub const SCENE_PATTERN_LED_FRAMES_MAX: u64 = 4_000_000;

/// The LED-frames a pattern keeps fitted on one group's lights, which is what
/// [`SCENE_PATTERN_LED_FRAMES_MAX`] counts. `shapes` is each light's LED count and arrangement.
/// Lights of one shape share one fitted set, so they count once; a sequence is one run of every
/// light's LEDs end to end (`run`). A pattern that stands still keeps no frames at all.
pub fn fitted_led_frames(pattern: &Pattern, shapes: &[(u32, Arrangement)], run: bool) -> u64 {
    if Timeline::of(pattern).is_still() {
        return 0;
    }
    let mut counted: Vec<(u32, Arrangement)> = Vec::new();
    let mut leds: u64 = 0;
    for shape in shapes {
        if run || !counted.contains(shape) {
            counted.push(*shape);
            leds += u64::from(shape.0);
        }
    }
    pattern.frames.len() as u64 * leds
}

/// The colours for one light, in position order (the order the owner sees the device in; the
/// engine applies the light's own `reversed` afterwards). `source` is the pattern's frame.
/// `source.len()` equals `built_on.led_count`. The result has `target_leds` entries.
///
/// A `source` of no colours (which [`canvas_proto::pattern::validate_pattern`] refuses) is all
/// off, so the caller still gets `target_leds` colours.
pub fn fit_colors(
    source: &[Rgb8],
    built_on: &BuiltOn,
    target_leds: u32,
    target: Arrangement,
) -> Vec<Rgb8> {
    let m = target_leds as usize;
    let n = source.len();
    if m == 0 {
        return Vec::new();
    }
    if n == 0 {
        return vec![Rgb8::BLACK; m];
    }
    // A light of one LED is the first colour that is on, whatever the shapes are.
    if fit_kind(built_on, target_leds, target) == FitKind::OneLed {
        let first = source.iter().copied().find(|c| *c != Rgb8::BLACK).unwrap_or(Rgb8::BLACK);
        return vec![first; m];
    }
    // Equal counts copy: `Exact`, and the "Equal counts copy" rule of a different shape.
    if n == m {
        return source.to_vec();
    }
    // The same ring keeps its positions around the circle; every other pair runs along the LED
    // order, including a ring onto another shape.
    if built_on.arrangement == Arrangement::Ring && target == Arrangement::Ring {
        ring_stretch(source, m)
    } else {
        strip_stretch(source, m)
    }
}

/// `m` LEDs read a ring of `n` at the same position: LED `j` reads `round(j * n / m) % n`, ties
/// up.
fn ring_stretch(source: &[Rgb8], m: usize) -> Vec<Rgb8> {
    let n = source.len() as u64;
    (0..m)
        .map(|j| {
            // `round(j * n / m)` in integers: floor((2 * j * n + m) / (2 * m)).
            let index = (2 * j as u64 * n + m as u64) / (2 * m as u64);
            source[(index % n) as usize]
        })
        .collect()
}

/// `m` LEDs read a run of `n`: LED `j` reads `round(j * (n - 1) / (m - 1))`, ties up, so the
/// first and last stay put. A one-colour source fills every LED.
fn strip_stretch(source: &[Rgb8], m: usize) -> Vec<Rgb8> {
    let n = source.len();
    if n == 1 {
        return vec![source[0]; m];
    }
    // `m > 1` here: a target of one LED is `OneLed`, and an equal count copied above.
    let span = (m - 1) as u64;
    let last = (n - 1) as u64;
    (0..m)
        .map(|j| {
            // `round(j * (n - 1) / (m - 1))` in integers: floor((2 * j * last + span) / (2 * span)).
            let index = (2 * j as u64 * last + span) / (2 * span);
            source[index as usize]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::pattern::PATTERN_LEDS_MAX;

    const RED: Rgb8 = Rgb8::new(255, 0, 0);
    const BLUE: Rgb8 = Rgb8::new(0, 0, 255);
    const OFF: Rgb8 = Rgb8::BLACK;

    /// `n` colours, no two alike and none of them off: index `i` is `(i % 256, i / 256, 1)`.
    fn ramp(n: usize) -> Vec<Rgb8> {
        (0..n).map(|i| Rgb8::new((i % 256) as u8, (i / 256) as u8, 1)).collect()
    }

    fn on(leds: u32, arrangement: Arrangement) -> BuiltOn {
        BuiltOn { led_count: leds, arrangement, grid: None }
    }

    /// Where a colour sits in the source: the tests' colours are all distinct.
    fn at(source: &[Rgb8], color: Rgb8) -> usize {
        source.iter().position(|c| *c == color).expect("a colour of the source")
    }

    #[test]
    fn exact_is_a_copy() {
        let source = ramp(12);
        let out = fit_colors(&source, &on(12, Arrangement::Ring), 12, Arrangement::Ring);
        assert_eq!(out, source);
        let out = fit_colors(&source, &on(12, Arrangement::Strip), 12, Arrangement::Strip);
        assert_eq!(out, source);
    }

    #[test]
    fn a_ring_stretched_to_double_keeps_the_top_led_at_the_top() {
        let source = ramp(12);
        let out = fit_colors(&source, &on(12, Arrangement::Ring), 24, Arrangement::Ring);
        assert_eq!(out.len(), 24);
        assert_eq!(out[0], source[0], "the top LED stays at the top");
        assert_eq!(out[12], source[6], "half way round stays half way round");
        assert_eq!(out[1], source[1], "the tie at 0.5 rounds up");
    }

    #[test]
    fn a_ring_stretched_to_half_reads_every_second_led() {
        let source = ramp(24);
        let out = fit_colors(&source, &on(24, Arrangement::Ring), 12, Arrangement::Ring);
        assert_eq!(out.len(), 12);
        for (j, color) in out.iter().enumerate() {
            assert_eq!(*color, source[2 * j], "LED {j} reads source LED {}", 2 * j);
        }
    }

    #[test]
    fn a_strip_stretch_maps_first_to_first_and_last_to_last() {
        for (n, m) in [(10usize, 25u32), (25, 10)] {
            let source = ramp(n);
            let out = fit_colors(&source, &on(n as u32, Arrangement::Strip), m, Arrangement::Strip);
            assert_eq!(out.len(), m as usize);
            assert_eq!(out[0], source[0], "{n} -> {m}: the first LED reads the first colour");
            assert_eq!(
                out[m as usize - 1],
                source[n - 1],
                "{n} -> {m}: the last LED reads the last colour"
            );
            // The run never goes back: each LED reads the same source LED as the one before it or
            // a later one.
            let mut last = 0;
            for color in &out {
                let index = at(&source, *color);
                assert!(index >= last, "{n} -> {m}: LED order goes back at {index}");
                last = index;
            }
        }
    }

    #[test]
    fn a_one_led_source_fills_every_led() {
        let source = vec![Rgb8::new(0, 255, 0)];
        let out = fit_colors(&source, &on(1, Arrangement::Single), 7, Arrangement::Strip);
        assert_eq!(out, vec![source[0]; 7]);
    }

    #[test]
    fn a_single_led_takes_the_first_colour_that_is_on() {
        let source = vec![OFF, OFF, RED, BLUE];
        let out = fit_colors(&source, &on(4, Arrangement::Ring), 1, Arrangement::Single);
        assert_eq!(out, vec![RED]);
        // A light of one LED on a ring is the same rule.
        let out = fit_colors(&source, &on(4, Arrangement::Ring), 1, Arrangement::Ring);
        assert_eq!(out, vec![RED]);
    }

    #[test]
    fn an_all_off_pattern_is_all_off() {
        let source = vec![OFF; 12];
        for (target_leds, target) in [
            (12u32, Arrangement::Ring),
            (24, Arrangement::Ring),
            (20, Arrangement::Strip),
            (1, Arrangement::Single),
            (5, Arrangement::Grid),
            (12, Arrangement::Strip),
        ] {
            let out = fit_colors(&source, &on(12, Arrangement::Ring), target_leds, target);
            assert_eq!(out.len(), target_leds as usize, "{target_leds} {target:?}");
            assert!(
                out.iter().all(|c| *c == OFF),
                "an all-off pattern is black on {target_leds} {target:?}: {out:?}"
            );
        }
        // A one-LED all-off pattern is all off on a long strip too.
        let out = fit_colors(&[OFF], &on(1, Arrangement::Single), 30, Arrangement::Strip);
        assert_eq!(out.len(), 30);
        assert!(out.iter().all(|c| *c == OFF));
    }

    #[test]
    fn a_different_arrangement_stretches_by_led_order() {
        let source = ramp(12);
        // Equal counts copy whatever the shapes are.
        let out = fit_colors(&source, &on(12, Arrangement::Ring), 12, Arrangement::Strip);
        assert_eq!(out, source);
        // Another count takes the strip formula, not the ring's.
        let out = fit_colors(&source, &on(12, Arrangement::Ring), 24, Arrangement::Strip);
        assert_eq!(out.len(), 24);
        assert_eq!(out[0], source[0]);
        assert_eq!(out[23], source[11]);
        for (j, color) in out.iter().enumerate() {
            let want = (2 * j as u64 * 11 + 23) / 46;
            assert_eq!(*color, source[want as usize], "LED {j} reads {want}");
        }
    }

    #[test]
    fn the_output_length_is_always_target_leds() {
        for n in 1..=300usize {
            let source = ramp(n);
            for m in 1..=300u32 {
                for built in
                    [Arrangement::Ring, Arrangement::Strip, Arrangement::Grid, Arrangement::Single]
                {
                    for target in [
                        Arrangement::Ring,
                        Arrangement::Strip,
                        Arrangement::Grid,
                        Arrangement::Single,
                    ] {
                        let out = fit_colors(&source, &on(n as u32, built), m, target);
                        assert_eq!(
                            out.len(),
                            m as usize,
                            "{n} {built:?} onto {m} {target:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn fit_colors_agrees_with_fit_kind_on_when_it_copies() {
        let counts = [1u32, 2, 3, 12, 24, 60, 300];
        let shapes =
            [Arrangement::Ring, Arrangement::Strip, Arrangement::Grid, Arrangement::Single];
        for n in counts {
            let source = ramp(n as usize);
            for m in counts {
                for built in shapes {
                    for target in shapes {
                        let out = fit_colors(&source, &on(n, built), m, target);
                        let copied = out == source;
                        let kind = fit_kind(&on(n, built), m, target);
                        assert_eq!(
                            kind == FitKind::Exact,
                            copied && built == target,
                            "{n} {built:?} onto {m} {target:?}: {kind:?}, copied {copied}"
                        );
                        if kind == FitKind::Exact {
                            assert!(copied, "{n} {built:?} onto {m} {target:?} must copy");
                        }
                    }
                }
            }
        }
    }

    /// The largest zone the engine maps: the index maths stays inside `u64` and the ends stay
    /// put on a ring one LED short of the pattern.
    #[test]
    fn a_4096_led_ring_stretches_without_overflow() {
        let source = ramp(PATTERN_LEDS_MAX as usize);
        let out =
            fit_colors(&source, &on(PATTERN_LEDS_MAX, Arrangement::Ring), PATTERN_LEDS_MAX, Arrangement::Ring);
        assert_eq!(out, source);
        let out = fit_colors(
            &source,
            &on(PATTERN_LEDS_MAX, Arrangement::Ring),
            PATTERN_LEDS_MAX - 1,
            Arrangement::Ring,
        );
        assert_eq!(out.len(), PATTERN_LEDS_MAX as usize - 1);
        assert_eq!(out[0], source[0], "the top LED stays at the top");
    }
}
