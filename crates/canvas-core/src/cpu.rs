//! Checks that the parked and paused loops cost (almost) no CPU.
//!
//! The spec budgets "Paused: daemon CPU mean ≤ 0.05%". The engine is one thread of
//! the daemon, so it has to be cheaper than that. The test times the render thread
//! on its own, so it is not disturbed by the other tests running in parallel.

#![cfg(windows)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use canvas_proto::api::Power;
use canvas_proto::traits::{Engine, EngineSettings};

use crate::engine::spawn_core;

/// The CPU time (user + kernel) of the thread with the given id, in 100 ns units.
fn thread_cpu_time_100ns(thread_id: u32) -> u64 {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{
        GetThreadTimes, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: the handle is opened for querying only and closed right after.
    let handle: HANDLE =
        unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, thread_id) };
    // A null handle means the thread does not exist (or cannot be opened).
    assert!(!handle.is_null(), "could not open thread {thread_id}");

    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut kernel = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut user = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    // SAFETY: the handle is valid and every out-parameter is a correctly sized
    // FILETIME.
    let ok = unsafe {
        GetThreadTimes(
            handle,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    // SAFETY: the handle was opened here and is not used afterwards.
    unsafe {
        CloseHandle(handle);
    }
    assert_ne!(ok, 0, "GetThreadTimes failed");
    let kernel = u64::from(kernel.dwHighDateTime) << 32 | u64::from(kernel.dwLowDateTime);
    let user = u64::from(user.dwHighDateTime) << 32 | u64::from(user.dwLowDateTime);
    kernel + user
}

/// The CPU time the thread used between two readings, in milliseconds.
fn cpu_ms_between(start: u64, end: u64) -> f64 {
    (end.saturating_sub(start)) as f64 / 10_000.0
}

/// Waits until the render thread has published its thread id.
fn render_thread_id(engine: &Arc<crate::engine::CoreEngine>) -> u32 {
    for _ in 0..100 {
        if let Some(id) = engine.render_thread_id() {
            return id;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the render thread never reported its id");
}

#[test]
fn a_parked_engine_costs_almost_no_cpu() {
    let engine = spawn_core(EngineSettings {
        fps: 30,
        power: Power::Running,
        brightness: 100,
    });
    let thread_id = render_thread_id(&engine);
    // Let it settle into the parked state.
    std::thread::sleep(Duration::from_millis(300));

    let cpu_start = thread_cpu_time_100ns(thread_id);
    let wall_start = Instant::now();
    std::thread::sleep(Duration::from_secs(3));
    let cpu_used = cpu_ms_between(cpu_start, thread_cpu_time_100ns(thread_id));
    let wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;

    let percent = cpu_used / wall_ms * 100.0;
    println!("parked: {cpu_used:.1} ms of CPU over {wall_ms:.0} ms -> {percent:.4}%");
    assert!(
        percent < 0.05,
        "the parked engine used {percent:.4}% CPU, over the 0.05% budget"
    );
    assert_eq!(engine.stats().frames_rendered, 0, "no frames were rendered");
    engine.shutdown();
}

/// A sink that throws the frames away, so the measurement is the engine's own cost.
struct NullSink;

impl canvas_proto::traits::FrameSink for NullSink {
    fn submit(&self, _colors: Vec<canvas_proto::Rgb8>) {}
}

#[test]
fn a_running_engine_at_thirty_fps_stays_inside_the_budget() {
    use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
    use canvas_proto::layout::{Layout, PlacedZone, RenderScale, SamplingMode, ZoneShape};
    use canvas_proto::traits::Engine;

    let engine = spawn_core(EngineSettings {
        fps: 30,
        power: Power::Running,
        brightness: 100,
    });
    let thread_id = render_thread_id(&engine);

    // 600 LEDs over six linear zones, the shape of a real desk.
    let zones: Vec<ZoneInfo> = (0..6)
        .map(|index| ZoneInfo {
            index,
            name: format!("zone{index}"),
            kind: ZoneKind::Linear,
            led_count: 100,
            min_leds: 100,
            max_leds: 100,
            resizable: false,
        })
        .collect();
    let device = DeviceInfo {
        id: DeviceId("cpu".into()),
        name: "cpu".into(),
        vendor: "cpu".into(),
        kind: DeviceKind::Other,
        adapter: "cpu".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones,
        connected: true,
    };
    let layout = Layout {
        id: "cpu".into(),
        name: "cpu".into(),
        render_scale: RenderScale::P100,
        sampling: SamplingMode::AreaAverage,
        zones: (0..6)
            .map(|index| PlacedZone {
                device_id: DeviceId("cpu".into()),
                zone_index: index,
                x: index as f32 * 53.0,
                y: 0.0,
                width: 53.0,
                height: 200.0,
                rotation_deg: 0.0,
                reversed: false,
                visible: true,
                shape: ZoneShape::Auto,
                label: None,
            })
            .collect(),
        effect: None,
    };
    engine.set_layout(layout, vec![device]);
    engine.add_sink(DeviceId("cpu".into()), Arc::new(NullSink));

    // Let the loop reach steady state.
    std::thread::sleep(Duration::from_millis(500));

    let cpu_start = thread_cpu_time_100ns(thread_id);
    let wall_start = Instant::now();
    let frames_start = engine.stats().frames_rendered;
    std::thread::sleep(Duration::from_secs(3));
    let cpu_used = cpu_ms_between(cpu_start, thread_cpu_time_100ns(thread_id));
    let wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;
    let frames = engine.stats().frames_rendered - frames_start;

    let percent = cpu_used / wall_ms * 100.0;
    println!(
        "running: {frames} frames, {cpu_used:.1} ms of CPU over {wall_ms:.0} ms -> {percent:.4}% of one core"
    );
    // The spec budgets 0.5% of a 16-thread machine, which is 8% of one core. The
    // engine is one thread of the daemon, so it must stay well inside that.
    assert!(
        percent < 8.0,
        "the running engine used {percent:.4}% of one core, over the 0.5%-of-the-machine budget"
    );
    engine.shutdown();
}
