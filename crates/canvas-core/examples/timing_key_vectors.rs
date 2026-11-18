//! Prints pairs of patterns and the daemon's same-timing verdict for each as JSON.
//!
//! The editor copies the engine's timing comparison in `app/web/src/lib/patternDraft.ts`
//! (`timingKey`) so a colour-only stroke keeps a running clock. The two copies are tied together by
//! `app/web/src/lib/__tests__/timingKey.vectors.json`: this printer writes the file, the editor's
//! test runs `timingKey` over every pair, and `crates/canvas-core/tests/timing_key_vectors.rs`
//! fails when the checked-in file is no longer what the Rust rule makes.
//!
//! Every pair differs in at most one field of `PatternTiming`, one at a time; the pairs cover every
//! field. The fields come from the rule itself (`canvas_core::pattern_timing_fields`): a field added
//! to `PatternTiming` does not compile there until this printer writes it down, and the editor's
//! test names every field, so none can drift past the editor unseen.
//!
//! Print it and write the file with:
//! `powershell -NoProfile -File tools/timing-key-vectors.ps1`

use canvas_proto::color::Rgb8;
use canvas_proto::pattern::{
    BuiltOn, Easing, Frame, Motion, Pattern, Playback, Transition, TransitionKind,
};
use canvas_proto::scene::Arrangement;
use serde_json::json;

/// The pattern the pairs start from: two frames, a spin, a looping playback and a fade, so every
/// field of `PatternTiming` has something to change.
fn base() -> Pattern {
    let colors = |r: u8, g: u8, b: u8| vec![Rgb8::new(r, g, b), Rgb8::new(b, r, g)];
    Pattern {
        id: "pattern:Base".into(),
        built_on: BuiltOn { led_count: 2, arrangement: Arrangement::Ring, grid: None },
        frames: vec![
            Frame {
                colors: colors(200, 0, 0),
                hold: 300,
                transition: Some(Transition { kind: TransitionKind::Fade, ms: 400, easing: Easing::EaseInOut }),
                extra: serde_json::Map::new(),
            },
            Frame {
                colors: colors(0, 200, 0),
                hold: 500,
                transition: Some(Transition { kind: TransitionKind::Cut, ms: 0, easing: Easing::Linear }),
                extra: serde_json::Map::new(),
            },
        ],
        motion: Motion::spin(0.25),
        playback: Some(Playback { looping: true }),
        extra: serde_json::Map::new(),
    }
}

/// One pair, and what it is meant to show. `setup` prepares both documents the same way, so a pair
/// can also start from a state the base does not have: a frame with no transition, playback off.
struct Case {
    name: &'static str,
    setup: Option<fn(&mut Pattern)>,
    change: fn(&mut Pattern),
}

/// A pair whose two documents start from [`base`].
fn pair(name: &'static str, change: fn(&mut Pattern)) -> Case {
    Case { name, setup: None, change }
}

/// A pair whose two documents are prepared by `setup` first, then differ by `change`.
fn pair_after(name: &'static str, setup: fn(&mut Pattern), change: fn(&mut Pattern)) -> Case {
    Case { name, setup: Some(setup), change }
}

/// The pairs: controls that must keep the clock, then one change for every timing field.
fn cases() -> Vec<Case> {
    vec![
        pair("only the colours change", |p| p.frames[0].colors[0] = Rgb8::new(0, 0, 255)),
        pair("only built_on and extra change", |p| {
            p.built_on.led_count = 12;
            p.extra.insert("note".into(), json!("a later version wrote this"));
        }),
        pair("the id changes", |p| p.id = "pattern:Other".into()),
        pair("the motion kind changes", |p| p.motion = Motion::none()),
        pair("the motion speed changes", |p| p.motion = Motion::spin(0.5)),
        pair("playback goes away", |p| p.playback = None),
        pair("playback stops looping", |p| p.playback = Some(Playback { looping: false })),
        // The other direction of each option: a document that says there is no transition, or no
        // playback, against one that says there is but not to loop.
        pair("a frame's transition goes away", |p| p.frames[0].transition = None),
        pair_after(
            "a frame gains a transition",
            |p| p.frames[0].transition = None,
            |p| {
                p.frames[0].transition =
                    Some(Transition { kind: TransitionKind::Fade, ms: 400, easing: Easing::EaseInOut })
            },
        ),
        pair_after(
            "playback is nothing instead of set not to loop",
            |p| p.playback = Some(Playback { looping: false }),
            |p| p.playback = None,
        ),
        pair("a frame's hold changes", |p| p.frames[0].hold += 1),
        pair("a frame's transition kind changes", |p| {
            p.frames[0].transition.as_mut().expect("a fade").kind = TransitionKind::Cut
        }),
        pair("a frame's transition ms changes", |p| {
            p.frames[0].transition.as_mut().expect("a fade").ms += 1
        }),
        pair("a frame's transition easing changes", |p| {
            p.frames[0].transition.as_mut().expect("a fade").easing = Easing::EaseOut
        }),
        pair("a frame is added", |p| p.frames.push(p.frames[1].clone())),
        pair("a frame is removed", |p| { p.frames.pop(); }),
    ]
}

/// The engine's timing of `pattern` as JSON, field by field: the values the engine compares when it
/// decides whether an install keeps the clock. The fields come from the rule itself
/// (`canvas_core::pattern_timing_fields`); this printer only writes them down.
fn timing_json(pattern: &Pattern) -> serde_json::Value {
    // Destructured, not read field by field: a field added to the rule's plain fields breaks this
    // line until the printer writes it down, so no timing field can be missing from the vectors.
    let canvas_core::PatternTimingFields { id, motion, playback, frames } =
        canvas_core::pattern_timing_fields(pattern);
    json!({
        "id": id,
        "motion": motion,
        "playback": playback,
        "frames": frames,
    })
}

/// The whole file: every pair with both documents, both timings and the verdict.
///
/// Public so the parity test (`crates/canvas-core/tests/timing_key_vectors.rs`) can ask the printer
/// what it makes today and compare it with the checked-in file.
pub fn print_json() -> String {
    let mut pairs = Vec::new();
    for case in cases() {
        let mut a = base();
        if let Some(setup) = case.setup {
            setup(&mut a);
        }
        let mut b = a.clone();
        (case.change)(&mut b);
        let timing_a = timing_json(&a);
        let timing_b = timing_json(&b);
        pairs.push(json!({
            "name": case.name,
            "a": a,
            "b": b,
            "timing_a": timing_a,
            "timing_b": timing_b,
            "same_timing": timing_a == timing_b,
        }));
    }
    let file = json!({
        "note": "Pairs of patterns and the daemon's same-timing verdict for each. Printed by crates/canvas-core/examples/timing_key_vectors.rs; tools/timing-key-vectors.ps1 writes this file. The editor's timingKey (app/web/src/lib/patternDraft.ts) is checked against it, and the timing objects printed here are the fields of the engine's PatternTiming.",
        "pairs": pairs,
    });
    let mut out = serde_json::to_string_pretty(&file).expect("the vectors are JSON");
    out.push('\n');
    out
}

fn main() {
    let json = print_json();
    // With a path, the file is written there (tools/timing-key-vectors.ps1 does that); without
    // one, the JSON goes to the standard output.
    let mut args = std::env::args().skip(1);
    match args.next() {
        Some(path) => {
            if let Err(err) = std::fs::write(&path, &json) {
                eprintln!("timing_key_vectors: cannot write {path}: {err}");
                std::process::exit(1);
            }
            eprintln!("timing_key_vectors: wrote {path} ({} bytes)", json.len());
        }
        None => print!("{json}"),
    }
}
