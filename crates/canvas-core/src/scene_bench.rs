//! The many-groups benchmark for Scenes (0.14.0).
//!
//! Run it with:
//!
//! ```text
//! cargo test -p canvas-core --release -- --ignored --nocapture --test-threads=1 bench_scene
//! ```
//!
//! A scene frame is: every group runs its own effect on its own canvas, and every
//! group's canvas is sampled for that group's LEDs. This measures what that
//! frame costs, for 1, 8 and 20 groups, in three sets:
//!
//! - **all built-in**: every group runs the same native Rust effect (`aurora`).
//! - **all HTML**: every group runs the daemon's own stock HTML effect (`ink-drop`),
//!   which is one QuickJS runtime each. HTML is the risk: a stack allows only two of
//!   them for that reason (`STACK_HTML_MAX`).
//! - **half and half**: the groups alternate, starting with the built-in one, so one
//!   group is one built-in group and this set is then the same as the all-built-in one.
//!
//! The frame step is 1/30 s, the engine's default frame rate. An HTML effect runs at
//! 60 Hz internally, so at 1/30 s each HTML group runs two of its own script frames per
//! engine frame.
//!
//! The sampling is area average over a strip of 60 LEDs. Which of the three sampling
//! modes is the most expensive is **unknown**: only area average was measured.
//!
//! A second test measures the mixed compositions the scene limit has to cover: 20
//! groups with one HTML group in them, and with two.
//!
//! # What a scene frame does not cover
//!
//! The frame timed here is the group's own work: every effect's `update` and every
//! group's LED sampling. The engine's frame does more around it (the brightness and
//! warmth stages, the per-device packing, the last-good copy and the sends to the
//! devices), and that part is **not** timed here. The bar therefore has to be
//! re-checked through the real engine (`CoreEngine`) when the scene engine exists;
//! until then every number below is a lower bound on a scene frame.
//!
//! # The render scale
//!
//! The render scale is the other half of a group's cost, and it is the engine's own
//! setting (`Canvas::with_scale`): 1.0 is a 320x200 canvas, 0.5 is 160x100 and 0.25 is
//! 80x50. The three stock HTML effects draw an 80x50 picture, so at a quarter scale the
//! canvas is exactly the picture's own size and nothing is scaled up on the way to the
//! lights. The tests below measure what each scale costs:
//!
//! - one HTML group alone, for each of the three stock effects, at each scale;
//! - the split between the script's own work and the runtime's floor (an effect whose
//!   script draws nothing), at each scale;
//! - the LED count of a group, at the scale the mixed table uses.
//!
//! # The stock-effect table
//!
//! The limit on HTML groups per scene has to hold for every stock effect, not for
//! `ink-drop` alone, so the table measures 20 groups with `k` of them HTML, for `k` from
//! 0 to 4 with no early stop, for each of the three stock effects and for the worst
//! case (two different effects in one scene), at the two scales the decision uses (0.25
//! and 0.5).
//!
//! Two things make that table honest:
//!
//! - **The window is 900 frames, 30 s of scene time.** Palette Knife's cycle is 24 s at
//!   its default speed (its own clock, `crates/canvas-daemon/stock-effects/palette-knife.html`:
//!   one cycle is `3 * (HOLD + TRANS)` T-units and the clock runs `sp / 60` a frame), so
//!   a 300-frame window (10 s) can sit inside one part of the cycle and miss the rest.
//! - **The table is run three times and no row is averaged.** A row is a measurement
//!   only if every run agrees. A run with a dropped frame (a script that ran past the
//!   runtime's own budget) or a load spike is reported and run again, never averaged
//!   in.
//!
//! # The engine's own clock
//!
//! The tables above step every effect by a fixed 1/30 s. The engine does not: it
//! starts every frame on a tick schedule and steps the effect by the real time between
//! two frame starts (`crates/canvas-core/src/engine.rs`, the render loop, and
//! `crate::timer::FrameTimer`, the timer it sleeps on). That step carries the timer
//! jitter, and the HTML runtime turns the step into whole 60 Hz script frames
//! (`canvas-html`'s accumulator), so a step that runs a little over 1/30 s makes the
//! runtime run a third script frame now and then. The engine-clock table measures what
//! that costs, on the same rows.
//!
//! # A measurement that the machine spoiled
//!
//! The HTML runtime gives every script frame a budget
//! (`canvas_html::FRAME_BUDGET_MS`, 100 ms). A script frame that runs past it is
//! dropped and the group draws nothing for that frame, so a run on a busy machine
//! measures the load and not the effect. Every measurement therefore says how many
//! groups dropped a frame, and a measurement with a drop in it is not a measurement of
//! the effect's cost.

use std::time::{Duration, Instant};

use canvas_proto::color::Rgb8;
use canvas_proto::layout::{SamplingMode, CANVAS_HEIGHT, CANVAS_WIDTH};
use canvas_proto::traits::{Canvas, Effect, Inputs};
use canvas_proto::{PropertySchema, Props};

use crate::geometry::LedSample;
use crate::sampling;
use crate::timer::FrameTimer;

/// The frames timed after the warm-up.
const FRAMES: usize = 300;

/// The frames timed in the stock-effect table.
///
/// 900 frames is 30 s of scene time at 30 fps, which is what Palette Knife's 24 s
/// cycle needs to be inside the window.
const STOCK_FRAMES: usize = 900;

/// How many times the stock-effect table is run: a row is a measurement only if every
/// run agrees, so the table is run three times and no row is averaged.
const STOCK_RUNS: usize = 3;

/// The frames run before the timing starts, so the pages are faulted in.
const WARMUP: usize = 30;

/// The LEDs sampled from every group's canvas per frame.
const LEDS_PER_GROUP: usize = 60;

/// The engine's step at 30 fps, its default frame rate.
const DT_30: f32 = 1.0 / 30.0;

/// The engine's step at 60 fps, its fastest rate.
const DT_60: f32 = 1.0 / 60.0;

/// The engine's frame rate the engine-clock table runs at.
const ENGINE_FPS: u32 = 30;

/// The group counts measured.
const GROUP_COUNTS: [usize; 3] = [1, 8, MAX_GROUPS];

/// The largest group count measured: the number of groups one scene promises to hold.
const MAX_GROUPS: usize = 20;

/// The built-in effect measured.
const BUILTIN_ID: &str = "aurora";

/// The HTML effect measured: the daemon's own stock effect, read at build time so the
/// benchmark measures the file the daemon ships, not a copy of it.
const INK_DROP_HTML: &str = include_str!("../../canvas-daemon/stock-effects/ink-drop.html");

/// The id the daemon gives that file.
const INK_DROP_ID: &str = "html:ink-drop";

/// The other two stock effects, read the same way.
const PALETTE_KNIFE_HTML: &str =
    include_str!("../../canvas-daemon/stock-effects/palette-knife.html");
const PALETTE_KNIFE_ID: &str = "html:palette-knife";
const WET_SMEAR_HTML: &str = include_str!("../../canvas-daemon/stock-effects/wet-smear.html");
const WET_SMEAR_ID: &str = "html:wet-smear";

/// The three stock effects: the id the daemon gives the file and the file itself.
const STOCK_EFFECTS: [(&str, &str); 3] = [
    (INK_DROP_ID, INK_DROP_HTML),
    (PALETTE_KNIFE_ID, PALETTE_KNIFE_HTML),
    (WET_SMEAR_ID, WET_SMEAR_HTML),
];

/// A minimal HTML file whose script draws nothing: the runtime's floor.
///
/// The file is a plain HTML document with the same canvas as the stock effects, whose
/// animation frame only asks for the next frame. The runtime still runs the script,
/// still runs its frame call and still copies the raster into the engine's canvas; it
/// just paints no pixels. What the stock effects cost on top of this floor is what
/// their scripts and their pixels cost.
///
/// The file declares no properties, so it is not a SignalRGB-format effect: it is a
/// smaller file that loads the same way and runs the same runtime.
const NOOP_HTML: &str = r#"<head>
  <title>Nothing</title>
</head>
<body style="margin: 0; padding: 0">
  <canvas id="stage" width="80" height="50"></canvas>
</body>
<script>
function draw() { window.requestAnimationFrame(draw); }
window.requestAnimationFrame(draw);
</script>
"#;

/// The id the benchmark gives the no-op file.
const NOOP_ID: &str = "html:nothing";

/// The render scales measured: the engine's own render scale, full, half and a quarter.
const SCALES: [f32; 3] = [1.0, 0.5, 0.25];

/// The HTML group counts the mixed table prints.
const K_TABLE: [usize; 6] = [1, 2, 3, 4, 6, 8];

/// The LED counts per group the sampling table prints.
const LED_COUNTS: [usize; 3] = [12, 24, 60];

/// The bar: half a 30 fps frame, in milliseconds.
const BAR_MS: f64 = 16.6;

/// Which effect a group runs.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A native Rust effect.
    Builtin,
    /// An HTML effect: one QuickJS runtime each. The id and the source are the file's.
    Html(&'static str, &'static str),
}

/// Which effects the groups of one measured set run.
#[derive(Clone, Copy)]
enum Set {
    /// Every group runs the built-in effect.
    AllBuiltin,
    /// Every group runs the HTML effect.
    AllHtml,
    /// The groups alternate, starting with the built-in one. With one group this set
    /// is the all-built-in one.
    Half,
}

impl Set {
    /// The name the results are printed under.
    fn name(self) -> &'static str {
        match self {
            Set::AllBuiltin => "all built-in (aurora)",
            Set::AllHtml => "all HTML (ink-drop)",
            Set::Half => "half built-in, half HTML",
        }
    }

    /// The effect the group at `index` runs.
    fn kind(self, index: usize) -> Kind {
        match self {
            Set::AllBuiltin => Kind::Builtin,
            Set::AllHtml => Kind::Html(INK_DROP_ID, INK_DROP_HTML),
            Set::Half => match index % 2 {
                0 => Kind::Builtin,
                _ => Kind::Html(INK_DROP_ID, INK_DROP_HTML),
            },
        }
    }
}

/// The HTML effects one measured scene mixes.
///
/// A scene's HTML groups do not have to run the same effect, so the table measures each
/// stock effect on its own and the worst case: the two effects with the highest p95
/// (`wet-smear` and `palette-knife`), in turn.
struct Mix {
    /// The name the table prints.
    name: &'static str,
    /// The effect the first HTML group runs.
    first: (&'static str, &'static str),
    /// The effect the second HTML group runs, and so on in turn.
    second: (&'static str, &'static str),
}

/// The mixes the stock-effect table measures.
const MIXES: [Mix; 4] = [
    Mix {
        name: "ink-drop",
        first: (INK_DROP_ID, INK_DROP_HTML),
        second: (INK_DROP_ID, INK_DROP_HTML),
    },
    Mix {
        name: "palette-knife",
        first: (PALETTE_KNIFE_ID, PALETTE_KNIFE_HTML),
        second: (PALETTE_KNIFE_ID, PALETTE_KNIFE_HTML),
    },
    Mix {
        name: "wet-smear",
        first: (WET_SMEAR_ID, WET_SMEAR_HTML),
        second: (WET_SMEAR_ID, WET_SMEAR_HTML),
    },
    Mix {
        name: "wet-smear + palette-knife",
        first: (WET_SMEAR_ID, WET_SMEAR_HTML),
        second: (PALETTE_KNIFE_ID, PALETTE_KNIFE_HTML),
    },
];

/// The scales the stock-effect table is measured at: the two the decision uses.
const STOCK_SCALES: [f32; 2] = [0.25, 0.5];

/// The HTML group counts the stock-effect table walks: 0 to 4, with no early stop, so
/// the table shows where each effect misses the bar and not only where it stops.
const STOCK_K: [usize; 5] = [0, 1, 2, 3, 4];

/// A group's effect: the built-in one or an HTML one.
///
/// The HTML effect is kept as its own type, not behind the trait, so the benchmark can
/// ask it whether it dropped a frame. A frame that runs past the runtime's own budget
/// is dropped and the group stops drawing for that frame, so the measurement of such a
/// run is the machine's load and not the effect's cost: it has to say so.
enum AnyEffect {
    /// A native Rust effect.
    Builtin(Box<dyn Effect>),
    /// An HTML effect: one QuickJS runtime each.
    Html(Box<canvas_html::HtmlEffect>),
}

impl AnyEffect {
    /// The effect's schema: the editor renders its controls from it.
    fn schema(&self) -> PropertySchema {
        match self {
            AnyEffect::Builtin(effect) => effect.schema(),
            AnyEffect::Html(effect) => effect.schema(),
        }
    }

    /// Called on activation and on every props change.
    fn init(&mut self, props: &Props) {
        match self {
            AnyEffect::Builtin(effect) => effect.init(props),
            AnyEffect::Html(effect) => effect.init(props),
        }
    }

    /// Draws one frame.
    fn update(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
        match self {
            AnyEffect::Builtin(effect) => effect.update(dt, canvas, inputs),
            AnyEffect::Html(effect) => effect.update(dt, canvas, inputs),
        }
    }

    /// Whether the effect dropped a frame for running past its budget.
    fn dropped_a_frame(&self) -> bool {
        match self {
            AnyEffect::Builtin(_) => false,
            AnyEffect::Html(effect) => effect.timed_out(),
        }
    }
}

/// One group of the measured scene: its effect, its canvas and its LEDs.
struct Group {
    effect: AnyEffect,
    canvas: Canvas,
    leds: Vec<LedSample>,
    /// The colours of the last sample; the buffer is reused so the hot path allocates
    /// nothing.
    out: Vec<Rgb8>,
}

impl Group {
    /// Builds one group with the effect of this kind, on a canvas at the given render
    /// scale, sampling the given number of LEDs.
    fn new(kind: Kind, scale: f32, leds: usize) -> Self {
        let mut effect = match kind {
            Kind::Builtin => AnyEffect::Builtin(
                canvas_effects::create(BUILTIN_ID).expect("the built-in effect builds"),
            ),
            Kind::Html(id, source) => AnyEffect::Html(Box::new(
                canvas_html::HtmlEffect::from_source(id, source, &Props::new())
                    .expect("the HTML effect loads"),
            )),
        };
        // The engine inits an effect when it is activated, with the effect's own
        // defaults when the owner has changed nothing.
        let schema = effect.schema();
        effect.init(&schema.defaults());
        Group {
            effect,
            canvas: Canvas::with_scale(scale),
            leds: led_strip(leds, scale),
            out: Vec::new(),
        }
    }
}

/// `count` LEDs in a row across the canvas at mid height: the plan a strip layout gives.
///
/// The regions are in device pixels, as the engine's own plan has them: the engine
/// multiplies the logical regions by the render scale before it samples them.
fn led_strip(count: usize, scale: f32) -> Vec<LedSample> {
    let width = CANVAS_WIDTH as f32 * scale;
    let height = CANVAS_HEIGHT as f32 * scale;
    let cell = width / count as f32;
    (0..count)
        .map(|i| {
            let x0 = i as f32 * cell;
            LedSample {
                center_x: x0 + cell / 2.0,
                center_y: height / 2.0,
                bounds: [x0, height / 2.0 - 1.0, x0 + cell, height / 2.0 + 1.0],
                sampled: true,
            }
        })
        .collect()
}

/// Runs one scene frame: every effect draws, every canvas is sampled for its LEDs.
fn frame(groups: &mut [Group], dt: f32, inputs: &Inputs) {
    for group in groups.iter_mut() {
        group.effect.update(dt, &mut group.canvas, inputs);
        sampling::sample_leds(
            &group.canvas.pixels,
            group.canvas.width,
            group.canvas.height,
            &group.leds,
            SamplingMode::AreaAverage,
            &mut group.out,
        );
    }
}

/// The nearest-rank percentile of the times: the value at `ceil(p * n)`, counted from
/// one. With 300 frames the p95 is the 285th smallest frame; with 900 frames it is
/// the 855th.
fn percentile(sorted: &[Duration], p: f64) -> f64 {
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1].as_secs_f64() * 1000.0
}

/// What one measurement found.
struct Measurement {
    /// The frames timed.
    frames: usize,
    /// The mean milliseconds per whole frame.
    mean: f64,
    /// The p95 milliseconds per whole frame.
    p95: f64,
    /// The HTML groups that dropped a frame for running past the runtime's budget.
    dropped: usize,
}

/// How the step between two frames is decided.
#[derive(Clone, Copy)]
enum Clock {
    /// A fixed step: every frame advances the effects by the same `dt`.
    Fixed(f32),
    /// The engine's own clock at `fps`: the frames start on a fixed tick schedule and
    /// the step is the real time between two frame starts.
    Engine(u32),
}

/// Measures one set of groups and returns what the measurement found.
fn measure(set: Set, count: usize, dt: f32) -> Measurement {
    let kinds: Vec<Kind> = (0..count).map(|i| set.kind(i)).collect();
    measure_kinds(&kinds, dt)
}

/// Measures the given groups and returns what the measurement found.
fn measure_kinds(kinds: &[Kind], dt: f32) -> Measurement {
    measure_kinds_at(kinds, dt, 1.0, LEDS_PER_GROUP)
}

/// Measures the given groups on canvases at the given render scale, with the given
/// number of LEDs per group, and returns what the measurement found.
fn measure_kinds_at(kinds: &[Kind], dt: f32, scale: f32, leds: usize) -> Measurement {
    measure_kinds_at_clock(kinds, Clock::Fixed(dt), scale, leds, FRAMES)
}

/// Measures the given groups on canvases at the given render scale, with the given
/// number of LEDs per group, over `frames` frames, on the given clock, and returns what
/// the measurement found.
fn measure_kinds_at_clock(
    kinds: &[Kind],
    clock: Clock,
    scale: f32,
    leds: usize,
    frames: usize,
) -> Measurement {
    let mut groups: Vec<Group> =
        kinds.iter().map(|kind| Group::new(*kind, scale, leds)).collect();
    // The engine reads its inputs once a frame and passes them to every effect. The
    // benchmark passes the default ones: the HTML runtime ignores them and `aurora`
    // ignores them too (`canvas-effects/src/aurora.rs`), so the engine's own
    // `build_inputs` would not change these numbers.
    let inputs = Inputs::default();

    let mut times: Vec<Duration> = Vec::with_capacity(frames);
    match clock {
        Clock::Fixed(dt) => {
            // Warm up, so the first timed frame is not the one that faults the pages in.
            for _ in 0..WARMUP {
                frame(&mut groups, dt, &inputs);
            }
            for _ in 0..frames {
                let start = Instant::now();
                frame(&mut groups, dt, &inputs);
                times.push(start.elapsed());
            }
        }
        Clock::Engine(fps) => {
            // The engine's own render loop: the frame starts, the step is the time
            // since the last frame started, the work runs, and the loop sleeps until
            // the next tick (`crates/canvas-core/src/engine.rs`, the render loop, and
            // `crate::timer::FrameTimer`, the timer it sleeps on). The timed span is
            // the work, as the engine times it (`frame_start.elapsed()`).
            let mut timer = FrameTimer::new();
            timer.acquire_resolution();
            let mut last_frame_at = Instant::now();
            for index in 0..(WARMUP + frames) {
                let frame_start = Instant::now();
                let dt = frame_start.duration_since(last_frame_at).as_secs_f32();
                last_frame_at = frame_start;
                let start = Instant::now();
                frame(&mut groups, dt, &inputs);
                let work = start.elapsed();
                if index >= WARMUP {
                    times.push(work);
                }
                timer.sleep_until_next(fps);
            }
            timer.release_resolution();
        }
    }

    let total: Duration = times.iter().sum();
    let mean = total.as_secs_f64() * 1000.0 / times.len() as f64;
    times.sort_unstable();
    // A group that dropped a frame did not draw it, so the frame it was in says
    // nothing about the effect's cost.
    let dropped = groups.iter().filter(|group| group.effect.dropped_a_frame()).count();
    Measurement { frames, mean, p95: percentile(&times, 0.95), dropped }
}

/// Prints the line under a measurement, the number of dropped frames on every row, and
/// the warning under it when a group dropped a frame: such a measurement is the
/// machine's load, not the effect's cost.
fn report(label: &str, m: &Measurement) {
    println!(
        "{label}: {frames} frames -> mean {mean:.3} ms/frame, p95 {p95:.3} ms/frame, {dropped} dropped frames",
        frames = m.frames,
        mean = m.mean,
        p95 = m.p95,
        dropped = m.dropped
    );
    if m.dropped > 0 {
        println!(
            "  {m} of the HTML groups dropped a frame: the script ran past the runtime's budget, so this measurement is the machine's load and not the effect's cost",
            m = m.dropped
        );
    }
}

/// A scene of the largest size: `k` groups run the HTML effect, the rest the built-in
/// one.
fn mixed_kinds(k: usize) -> Vec<Kind> {
    let mut kinds: Vec<Kind> = vec![Kind::Html(INK_DROP_ID, INK_DROP_HTML); k];
    kinds.resize(MAX_GROUPS, Kind::Builtin);
    kinds
}

/// The groups of a scene with `k` HTML groups running the mix's two effects in turn,
/// the rest running the built-in effect.
fn mix_kinds(mix: &Mix, k: usize) -> Vec<Kind> {
    let mut kinds: Vec<Kind> = (0..k)
        .map(|index| {
            if index % 2 == 0 {
                Kind::Html(mix.first.0, mix.first.1)
            } else {
                Kind::Html(mix.second.0, mix.second.1)
            }
        })
        .collect();
    kinds.resize(MAX_GROUPS, Kind::Builtin);
    kinds
}

/// The scenes benchmark: nine numbers per frame rate.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_groups() {
    for (label, dt) in [("30 fps", DT_30), ("60 fps", DT_60)] {
        println!();
        println!("scenes benchmark at {label} (dt {dt:.4} s):");
        for count in GROUP_COUNTS {
            for set in [Set::AllBuiltin, Set::AllHtml, Set::Half] {
                let m = measure(set, count, dt);
                report(&format!("{count:>2} groups, {:<24}", set.name()), &m);
            }
        }
    }
    println!();
}

/// The mixed compositions the scene limit has to cover: the largest scene, with the
/// most HTML groups the limit allows in it, and with one more than that.
///
/// The built-in effect is cheap and the HTML one is a QuickJS runtime each, so the
/// limit on HTML groups per scene is what decides whether a scene of 20 groups fits
/// in a 30 fps frame. These numbers are measured, not extrapolated from the per-group
/// costs above.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_groups_mixed() {
    for (label, dt) in [("30 fps", DT_30), ("60 fps", DT_60)] {
        println!();
        println!("scenes benchmark, mixed compositions at {label} (dt {dt:.4} s):");
        for html in [1usize, 2] {
            let mut kinds: Vec<Kind> = vec![Kind::Html(INK_DROP_ID, INK_DROP_HTML); html];
            kinds.resize(MAX_GROUPS, Kind::Builtin);
            let m = measure_kinds(&kinds, dt);
            report(
                &format!("{MAX_GROUPS:>2} groups, {html} HTML + {} built-in ", MAX_GROUPS - html),
                &m,
            );
        }
    }
    println!();
}

/// One HTML group alone, for each of the three stock effects, at each render scale.
///
/// The stock effects draw an 80x50 picture, so the render scale decides how big the
/// canvas the picture is drawn on and composited into is: 320x200 at full scale, and
/// the picture's own 80x50 at a quarter.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_html_by_scale() {
    println!();
    println!("scenes benchmark, one HTML group alone at 30 fps (dt {DT_30:.4} s):");
    for (id, source) in STOCK_EFFECTS {
        let name = id.trim_start_matches("html:");
        for scale in SCALES {
            let m = measure_kinds_at(&[Kind::Html(id, source)], DT_30, scale, LEDS_PER_GROUP);
            report(&format!("1 group, HTML {name:<14} at scale {scale:<4}"), &m);
        }
    }
    println!();
}

/// The split for one ink-drop group: the script's own work against the runtime's
/// floor, at each render scale.
///
/// `canvas-html` gives no seam for this: `HtmlEffect::update` runs the script and
/// copies the raster into the engine's canvas in one call, and neither half is timed
/// on its own. So the floor is measured with a file whose script draws nothing: the
/// same QuickJS runtime, the same frame call, the same copy of the raster, no
/// painting. What ink-drop costs on top of that floor is what its script and its
/// pixels cost. The floor is a lower bound for the copy: a transparent pixel is
/// cheaper to copy than an opaque one.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_html_split() {
    println!();
    println!("scenes benchmark, the HTML cost split at 30 fps (dt {DT_30:.4} s):");
    for scale in SCALES {
        let floor = measure_kinds_at(&[Kind::Html(NOOP_ID, NOOP_HTML)], DT_30, scale, LEDS_PER_GROUP);
        let ink = measure_kinds_at(&[Kind::Html(INK_DROP_ID, INK_DROP_HTML)], DT_30, scale, LEDS_PER_GROUP);
        report(
            &format!("scale {scale}: the runtime's floor (a script that draws nothing)"),
            &floor,
        );
        report(&format!("scale {scale}: ink-drop (the script and the pixels)"), &ink);
        println!(
            "scale {scale}: the script's own work, the difference of the two  : mean {:.3} ms/frame, p95 {:.3} ms/frame",
            ink.mean - floor.mean,
            ink.p95 - floor.p95
        );
    }
    println!();
}

/// The mixed scenes of the largest size at the full render scale: 20 groups, k of them
/// HTML (ink-drop) and the rest built-in, at 320x200.
///
/// The scale the decision does not use, measured so the table below has a row for it.
/// Every k from 1 to 4 is measured: the search does not stop at the first miss, so the
/// table says where the bar is missed and not only that it was.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_html_mix_by_scale() {
    println!();
    println!("scenes benchmark, 20 groups, k HTML + (20-k) built-in, at 30 fps (dt {DT_30:.4} s):");
    for scale in SCALES {
        // The built-in groups on their own: what a smaller canvas saves them.
        let builtin =
            measure_kinds_at(&vec![Kind::Builtin; MAX_GROUPS], DT_30, scale, LEDS_PER_GROUP);
        report(&format!("{MAX_GROUPS:>2} groups, all built-in (aurora) at scale {scale:<4}"), &builtin);

        let mut largest = 0usize;
        for k in STOCK_K {
            let m = measure_kinds_at(&mixed_kinds(k), DT_30, scale, LEDS_PER_GROUP);
            report(
                &format!(
                    "{MAX_GROUPS:>2} groups, {k} HTML + {} built-in at scale {scale:<4}",
                    MAX_GROUPS - k
                ),
                &m,
            );
            if m.p95 <= BAR_MS && m.dropped == 0 {
                largest = k;
            }
        }
        println!("scale {scale}: the largest k with a p95 at or under {BAR_MS} ms is {largest}");
    }
    println!();
}

/// The stock-effect table: 20 groups, k of them HTML, for every stock effect and for
/// the worst-case mix of two of them, at the two scales the decision uses.
///
/// The table is run three times and no row is averaged, and the walk over k does not
/// stop at the first miss: every k from 0 to 4 is measured, so the table says where
/// each effect misses the bar and not only where it stops.
fn stock_k_table(scales: &[f32]) {
    for run in 1..=STOCK_RUNS {
        println!();
        println!(
            "scenes benchmark, the stock-effect table, run {run} of {STOCK_RUNS} (dt {DT_30:.4} s, {STOCK_FRAMES} frames):"
        );
        for scale in scales {
            for mix in &MIXES {
                for k in STOCK_K {
                    let m = measure_kinds_at_clock(
                        &mix_kinds(mix, k),
                        Clock::Fixed(DT_30),
                        *scale,
                        LEDS_PER_GROUP,
                        STOCK_FRAMES,
                    );
                    report(
                        &format!(
                            "{MAX_GROUPS:>2} groups, {k} HTML + {} built-in, {} at scale {scale:<4}",
                            MAX_GROUPS - k,
                            mix.name
                        ),
                        &m,
                    );
                }
            }
        }
    }
    println!();
}

/// The stock-effect table, both scales, three runs.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_stock_k_table() {
    stock_k_table(&STOCK_SCALES);
}

/// The stock-effect table at a quarter scale, three runs.
///
/// The table is split by scale because one invocation of the whole table, three runs
/// over, is longer than the build lock lets one build hold the lock.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_stock_k_table_quarter() {
    stock_k_table(&[0.25]);
}

/// The stock-effect table at a half scale, three runs.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_stock_k_table_half() {
    stock_k_table(&[0.5]);
}

/// The stock-effect table on the engine's own clock: the frames start on the engine's
/// tick schedule and every effect is stepped by the real time between two frame
/// starts.
///
/// The engine's step carries the timer jitter, and the HTML runtime turns the step into
/// whole 60 Hz script frames, so a step that runs a little over 1/30 s makes the
/// runtime run a third script frame now and then. This table is what that costs.
fn stock_k_table_engine_clock(scales: &[f32]) {
    println!();
    println!(
        "scenes benchmark, the stock-effect table on the engine's clock ({ENGINE_FPS} fps, {STOCK_FRAMES} frames):"
    );
    for scale in scales {
        for mix in &MIXES {
            for k in STOCK_K {
                let m = measure_kinds_at_clock(
                    &mix_kinds(mix, k),
                    Clock::Engine(ENGINE_FPS),
                    *scale,
                    LEDS_PER_GROUP,
                    STOCK_FRAMES,
                );
                report(
                    &format!(
                        "{MAX_GROUPS:>2} groups, {k} HTML + {} built-in, {} at scale {scale:<4}",
                        MAX_GROUPS - k,
                        mix.name
                    ),
                    &m,
                );
            }
        }
    }
    println!();
}

/// The stock-effect table on the engine's own clock, both scales.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_stock_k_table_engine_clock() {
    stock_k_table_engine_clock(&STOCK_SCALES);
}

/// The stock-effect table on the engine's own clock, a quarter scale.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_stock_k_table_engine_clock_quarter() {
    stock_k_table_engine_clock(&[0.25]);
}

/// The stock-effect table on the engine's own clock, a half scale.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_stock_k_table_engine_clock_half() {
    stock_k_table_engine_clock(&[0.5]);
}

/// The LED count of a group, at the scale the mixed table uses: does the sampling
/// matter?
///
/// A real group is often a fan ring of 8 to 24 LEDs, so the sampling of a group costs
/// about as much as its effect does.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_led_count() {
    println!();
    println!("scenes benchmark, the LED count of a group at scale 0.25, at 30 fps (dt {DT_30:.4} s):");
    for leds in LED_COUNTS {
        for k in K_TABLE {
            let m = measure_kinds_at(&mixed_kinds(k), DT_30, 0.25, leds);
            report(
                &format!(
                    "{MAX_GROUPS:>2} groups, {k} HTML + {} built-in, {leds} LEDs a group",
                    MAX_GROUPS - k
                ),
                &m,
            );
        }
    }
    println!();
}
