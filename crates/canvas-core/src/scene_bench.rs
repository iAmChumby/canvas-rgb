//! The many-groups benchmark for Scenes (0.14.0).
//!
//! Run it with:
//!
//! ```text
//! cargo test -p canvas-core --release -- --ignored --nocapture bench_scene_groups
//! ```
//!
//! A scene frame is: every group runs its own effect on its own 320x200 canvas, and
//! every group's canvas is sampled for that group's LEDs. This measures what that
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
//! The sampling is area average over a strip of 60 LEDs, the most expensive of the
//! three sampling modes, so the numbers are an upper bound on the sampling cost.
//!
//! A second test measures the mixed compositions the scene limit has to cover: 20
//! groups with one HTML group in them, and with two.

use std::time::{Duration, Instant};

use canvas_proto::color::Rgb8;
use canvas_proto::layout::{SamplingMode, CANVAS_HEIGHT, CANVAS_WIDTH};
use canvas_proto::traits::{Canvas, Effect, Inputs};
use canvas_proto::Props;

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

/// Which effect a group runs.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A native Rust effect.
    Builtin,
    /// An HTML effect: one QuickJS runtime each.
    Html,
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
            Set::AllHtml => Kind::Html,
            Set::Half => match index % 2 {
                0 => Kind::Builtin,
                _ => Kind::Html,
            },
        }
    }
}

/// One group of the measured scene: its effect, its canvas and its LEDs.
struct Group {
    effect: Box<dyn Effect>,
    canvas: Canvas,
    leds: Vec<LedSample>,
    /// The colours of the last sample; the buffer is reused so the hot path allocates
    /// nothing.
    out: Vec<Rgb8>,
}

impl Group {
    /// Builds one group with the effect of this kind.
    fn new(kind: Kind) -> Self {
        let mut effect: Box<dyn Effect> = match kind {
            Kind::Builtin => canvas_effects::create(BUILTIN_ID).expect("the built-in effect builds"),
            Kind::Html => Box::new(
                canvas_html::HtmlEffect::from_source(INK_DROP_ID, INK_DROP_HTML, &Props::new())
                    .expect("the HTML effect loads"),
            ),
        };
        // The engine inits an effect when it is activated, with the effect's own
        // defaults when the owner has changed nothing.
        let schema = effect.schema();
        effect.init(&schema.defaults());
        Group {
            effect,
            canvas: Canvas::new(CANVAS_WIDTH, CANVAS_HEIGHT),
            leds: led_strip(LEDS_PER_GROUP),
            out: Vec::new(),
        }
    }
}

/// 60 LEDs in a row across the canvas at mid height: the plan a strip layout gives.
fn led_strip(count: usize) -> Vec<LedSample> {
    let width = CANVAS_WIDTH as f32;
    let height = CANVAS_HEIGHT as f32;
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
/// one. With 300 frames the p95 is the 285th slowest frame.
fn percentile(sorted: &[Duration], p: f64) -> f64 {
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1].as_secs_f64() * 1000.0
}

/// Measures one set of groups and returns the mean and the p95 in milliseconds per
/// whole frame (every group).
fn measure(set: Set, count: usize, dt: f32) -> (f64, f64) {
    let kinds: Vec<Kind> = (0..count).map(|i| set.kind(i)).collect();
    measure_kinds(&kinds, dt)
}

/// Measures the given groups and returns the mean and the p95 in milliseconds per
/// whole frame (every group).
fn measure_kinds(kinds: &[Kind], dt: f32) -> (f64, f64) {
    let mut groups: Vec<Group> = kinds.iter().map(|kind| Group::new(*kind)).collect();
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
    (mean, percentile(&times, 0.95))
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
                let (mean, p95) = measure(set, count, dt);
                println!(
                    "{count:>2} groups, {:<24}: {FRAMES} frames -> mean {mean:.3} ms/frame, p95 {p95:.3} ms/frame",
                    set.name()
                );
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
            let mut kinds: Vec<Kind> = vec![Kind::Html; html];
            kinds.resize(MAX_GROUPS, Kind::Builtin);
            let (mean, p95) = measure_kinds(&kinds, dt);
            println!(
                "{MAX_GROUPS:>2} groups, {html} HTML + {} built-in  : {FRAMES} frames -> mean {mean:.3} ms/frame, p95 {p95:.3} ms/frame",
                MAX_GROUPS - html
            );
        }
    }
    println!();
}
