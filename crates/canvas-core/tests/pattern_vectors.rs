//! Frames in time against the vectors the editor's TypeScript shares (#468).
//!
//! `app/web/src/lib/__tests__/pattern.vectors.json` is the agreement between this crate
//! (`pattern_play.rs`) and the editor (`lib/patternPlay.ts`): the editor's tests compare their own
//! output with the same file, so the preview and the lights cannot drift apart. The hold, cut and
//! still cases carry `expect` values written by hand, so a wrong engine cannot write its own answer
//! into them; the fades are what this crate makes (OKLab, itself checked against Ottosson's
//! published values in `spin_vectors.rs`). Set `CANVAS_WRITE_VECTORS=1` to rewrite the `expect`
//! fields after a deliberate change, then read the diff: hand-written cases must not move.

use canvas_core::pattern_play::{ease, eval, Timeline};
use canvas_core::pattern_spin::SpinSource;
use canvas_proto::color::Rgb8;
use canvas_proto::pattern::{Easing, Pattern};
use serde_json::Value;
use std::path::PathBuf;

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/web/src/lib/__tests__/pattern.vectors.json")
}

fn hex(c: Rgb8) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

fn easing(name: &str) -> Easing {
    serde_json::from_value(Value::String(name.into())).expect("an easing")
}

/// The vectors with every `expect` as this crate makes it.
fn made(vectors: &Value) -> Value {
    let mut out = vectors.clone();
    for case in out["ease"].as_array_mut().expect("an ease list") {
        let got = ease(easing(case["easing"].as_str().unwrap()), case["u"].as_f64().unwrap());
        case["expect"] = serde_json::json!(got);
    }
    for case in out["cases"].as_array_mut().expect("a case list") {
        let pattern: Pattern = serde_json::from_value(case["pattern"].clone()).expect("a pattern");
        let frames: Vec<SpinSource> = pattern.frames.iter().map(|f| SpinSource::new(f.colors.clone())).collect();
        let timeline = Timeline::of(&pattern);
        let spin = case["pattern"]["motion"]["turns_per_s"].as_f64();
        for at in case["at"].as_array_mut().unwrap() {
            let mut colours = Vec::new();
            eval(&frames, &timeline, at["t"].as_f64().unwrap(), spin, &mut colours);
            at["expect"] = Value::Array(colours.into_iter().map(|c| Value::String(hex(c))).collect());
        }
    }
    out
}

#[test]
fn the_checked_in_vectors_are_what_the_rust_engine_makes() {
    let text = std::fs::read_to_string(path()).expect("the vector file is read");
    let vectors: Value = serde_json::from_str(&text).expect("the vector file is JSON");
    let now = made(&vectors);
    if std::env::var_os("CANVAS_WRITE_VECTORS").is_some() {
        let mut pretty = serde_json::to_string_pretty(&now).unwrap();
        pretty.push('\n');
        std::fs::write(path(), pretty).expect("the vector file is written");
        return;
    }
    assert!(vectors["cases"].as_array().unwrap().len() >= 10);
    for (checked_in, made) in vectors["ease"].as_array().unwrap().iter().zip(now["ease"].as_array().unwrap()) {
        assert_eq!(checked_in["expect"], made["expect"], "ease: {checked_in}");
    }
    for (checked_in, made) in vectors["cases"].as_array().unwrap().iter().zip(now["cases"].as_array().unwrap()) {
        for (a, b) in checked_in["at"].as_array().unwrap().iter().zip(made["at"].as_array().unwrap()) {
            assert_eq!(a["expect"], b["expect"], "{}: t = {}", checked_in["name"], a["t"]);
        }
    }
}

/// Every pattern of the vectors is also one the daemon would save: its caps hold.
#[test]
fn every_vector_pattern_is_a_valid_pattern() {
    let text = std::fs::read_to_string(path()).expect("the vector file is read");
    let vectors: Value = serde_json::from_str(&text).unwrap();
    for case in vectors["cases"].as_array().unwrap() {
        let pattern: Pattern = serde_json::from_value(case["pattern"].clone()).unwrap();
        canvas_proto::pattern::validate_pattern(&pattern).unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
    }
}

/// The seam of a loop, measured: how far each channel moves between the picture just before the
/// cycle ends and the picture at its start. When the last frame fades into the first the move is
/// within one level for any distance from the seam down to a microsecond, which is the fade's own
/// slope and no jump; the same pattern with its last frame cutting back jumps by a whole fade.
///
/// Run with `--nocapture` to read the measurements.
#[test]
fn the_loop_seam_is_measured() {
    use canvas_proto::pattern::{BuiltOn, Frame, Playback, Transition, TransitionKind};
    use canvas_proto::scene::Arrangement;

    let colours = [[Rgb8::new(255, 0, 0), Rgb8::new(0, 255, 0)], [Rgb8::new(0, 0, 255), Rgb8::new(255, 255, 0)], [Rgb8::new(255, 255, 255), Rgb8::new(10, 20, 30)], [Rgb8::new(40, 200, 120), Rgb8::new(0, 0, 0)]];
    let make = |easing: Easing, last_fades: bool| -> Pattern {
        let frames = colours
            .iter()
            .enumerate()
            .map(|(i, c)| Frame {
                colors: c.to_vec(),
                hold: 300 + 50 * i as u32,
                transition: (i + 1 < colours.len() || last_fades)
                    .then_some(Transition { kind: TransitionKind::Fade, ms: 400, easing }),
                extra: Default::default(),
            })
            .collect();
        Pattern {
            id: "pattern:Seam".into(),
            built_on: BuiltOn { led_count: 2, arrangement: Arrangement::Strip, grid: None },
            frames,
            motion: canvas_proto::pattern::Motion::none(),
            playback: None::<Playback>,
            extra: Default::default(),
        }
    };
    let at = |p: &Pattern, t: f64| -> Vec<Rgb8> {
        let frames: Vec<SpinSource> = p.frames.iter().map(|f| SpinSource::new(f.colors.clone())).collect();
        let mut out = Vec::new();
        eval(&frames, &Timeline::of(p), t, None, &mut out);
        out
    };
    let delta = |a: &[Rgb8], b: &[Rgb8]| -> u8 {
        a.iter().zip(b).flat_map(|(x, y)| [x.r.abs_diff(y.r), x.g.abs_diff(y.g), x.b.abs_diff(y.b)]).max().unwrap_or(0)
    };
    let mut worst_loop = 0u8;
    let mut jump = 0u8;
    for easing in [Easing::Linear, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut] {
        let looping = make(easing, true);
        let cycle = Timeline::of(&looping).cycle_ms();
        let start = at(&looping, 0.0);
        for back in [1e-6, 1e-3, 0.1] {
            let d = delta(&at(&looping, cycle - back), &start);
            println!("{easing:?}: cycle {cycle} ms, {back} ms before the end vs the start: largest channel move {d}");
            worst_loop = worst_loop.max(d);
        }
        assert_eq!(at(&looping, cycle), start, "the cycle's end is the start, exactly");
        // The same frames with the last one cutting back: the seam is a jump by a whole fade.
        let cutting = make(easing, false);
        let c = Timeline::of(&cutting).cycle_ms();
        let d = delta(&at(&cutting, c - 1e-6), &at(&cutting, 0.0));
        println!("{easing:?}: the same pattern with the last frame cutting back jumps {d}");
        jump = jump.max(d);
    }
    println!("worst move across the loop seam: {worst_loop}; across a cutting seam: {jump}");
    assert!(worst_loop <= 1, "a looping fade has no jump at the seam: {worst_loop}");
    assert!(jump > 100, "the cut-back control does jump, so the measure can see one: {jump}");
}
