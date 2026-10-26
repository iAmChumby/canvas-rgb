//! A criterion-free micro-benchmark of the sampling path.
//!
//! Run it with:
//!
//! ```text
//! cargo test -p canvas-core --release -- --ignored --nocapture bench
//! ```
//!
//! It times 1000 frames of sampling ~600 LEDs from a 320x200 canvas and prints the
//! nanoseconds per frame. The budget from the spec is "the whole stack renders Poison
//! at 30 fps under 0.5% CPU of a 16-thread machine", so the sampling path has a very
//! small per-frame allowance.

use std::time::Instant;

use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::{Layout, PlacedZone, RenderScale, SamplingMode};
use canvas_proto::color::Rgb8;

use crate::geometry;
use crate::sampling;

/// The number of frames the benchmark times.
const FRAMES: usize = 1000;

/// The number of LEDs the benchmark samples per frame.
const LED_COUNT: u32 = 600;

/// Builds a device with `LED_COUNT` LEDs spread over a few linear zones.
fn bench_device(zone_count: u32, leds_per_zone: u32) -> DeviceInfo {
    let zones: Vec<ZoneInfo> = (0..zone_count)
        .map(|index| ZoneInfo {
            index,
            name: format!("zone{index}"),
            kind: ZoneKind::Linear,
            led_count: leds_per_zone,
            min_leds: leds_per_zone,
            max_leds: leds_per_zone,
            resizable: false,
        })
        .collect();
    DeviceInfo {
        id: DeviceId("bench".into()),
        name: "bench".into(),
        vendor: "bench".into(),
        kind: DeviceKind::Other,
        adapter: "bench".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones,
        connected: true,
    }
}

/// Builds a layout that covers the whole canvas with the device's zones.
fn bench_layout(device: &DeviceInfo) -> Layout {
    let zone_count = device.zones.len() as f32;
    let width = canvas_proto::CANVAS_WIDTH as f32 / zone_count;
    let zones = device
        .zones
        .iter()
        .map(|z| PlacedZone {
            device_id: device.id.clone(),
            zone_index: z.index,
            x: z.index as f32 * width,
            y: 0.0,
            width,
            height: canvas_proto::CANVAS_HEIGHT as f32,
            rotation_deg: 0.0,
            reversed: false,
            visible: true,
        })
        .collect();
    Layout {
        id: "bench".into(),
        name: "bench".into(),
        render_scale: RenderScale::P100,
        sampling: SamplingMode::AreaAverage,
        zones,
        effect: None,
    }
}

/// Runs the benchmark for one sampling mode.
fn bench_mode(mode: SamplingMode) {
    let zone_count = 6;
    let leds_per_zone = LED_COUNT / zone_count;
    let device = bench_device(zone_count, leds_per_zone);
    let mut layout = bench_layout(&device);
    layout.sampling = mode;

    let plan = geometry::build_plan(&layout, std::slice::from_ref(&device));
    let canvas = canvas_proto::traits::Canvas::with_scale(plan.scale);
    let mut out: Vec<Rgb8> = Vec::new();

    // Warm up, so the first timed frame is not the one that faults the pages in.
    for _ in 0..10 {
        for zone in &plan.devices[0].zones {
            sampling::sample_leds(
                &canvas.pixels,
                canvas.width,
                canvas.height,
                &zone.leds,
                mode,
                &mut out,
            );
        }
    }

    let start = Instant::now();
    for _ in 0..FRAMES {
        for zone in &plan.devices[0].zones {
            sampling::sample_leds(
                &canvas.pixels,
                canvas.width,
                canvas.height,
                &zone.leds,
                mode,
                &mut out,
            );
        }
    }
    let elapsed = start.elapsed();
    let per_frame = elapsed.as_nanos() as f64 / FRAMES as f64;
    println!(
        "{mode:?}: {FRAMES} frames of {LED_COUNT} LEDs at {}x{} in {:.3} ms -> {per_frame:.0} ns/frame",
        canvas.width,
        canvas.height,
        elapsed.as_secs_f64() * 1000.0
    );
}

#[test]
#[ignore = "a micro-benchmark, not a correctness test"]
fn bench_sampling() {
    println!();
    bench_mode(SamplingMode::Nearest);
    bench_mode(SamplingMode::Bilinear);
    bench_mode(SamplingMode::AreaAverage);
    println!();
}
