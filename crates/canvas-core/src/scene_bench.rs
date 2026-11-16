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
//! The sampling is area average over a strip of 60 LEDs (the other two modes are not
//! timed here, so it is not known to be the costliest). The
//! LED count test below varies it, to see what the sampling costs next to the effect.
//!
//! A second test measures the mixed compositions the scene limit has to cover: 20
//! groups with one HTML group in them, and with two.
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
//! - the mixed scenes of 20 groups with k HTML groups in them, at each scale, and the
//!   largest k that still fits in half a 30 fps frame;
//! - the LED count of a group, at the scale the mixed table uses.
//!
//! The bench is single-threaded, like the engine's render thread.
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

/// The frames timed after the warm-up.
const FRAMES: usize = 300;

/// The frames run before the timing starts, so the pages are faulted in.
const WARMUP: usize = 30;

/// The LEDs sampled from every group's canvas per frame.
const LEDS_PER_GROUP: usize = 60;

/// The engine's step at 30 fps, its default frame rate.
const DT_30: f32 = 1.0 / 30.0;

/// The engine's step at 60 fps, its fastest rate.
const DT_60: f32 = 1.0 / 60.0;

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

/// A minimal HTML effect (a script and a canvas, no meta tags) whose script draws nothing: the
/// runtime's floor.
///
/// The file is a stock-format effect with the same canvas as the stock effects, whose
/// animation frame only asks for the next frame. The runtime still runs the script,
/// still runs its frame call and still copies the raster into the engine's canvas; it
/// just paints no pixels. What the stock effects cost on top of this floor is what
/// their scripts and their pixels cost.
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

/// A group's effect: the built-in one or an HTML one.
///
/// The HTML effect is kept as its own type, not behind the trait, so the benchmark can
/// ask it whether it dropped a frame. A frame that runs past the runtime's own budget
/// is dropped and the group stops drawing for that frame, so the measurement of such
/// a run is the machine's load and not the effect's cost: it has to say so.
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
/// one. With 300 frames the p95 is the 285th of the frames sorted fastest first (`sorted[284]`).
fn percentile(sorted: &[Duration], p: f64) -> f64 {
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1].as_secs_f64() * 1000.0
}

/// What one measurement found.
struct Measurement {
    /// The mean milliseconds per whole frame.
    mean: f64,
    /// The p95 milliseconds per whole frame.
    p95: f64,
    /// The HTML groups that dropped a frame for running past the runtime's budget.
    dropped: usize,
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
    let mut groups: Vec<Group> =
        kinds.iter().map(|kind| Group::new(*kind, scale, leds)).collect();
    let inputs = Inputs::default();

    // Warm up, so the first timed frame is not the one that faults the pages in.
    for _ in 0..WARMUP {
        frame(&mut groups, dt, &inputs);
    }

    let mut times: Vec<Duration> = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let start = Instant::now();
        frame(&mut groups, dt, &inputs);
        times.push(start.elapsed());
    }

    let total: Duration = times.iter().sum();
    let mean = total.as_secs_f64() * 1000.0 / times.len() as f64;
    times.sort_unstable();
    // A group that dropped a frame did not draw it, so the frame it was in says
    // nothing about the effect's cost.
    let dropped = groups.iter().filter(|group| group.effect.dropped_a_frame()).count();
    Measurement { mean, p95: percentile(&times, 0.95), dropped }
}

/// Prints the line under a measurement, and the warning under it when a group dropped
/// a frame: such a measurement is the machine's load, not the effect's cost.
fn report(label: &str, m: &Measurement) {
    println!(
        "{label}: {FRAMES} frames -> mean {:.3} ms/frame, p95 {:.3} ms/frame",
        m.mean, m.p95
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

/// The scenes benchmark: nine numbers per frame rate.
#[test]
#[ignore = "a benchmark, not a correctness test"]
fn bench_scene_groups() {
    for (label, dt) in [("30 fps", DT_30), ("60 fps", DT_60)] {
        println!();
        println!("scenes benchmark at {label} (dt {:.4} s):", dt);
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
/// on its own. So the floor is measured with an effect whose script draws nothing:
/// the same QuickJS runtime, the same frame call, the same copy of the raster, no
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

/// The mixed scenes of the largest size: 20 groups, k of them HTML (ink-drop) and the
/// rest built-in, at each render scale.
///
/// The limit on HTML groups per scene depends on the render scale, so the table is
/// measured at each scale. The search walks k up until one k misses the bar, so the
/// largest k that fits is measured and not guessed.
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
        for k in 1..=MAX_GROUPS {
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
            } else {
                break;
            }
        }
        println!("scale {scale}: the largest k with a p95 at or under {BAR_MS} ms is {largest}");
    }
    println!();
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
