//! Checks that the parked and paused loops cost (almost) no CPU.
//!
//! The spec budgets "Paused: daemon CPU mean ≤ 0.05%". The engine is one thread of
//! the daemon, so it has to be cheaper than that.
//!
//! Two tests read that budget, in two ways:
//!
//! - `a_parked_engine_wakes_on_its_period_and_renders_nothing` counts the wake-ups of the
//!   parked loop. That is a property of the loop, and no clock is involved: a slow machine
//!   can only delay a wake-up, never add one.
//! - `a_parked_engine_costs_almost_no_cpu` measures the CPU time of the render thread. That
//!   is a measurement, not a gate: `GetThreadTimes` counts in 15.625 ms ticks, so the 0.05 %
//!   budget (1.5 ms) is below the clock's own resolution and one tick fails it. It is
//!   `#[ignore]`d and run by hand on a quiet machine.
//!
//! The tests run on this machine only; the timer resolution the running engine asks for
//! (`timeBeginPeriod`, `timer.rs`) is a system-wide setting, so the sibling test below can
//! change how finely the parked loop's wait is timed while both run.

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

/// A parked engine waits on a fixed period and renders nothing: it must not spin.
///
/// The property is measured by counting the wake-ups of the parked loop, not by reading a
/// clock: the loop wakes every `PARK_POLL` (100 ms) by design, so the count over a window says
/// how often the loop looked at its state, and a slow machine can only make that count smaller
/// (class `cause:timing-test`, #39 and #47).
#[test]
fn a_parked_engine_wakes_on_its_period_and_renders_nothing() {
    let engine = spawn_core(EngineSettings {
        fps: 30,
        power: Power::Running,
        brightness: 100,
    });
    // Let it settle into the parked state.
    std::thread::sleep(Duration::from_millis(300));
    assert!(engine.stats().parked, "the engine is parked: nothing consumes its frames");

    let before = engine.park_wakeups();
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(3));
    let woke = engine.park_wakeups() - before;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    // One wake-up per period, plus room for the wait that was already running when the
    // window opened. A spinning loop wakes thousands of times in the same window.
    let allowed = elapsed_ms / crate::engine::PARK_POLL.as_millis() as u64 + 2;
    assert!(
        woke <= allowed,
        "the parked engine woke {woke} times in {elapsed_ms} ms; at most {allowed} wake-ups fit"
    );
    assert_eq!(engine.stats().frames_rendered, 0, "no frames were rendered");
    engine.shutdown();
}

/// The CPU time of the parked engine, on this machine.
///
/// The budget is 0.05 % of a core, and `GetThreadTimes` counts in 15.625 ms ticks: one tick
/// charged to the parked thread is already 0.52 %, so this measurement can only pass on a
/// quiet machine. It is the measurement, not the gate; the gate is the wake-up count above.
///
/// Run it by hand:
///
/// ```text
/// cargo test -p canvas-core -- --ignored a_parked_engine_costs_almost_no_cpu
/// ```
#[test]
#[ignore = "a perf measurement, not a gate: run it on a quiet machine"]
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

    // This engine runs, so it asks the system for a 1 ms timer (`timeBeginPeriod`, `timer.rs`).
    // That setting is system-wide, so it is in force for the whole machine while this test
    // runs, including for the parked engine of the test above. The parked loop's wake-ups do
    // not depend on it: it waits 100 ms either way.
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
                leds: None,
                part: None,
                locked: false,
            })
            .collect(),
        effect: None,
        case_form: canvas_proto::layout::CaseForm::None,
        landmarks: Vec::new(),
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
