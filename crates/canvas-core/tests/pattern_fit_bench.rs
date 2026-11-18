//! What a pattern of many frames costs across the lights of a group (#481), measured on the fakes.
//!
//! A pattern that plays through its frames keeps every frame fitted to every light, once per
//! change, on the render thread. This binary puts one group of fake lights on the real engine and
//! reads two numbers for it: the heap the engine holds once the first frame has reached every
//! light, and the time from `set_scene` to that frame (which is the fit, on the render thread: the
//! lights stay dark until it is done). The allocator counts live bytes for every thread.
//!
//! Run in release; the tests are ignored in a debug run, where the numbers mean nothing:
//!
//! cargo test -p canvas-core --release --test pattern_fit_bench -- --ignored --nocapture --test-threads=1

use canvas_proto::color::Rgb8;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::pattern::{BuiltOn, Frame, Motion, Pattern};
use canvas_proto::props::{PropertySchema, Props};
use canvas_proto::scene::{zone_light_id, Arrangement, GroupMode, Light};
use canvas_proto::traits::{Canvas, Effect, EngineGroup, EngineSettings, FrameSink, Inputs};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Live heap bytes, and the most there has been since the last reset.
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

fn grew(by: usize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grew(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        grew(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size >= layout.size() {
            grew(new_size - layout.size());
        } else {
            LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A pattern as an effect: the engine reads its colours and draws nothing.
struct Reel {
    pattern: Pattern,
}

impl Effect for Reel {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: self.pattern.id.clone(),
            name: "Reel".into(),
            description: String::new(),
            needs: Default::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }
    fn init(&mut self, _props: &Props) {}
    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {}
    fn as_pattern(&self) -> Option<&Pattern> {
        Some(&self.pattern)
    }
}

/// A sink that keeps only whether a frame has arrived.
#[derive(Default)]
struct Seen(AtomicUsize);

impl FrameSink for Seen {
    fn submit(&self, _colors: Vec<Rgb8>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// `frames` frames of `leds` colours, no two alike, each held 10 ms so the pattern plays through them.
fn reel(frames: usize, leds: u32) -> Pattern {
    Pattern {
        id: "pattern:Reel".into(),
        built_on: BuiltOn { led_count: leds, arrangement: Arrangement::Strip, grid: None },
        frames: (0..frames)
            .map(|f| Frame {
                colors: (0..leds).map(|i| Rgb8::new((i % 256) as u8, (f % 256) as u8, ((i / 256) % 256) as u8)).collect(),
                hold: 10,
                transition: None,
                extra: Default::default(),
            })
            .collect(),
        motion: Motion::none(),
        playback: None,
        extra: Default::default(),
    }
}

fn device(index: usize, leds: u32) -> DeviceInfo {
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
            led_count: leds,
            min_leds: leds,
            max_leds: leds,
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
        kind: None,
        copies: None,
        first_led: None,
    }
}

/// One group of `sizes.len()` fake lights playing a `frames`-frame reel built on 4096 LEDs; each
/// light has the LED count of its entry. Returns (heap held, ms to the first frame on every light).
fn measure(frames: usize, sizes: &[u32]) -> (usize, u128) {
    let pattern = reel(frames, 4096);
    let devices: Vec<DeviceInfo> = sizes.iter().enumerate().map(|(i, leds)| device(i, *leds)).collect();
    let lights: Vec<Light> = devices.iter().map(light).collect();
    let engine = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    });
    let sinks: Vec<Arc<Seen>> = devices.iter().map(|_| Arc::new(Seen::default())).collect();
    for (device, sink) in devices.iter().zip(&sinks) {
        engine.add_sink(device.id.clone(), Arc::clone(sink) as Arc<dyn FrameSink>);
    }
    let group = EngineGroup {
        id: "reel".into(),
        members: lights.iter().map(|l| l.id.clone()).collect(),
        mode: GroupMode::FlowAcross,
        brightness: 100,
        effect: Some(Box::new(Reel { pattern: pattern.clone() })),
        props: Props::new(),
        sequence: None,
    };
    // What is held before the scene is set (the pattern the test itself holds is not the engine's).
    let before = LIVE.load(Ordering::Relaxed);
    let start = Instant::now();
    engine.set_scene(lights, devices, vec![group], false);
    let deadline = Instant::now() + Duration::from_secs(120);
    while sinks.iter().any(|s| s.0.load(Ordering::Relaxed) == 0) {
        if Instant::now() >= deadline {
            panic!("the engine never reached every light");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let took = start.elapsed().as_millis();
    let held = LIVE.load(Ordering::Relaxed).saturating_sub(before);
    engine.shutdown();
    drop(pattern);
    (held, took)
}

fn report(what: &str, frames: usize, sizes: &[u32]) -> (usize, u128) {
    let (held, ms) = measure(frames, sizes);
    println!(
        "pattern fit | {what} | {frames} frames x {} lights | engine heap {:.1} MB | first frame on every light after {ms} ms",
        sizes.len(),
        held as f64 / 1_048_576.0,
    );
    (held, ms)
}

#[test]
#[ignore = "a release-build measurement: cargo test -p canvas-core --release --test pattern_fit_bench -- --ignored --nocapture"]
fn the_cost_of_a_pattern_of_many_frames_across_the_lights_of_a_group() {
    // One light of the largest size: the per-light figure the issue quotes (28 MB, 124 ms).
    let (one, _) = report("1 light of 4096 LEDs", 256, &[4096]);
    // Ten lights of the one shape: they share one fitted set, so this is about one light's worth.
    let (ten_same, _) = report("10 lights of 4096 LEDs, one shape", 256, &[4096; 10]);
    // Ten lights of ten different sizes: each is its own fit, the cost the issue measured per light.
    let distinct: Vec<u32> = (0..10).map(|k| 4096 - k).collect();
    let (ten_distinct, _) = report("10 lights of 4096-4087 LEDs, ten shapes", 256, &distinct);
    // The realistic case: thirty lights of 12 to 300 LEDs.
    let realistic: Vec<u32> = (0..30).map(|k| 12 + (k * 10) as u32).collect();
    let (small, small_ms) = report("30 lights of 12-302 LEDs", 256, &realistic);
    println!(
        "pattern fit | ratios | ten of one shape = {:.2} x one light; ten shapes = {:.2} x one light; thirty small lights {:.1} MB, {small_ms} ms",
        ten_same as f64 / one as f64,
        ten_distinct as f64 / one as f64,
        small as f64 / 1_048_576.0,
    );
    // Ten lights of one shape hold about what one holds, not ten times it.
    assert!(
        ten_same < one * 3,
        "ten lights of one shape held {ten_same} bytes, one light {one}: the fit is not shared"
    );
}
