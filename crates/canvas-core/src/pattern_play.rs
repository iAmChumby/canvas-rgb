//! Frames in time (#468): which frame of a pattern shows at a moment, and how one fades into the
//! next.
//!
//! A pattern with more than one frame plays as a pure function of the effect's clock: no state, no
//! step, so a dropped frame never makes it lurch and two lights playing the same pattern agree to
//! the bit. `tau = t mod cycle_ms` when it loops (`min(t, end)` when it does not), the frame is a
//! binary search of the frame start times, and inside the frame's `hold` it shows solid. After the
//! hold a [`TransitionKind::Fade`] blends it into the next frame (the first, for the last frame of
//! a looping pattern) over the transition's `ms`, paced by an [`Easing`], every LED mixed in OKLab
//! (`pattern_spin::mix`). A cut jumps at the end of the hold.
//!
//! The span of frame `i` is `hold_i` plus its fade's `ms`, and the last frame's fade counts only
//! when the pattern loops ([`Pattern::cycle_ms`]). A pattern with one frame, or with a cycle of
//! 0 ms, is *still*: it shows its first frame and the engine does no per-tick work for it.
//!
//! The editor's preview runs the same maths in TypeScript (`app/web/src/lib/patternPlay.ts`). Both
//! are checked against one fixture, `app/web/src/lib/__tests__/pattern.vectors.json`, which
//! `tests/pattern_vectors.rs` reads here and `patternPlay.test.ts` reads there. All the arithmetic
//! is `f64`, polynomial or `%`, as JavaScript's is, so the two agree without a tolerance.

use crate::pattern_spin::SpinSource;
use canvas_proto::color::Rgb8;
use canvas_proto::pattern::{Easing, Pattern, TransitionKind};

/// The pace of a fade: `u`, how far through it (0..=1, clamped; a `u` that is not a number is 0),
/// to how much of the way the colours have gone. Linear is `u`; ease-in starts slowly (`u²`);
/// ease-out ends slowly (`1 - (1 - u)²`); ease-in-out is the smoothstep `u²(3 - 2u)`. Each maps 0
/// to 0 and 1 to 1 exactly.
pub fn ease(easing: Easing, u: f64) -> f64 {
    let u = if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 };
    match easing {
        Easing::Linear => u,
        Easing::EaseIn => u * u,
        Easing::EaseOut => 1.0 - (1.0 - u) * (1.0 - u),
        Easing::EaseInOut => u * u * (3.0 - 2.0 * u),
    }
}

/// What a pattern shows at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Beat {
    /// Frame `.0` solid.
    Hold(usize),
    /// Frame `from` blending into frame `to`, `u` of the way (already eased, 0 < u < 1 inside a fade).
    Fade { from: usize, to: usize, u: f64 },
}

/// When each frame of one pattern starts, built once per change and read every tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    /// Where each frame's span starts in the cycle, in ms (ascending; frame 0 is 0).
    starts: Vec<f64>,
    holds: Vec<f64>,
    /// Each frame's fade, in ms: 0 for a cut, and for the last frame of a pattern that does not loop.
    fades: Vec<f64>,
    easings: Vec<Easing>,
    cycle: f64,
    looping: bool,
}

impl Timeline {
    /// The timeline of `pattern`'s frames.
    pub fn of(pattern: &Pattern) -> Self {
        let n = pattern.frames.len();
        let looping = pattern.looping();
        let mut starts = Vec::with_capacity(n);
        let mut holds = Vec::with_capacity(n);
        let mut fades = Vec::with_capacity(n);
        let mut easings = Vec::with_capacity(n);
        let mut at = 0.0_f64;
        for (i, frame) in pattern.frames.iter().enumerate() {
            let (fade, easing) = match frame.transition {
                Some(t) if t.kind == TransitionKind::Fade && (i + 1 < n || looping) => (f64::from(t.ms), t.easing),
                Some(t) => (0.0, t.easing),
                None => (0.0, Easing::Linear),
            };
            starts.push(at);
            holds.push(f64::from(frame.hold));
            fades.push(fade);
            easings.push(easing);
            at += f64::from(frame.hold) + fade;
        }
        Self { starts, holds, fades, easings, cycle: at, looping }
    }

    /// One pass through the frames, in ms.
    pub fn cycle_ms(&self) -> f64 {
        self.cycle
    }

    /// Whether the pattern shows its first frame and nothing else, ever: one frame, or a cycle of 0 ms.
    pub fn is_still(&self) -> bool {
        self.starts.len() < 2 || self.cycle <= 0.0
    }

    /// What shows `t_ms` milliseconds into the effect. A time before 0 or not a number is 0.
    pub fn at(&self, t_ms: f64) -> Beat {
        if self.is_still() {
            return Beat::Hold(0);
        }
        let t = if t_ms.is_finite() && t_ms > 0.0 { t_ms } else { 0.0 };
        let tau = if self.looping {
            t % self.cycle
        } else if t >= self.cycle {
            return Beat::Hold(self.starts.len() - 1);
        } else {
            t
        };
        // The last frame that has started: a frame of no span shares its start with the next and is
        // never the one found.
        let i = self.starts.partition_point(|s| *s <= tau).saturating_sub(1);
        let into = tau - self.starts[i];
        let (hold, fade) = (self.holds[i], self.fades[i]);
        if into < hold || fade <= 0.0 {
            return Beat::Hold(i);
        }
        let next = (i + 1) % self.starts.len();
        Beat::Fade { from: i, to: next, u: ease(self.easings[i], (into - hold) / fade) }
    }
}

/// One light's colours for `beat`: the frame it names, or the blend of two (written into `scratch`,
/// which keeps its buffers, so a tick allocates nothing once they have grown), then the turn of
/// that picture when `spin` is `(turns_per_s, t_secs)`. The light's own `reversed` is the caller's,
/// after this. Returns false when `beat` names a frame the set does not hold.
pub fn render(
    frames: &[SpinSource],
    scratch: &mut SpinSource,
    beat: Beat,
    spin: Option<(f64, f64)>,
    out: &mut Vec<Rgb8>,
) -> bool {
    let shown: &SpinSource = match beat {
        Beat::Hold(i) => match frames.get(i) {
            Some(frame) => frame,
            None => return false,
        },
        Beat::Fade { from, to, u } => {
            let (Some(a), Some(b)) = (frames.get(from), frames.get(to)) else {
                return false;
            };
            scratch.blend_from(a, b, u);
            scratch
        }
    };
    match spin {
        Some((turns, t)) => shown.at(turns, t, out),
        None => {
            out.clear();
            out.extend_from_slice(shown.rgb());
        }
    }
    true
}

/// The colours of a pattern `t_ms` into its clock (turned by `spin_turns_per_s` laps a second, if
/// any), written into `out`: the shared definition the editor's TypeScript is checked against
/// (every frame is already at the light's own LED count). The engine reads [`Timeline::at`] and
/// calls [`render`] with its own buffers.
pub fn eval(
    frames: &[SpinSource],
    timeline: &Timeline,
    t_ms: f64,
    spin_turns_per_s: Option<f64>,
    out: &mut Vec<Rgb8>,
) {
    let mut scratch = SpinSource::new(Vec::new());
    let spin = spin_turns_per_s.map(|turns| (turns, t_ms / 1000.0));
    if !render(frames, &mut scratch, timeline.at(t_ms), spin, out) {
        out.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::pattern::{BuiltOn, Frame, Motion, Playback, Transition};
    use canvas_proto::scene::Arrangement;

    const RED: Rgb8 = Rgb8::new(255, 0, 0);
    const GREEN: Rgb8 = Rgb8::new(0, 255, 0);
    const BLUE: Rgb8 = Rgb8::new(0, 0, 255);

    fn frame(colour: Rgb8, hold: u32, transition: Option<Transition>) -> Frame {
        Frame { colors: vec![colour; 2], hold, transition, extra: Default::default() }
    }

    fn fade(ms: u32, easing: Easing) -> Option<Transition> {
        Some(Transition { kind: TransitionKind::Fade, ms, easing })
    }

    fn pattern(frames: Vec<Frame>, looping: bool) -> Pattern {
        Pattern {
            id: "pattern:Test".into(),
            built_on: BuiltOn { led_count: 2, arrangement: Arrangement::Strip, grid: None },
            frames,
            motion: Motion::none(),
            playback: (!looping).then_some(Playback { looping: false }),
            extra: Default::default(),
        }
    }

    #[test]
    fn every_easing_maps_the_ends_exactly_and_the_middle_as_named() {
        for e in [Easing::Linear, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut] {
            assert_eq!(ease(e, 0.0), 0.0);
            assert_eq!(ease(e, 1.0), 1.0);
            assert_eq!(ease(e, -3.0), 0.0, "clamped below");
            assert_eq!(ease(e, 9.0), 1.0, "clamped above");
            assert_eq!(ease(e, f64::NAN), 0.0);
        }
        assert_eq!(ease(Easing::Linear, 0.25), 0.25);
        assert_eq!(ease(Easing::EaseIn, 0.5), 0.25);
        assert_eq!(ease(Easing::EaseOut, 0.5), 0.75);
        assert_eq!(ease(Easing::EaseInOut, 0.5), 0.5);
        assert!(ease(Easing::EaseInOut, 0.25) < 0.25 && ease(Easing::EaseInOut, 0.75) > 0.75);
    }

    #[test]
    fn a_hold_a_cut_and_a_fade_land_where_the_spans_say() {
        // A holds 100, fades 50 into B; B holds 100, cuts to C; C holds 100, fades 50 back to A.
        let p = pattern(
            vec![
                frame(RED, 100, fade(50, Easing::Linear)),
                frame(GREEN, 100, None),
                frame(BLUE, 100, fade(50, Easing::Linear)),
            ],
            true,
        );
        let t = Timeline::of(&p);
        assert_eq!(t.cycle_ms(), 400.0);
        assert_eq!(t.cycle_ms() as u64, p.cycle_ms());
        assert_eq!(t.at(0.0), Beat::Hold(0));
        assert_eq!(t.at(99.9), Beat::Hold(0));
        assert_eq!(t.at(100.0), Beat::Fade { from: 0, to: 1, u: 0.0 });
        assert_eq!(t.at(125.0), Beat::Fade { from: 0, to: 1, u: 0.5 });
        assert_eq!(t.at(150.0), Beat::Hold(1));
        assert_eq!(t.at(249.9), Beat::Hold(1), "a cut holds to the end of the hold");
        assert_eq!(t.at(250.0), Beat::Hold(2), "then jumps");
        assert_eq!(t.at(375.0), Beat::Fade { from: 2, to: 0, u: 0.5 }, "the last fades into the first");
        assert_eq!(t.at(400.0), Beat::Hold(0), "the cycle wraps to frame 0 exactly");
        assert_eq!(t.at(800.0 + 125.0), t.at(125.0), "two cycles on");
    }

    #[test]
    fn a_pattern_that_does_not_loop_stops_on_its_last_frame() {
        let p = pattern(
            vec![frame(RED, 100, fade(100, Easing::Linear)), frame(GREEN, 100, fade(100, Easing::Linear))],
            false,
        );
        let t = Timeline::of(&p);
        // The last frame's fade does not count when nothing follows it: 100 + 100 + 100.
        assert_eq!(t.cycle_ms(), 300.0);
        assert_eq!(t.at(299.0), Beat::Hold(1));
        assert_eq!(t.at(300.0), Beat::Hold(1));
        assert_eq!(t.at(9.0e9), Beat::Hold(1));
        assert_eq!(t.at(150.0), Beat::Fade { from: 0, to: 1, u: 0.5 });
    }

    #[test]
    fn a_still_pattern_is_frame_zero_at_any_time() {
        let one = Timeline::of(&pattern(vec![frame(RED, 500, fade(500, Easing::Linear))], true));
        assert!(one.is_still());
        let no_time = Timeline::of(&pattern(vec![frame(RED, 0, None), frame(GREEN, 0, None)], true));
        assert!(no_time.is_still());
        for t in [0.0, 1.0, 1e9, -5.0, f64::NAN] {
            assert_eq!(one.at(t), Beat::Hold(0));
            assert_eq!(no_time.at(t), Beat::Hold(0));
        }
    }

    #[test]
    fn a_frame_with_no_time_is_skipped_and_a_fade_of_no_time_is_a_cut() {
        let p = pattern(
            vec![frame(RED, 100, None), frame(GREEN, 0, None), frame(BLUE, 100, fade(0, Easing::EaseIn))],
            true,
        );
        let t = Timeline::of(&p);
        assert_eq!(t.cycle_ms(), 200.0);
        assert_eq!(t.at(100.0), Beat::Hold(2), "the frame of no time never shows");
        assert_eq!(t.at(199.0), Beat::Hold(2));
        assert_eq!(t.at(200.0), Beat::Hold(0));
    }

    #[test]
    fn a_fade_midpoint_is_the_oklab_mix_not_the_rgb_average() {
        let p = pattern(vec![frame(RED, 100, fade(100, Easing::Linear)), frame(BLUE, 100, None)], true);
        let frames: Vec<SpinSource> = p.frames.iter().map(|f| SpinSource::new(f.colors.clone())).collect();
        let t = Timeline::of(&p);
        let mut out = Vec::new();
        eval(&frames, &t, 150.0, None, &mut out);
        let want = crate::pattern_spin::mix(RED, BLUE, 0.5);
        assert_eq!(out, vec![want; 2]);
        assert_ne!(want, Rgb8::new(128, 0, 128), "an RGB average would be (128, 0, 128)");
    }

    #[test]
    fn the_loop_has_no_seam() {
        // The last frame fades into the first: just before the cycle ends the colours are almost
        // frame 0, and at the cycle's end they are frame 0 exactly.
        let p = pattern(vec![frame(RED, 100, None), frame(BLUE, 100, fade(100, Easing::EaseInOut))], true);
        let frames: Vec<SpinSource> = p.frames.iter().map(|f| SpinSource::new(f.colors.clone())).collect();
        let t = Timeline::of(&p);
        let (mut before, mut after) = (Vec::new(), Vec::new());
        eval(&frames, &t, t.cycle_ms() - 1e-6, None, &mut before);
        eval(&frames, &t, t.cycle_ms(), None, &mut after);
        assert_eq!(after, vec![RED; 2]);
        assert_eq!(before, after, "no jump across the seam");
    }
}
