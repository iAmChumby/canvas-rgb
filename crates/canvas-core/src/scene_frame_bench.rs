//! The whole-frame benchmark for Scenes (0.14.0), through the real `CoreEngine` scene path.
//!
//! The first benchmark (`scene_bench.rs` on the bench branch) timed only each group's effect
//! update and the sampling of its LEDs. A real frame also merges the lights into device frames,
//! applies brightness, warmth and white balance, packs and submits every device's frame. This
//! benchmark times all of it: it builds a scene of 20 groups on 20 devices, hands it to a spawned
//! `CoreEngine` with `set_scene`, lets the render thread run at 30 fps, and reads the engine's
//! own frame time.
//!
//! # What is measured
//!
//! The engine's own frame timing: the same number `stats().frame_time_ms` reports (the time from
//! the start of a frame to the end of its bookkeeping, sleep excluded), read for every frame
//! through a test-only hook in `finish_frame` instead of sampling the last one from another
//! thread. A frame covers taking the effects out of the state, every group's effect update, the
//! sampling, the merge into device frames, the global stages, the fade check, packing, and
//! submitting every device's frame to a sink, with the state lock taken the way production takes
//! it.
//!
//! Setup, the same for every run:
//! - 20 groups, one light each, on 20 devices of one 60-LED strip each, one sink per device (a
//!   sink that only counts: the real adapters' writer tasks are not part of the engine's frame);
//! - `k` groups play a stock HTML effect (the first `k` groups), the other `20 - k` play
//!   `aurora`;
//! - 30 fps, the default, with the global brightness at 80, a warmth of 4500 K and a white
//!   balance on every device, so the stages run (at their neutral settings they are skipped);
//! - 30 warm-up frames, then 900 timed frames, three runs, release build.
//!
//! Not in it: the editor's light preview (no sink set), the daemon's adapters, the HTTP layer.
//!
//! # Dropped frames
//!
//! Two kinds are counted per run. A group whose HTML script ran past the runtime's own budget
//! (`canvas_html::FRAME_BUDGET_MS`) drops that script frame and draws nothing; that is read from
//! the effect's own flag. A late frame is one whose measured time is longer than its 30 fps slot
//! (33.3 ms): the render thread missed a tick. Either one in a run says the run measured the
//! machine's load as well as the effect.
//!
//! Run with:
//!
//! ```text
//! cargo test -p canvas-core --release -- --ignored --nocapture --test-threads=1 bench_engine_scene_<name>
//! ```
//!
//! where `<name>` is `builtin`, `ink_drop`, `palette_knife` or `wet_smear`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canvas_proto::color::{Rgb8, WhiteBalance};
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::props::Props;
use canvas_proto::scene::{zone_light_id, Arrangement, GroupMode, Light};
use canvas_proto::traits::{
    Canvas, Effect, Engine, EngineGroup, EngineSettings, FrameSink, Inputs,
};
use canvas_proto::PropertySchema;

use crate::engine::spawn_core;

/// Groups in the scene: the number one scene promises to hold.
const GROUPS: usize = 20;
/// LEDs of each group's one light.
const LEDS: u32 = 60;
/// Frames run before the timing starts.
const WARMUP: usize = 30;
/// Frames timed after the warm-up.
const FRAMES: usize = 900;
/// Runs per cell.
const RUNS: usize = 3;
/// The engine's default frame rate.
const FPS: u32 = 30;
/// The bar: half a 30 fps frame, in ms.
const BAR_MS: f32 = 16.6;
/// A frame longer than its slot: the render thread missed a tick.
const SLOT_MS: f32 = 1000.0 / FPS as f32;
/// The HTML group counts measured.
const KS: [usize; 4] = [0, 1, 2, 3];

const INK_DROP: &str = include_str!("../../canvas-daemon/stock-effects/ink-drop.html");
const PALETTE_KNIFE: &str = include_str!("../../canvas-daemon/stock-effects/palette-knife.html");
const WET_SMEAR: &str = include_str!("../../canvas-daemon/stock-effects/wet-smear.html");

/// A sink that counts the frames it gets and keeps nothing.
#[derive(Default)]
struct CountSink {
    frames: AtomicU64,
}

impl FrameSink for CountSink {
    fn submit(&self, colors: Vec<Rgb8>) {
        std::hint::black_box(&colors);
        self.frames.fetch_add(1, Ordering::Relaxed);
    }
}

/// An HTML effect that records whether its script ever ran past the runtime's budget.
struct WatchedHtml {
    inner: canvas_html::HtmlEffect,
    dropped: Arc<AtomicBool>,
}

impl Effect for WatchedHtml {
    fn schema(&self) -> PropertySchema {
        self.inner.schema()
    }

    fn init(&mut self, props: &Props) {
        self.inner.init(props);
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
        self.inner.update(dt, canvas, inputs);
        if self.inner.timed_out() {
            self.dropped.store(true, Ordering::Relaxed);
        }
    }
}

fn device(index: usize) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(format!("bench-{index}")),
        name: format!("bench-{index}"),
        vendor: "bench".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: index as u32,
        serial: String::new(),
        location: String::new(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "strip".into(),
            kind: ZoneKind::Linear,
            led_count: LEDS,
            min_leds: LEDS,
            max_leds: LEDS,
            resizable: false,
        }],
        connected: true,
    }
}

fn light(device: &DeviceInfo) -> Light {
    Light {
        id: zone_light_id(&device.id, 0),
        device_id: device.id.clone(),
        zone_index: 0,
        leds: None,
        name: None,
        arrangement: Arrangement::Strip,
        reversed: false,
    }
}

/// The result of one run.
struct Run {
    mean: f32,
    p95: f32,
    max: f32,
    /// Frames longer than their 30 fps slot.
    late: usize,
    /// HTML groups whose script ran past the runtime's budget in this run.
    dropped_groups: usize,
}

/// Nearest-rank percentile of an ascending list.
fn percentile(sorted: &[f32], q: f32) -> f32 {
    let rank = (q * sorted.len() as f32).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// One run: a spawned engine with the scene set, 20 groups, `k` of them playing `html`.
fn run_once(html: Option<(&str, &str)>, k: usize) -> Run {
    let devices: Vec<DeviceInfo> = (0..GROUPS).map(device).collect();
    let lights: Vec<Light> = devices.iter().map(light).collect();
    let flags: Vec<Arc<AtomicBool>> = (0..k).map(|_| Arc::new(AtomicBool::new(false))).collect();

    let engine = spawn_core(EngineSettings {
        fps: FPS,
        power: canvas_proto::api::Power::Running,
        // Not 100 and not off: the global stages run only when a gain is below one.
        brightness: 80,
        warmth: 4500,
    });
    let times = Arc::new(Mutex::new(Vec::<f32>::with_capacity(WARMUP + FRAMES + 8)));
    {
        let times = Arc::clone(&times);
        engine.set_frame_time_hook(Some(Arc::new(move |ms| {
            times.lock().unwrap_or_else(|e| e.into_inner()).push(ms);
        })));
    }
    let balance = WhiteBalance { red: 0.95, green: 1.0, blue: 0.9 };
    engine.set_white_balance(devices.iter().map(|d| (d.id.clone(), balance)).collect());
    let sinks: Vec<Arc<CountSink>> = devices.iter().map(|_| Arc::new(CountSink::default())).collect();
    for (device, sink) in devices.iter().zip(&sinks) {
        engine.add_sink(device.id.clone(), sink.clone());
    }

    let groups: Vec<EngineGroup> = lights
        .iter()
        .enumerate()
        .map(|(i, light)| {
            let effect: Box<dyn Effect> = match html {
                Some((id, source)) if i < k => Box::new(WatchedHtml {
                    inner: canvas_html::HtmlEffect::from_source(id, source, &Props::new())
                        .expect("the stock HTML effect loads"),
                    dropped: Arc::clone(&flags[i]),
                }),
                _ => canvas_effects::create("aurora").expect("aurora builds"),
            };
            EngineGroup {
                id: format!("g{i}"),
                members: vec![light.id.clone()],
                mode: GroupMode::FlowAcross,
                brightness: 100,
                effect: Some(effect),
                props: Props::new(),
            }
        })
        .collect();
    engine.set_scene(lights, devices, groups, false);

    // Wait for the frames. A frame takes at most a few tens of ms, so 900 frames at 30 fps is
    // 30 s; the deadline is a hang guard, many times that.
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if times.lock().unwrap_or_else(|e| e.into_inner()).len() >= WARMUP + FRAMES {
            break;
        }
        assert!(Instant::now() < deadline, "the engine produced too few frames");
        std::thread::sleep(Duration::from_millis(100));
    }
    engine.shutdown();

    let all = times.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let timed = &all[WARMUP..WARMUP + FRAMES];
    let mean = timed.iter().sum::<f32>() / timed.len() as f32;
    let late = timed.iter().filter(|ms| **ms > SLOT_MS).count();
    let mut sorted = timed.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let delivered: u64 = sinks.iter().map(|s| s.frames.load(Ordering::Relaxed)).sum();
    assert!(delivered > 0, "no frame reached a sink");
    Run {
        mean,
        p95: percentile(&sorted, 0.95),
        max: *sorted.last().unwrap(),
        late,
        dropped_groups: flags.iter().filter(|f| f.load(Ordering::Relaxed)).count(),
    }
}

/// Runs every k for one effect, three runs each, and prints the lines the doc quotes.
fn bench(name: &str, html: Option<(&str, &str)>) {
    let ks: &[usize] = if html.is_some() { &KS } else { &KS[..1] };
    let mut largest_ok: Option<usize> = None;
    for &k in ks {
        let mut p95s = Vec::new();
        let mut clean = true;
        for run_number in 1..=RUNS {
            let run = run_once(html, k);
            println!(
                "{name} | {GROUPS} groups, {k} HTML + {} built-in | run {run_number} | {FRAMES} frames | mean {:.3} ms | p95 {:.3} ms | max {:.3} ms | late frames {} | HTML groups that dropped a script frame {}",
                GROUPS - k,
                run.mean,
                run.p95,
                run.max,
                run.late,
                run.dropped_groups,
            );
            p95s.push(run.p95);
            clean &= run.p95 <= BAR_MS;
        }
        let worst = p95s.iter().copied().fold(f32::MIN, f32::max);
        println!(
            "{name} | {k} HTML | worst p95 of {RUNS} runs {worst:.3} ms | {} the {BAR_MS} ms bar in all {RUNS} runs",
            if clean { "at or under" } else { "OVER" },
        );
        // The largest k that fits: every smaller k fits too.
        if clean && largest_ok.map_or(k == ks[0], |last| last + 1 == k) {
            largest_ok = Some(k);
        }
    }
    match largest_ok {
        Some(k) => println!("{name} | the largest k at or under {BAR_MS} ms p95 in all {RUNS} runs is {k}"),
        None => println!("{name} | no k at or under {BAR_MS} ms p95 in all {RUNS} runs"),
    }
}

#[test]
#[ignore]
fn bench_engine_scene_builtin() {
    bench("aurora only", None);
}

#[test]
#[ignore]
fn bench_engine_scene_ink_drop() {
    bench("ink-drop", Some(("html:ink-drop", INK_DROP)));
}

#[test]
#[ignore]
fn bench_engine_scene_palette_knife() {
    bench("palette-knife", Some(("html:palette-knife", PALETTE_KNIFE)));
}

#[test]
#[ignore]
fn bench_engine_scene_wet_smear() {
    bench("wet-smear", Some(("html:wet-smear", WET_SMEAR)));
}
