//! The stack compositor's own frame work allocates nothing (#407).
//!
//! The compositor draws a stack in the render thread, at 30 frames a second, and a heap
//! allocation there is a stall every layer of every stack pays. This binary counts the
//! allocations of one thread, so the count is the frame's own and no other test's.

use canvas_daemon::stack::{AreaMap, AreaMaps, LeafBuilder, LiveMap, StackEffect};
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::props::{InputNeeds, PropertySchema, Props};
use canvas_proto::stack::{BlendMode, LayerArea, Stack, StackLayer};
use canvas_proto::traits::{Canvas, Effect, Inputs, LightLeds};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::time::Instant;

thread_local! {
    /// The allocations of this thread since it was last cleared.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// Counts the allocations of the thread that runs the test.
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A layer that paints a vertical gradient and allocates nothing.
///
/// The gradient matters: a column read and a cell read of it differ, so the compositor draws
/// the picture over the strip LEDs' columns when the stack answers the column (#407), and that
/// work is in the measured frame.
struct Gradient {
    /// True when the layer is read over the LED's column, false over its cell.
    column: bool,
}

impl Effect for Gradient {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "gradient".into(),
            name: "Gradient".into(),
            description: String::new(),
            needs: InputNeeds::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        let height = canvas.height.max(1) as f32;
        for y in 0..canvas.height {
            let level = y as f32 / height;
            for x in 0..canvas.width {
                canvas.set(x, y, [level, 1.0 - level, 0.25]);
            }
        }
    }

    fn strip_read(&self) -> canvas_proto::traits::StripRead {
        if self.column {
            canvas_proto::traits::StripRead::Column
        } else {
            canvas_proto::traits::StripRead::Cell
        }
    }
}

fn device() -> DeviceInfo {
    DeviceInfo {
        id: DeviceId("d".into()),
        name: "d".into(),
        vendor: "fake".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "z".into(),
            kind: ZoneKind::Linear,
            led_count: 8,
            min_leds: 0,
            max_leds: 240,
            resizable: true,
        }],
        connected: true,
    }
}

fn layer(id: &str) -> StackLayer {
    StackLayer {
        id: id.to_string(),
        name: format!("Layer {id}"),
        effect: if id == "l1" { "gradient".to_string() } else { "cell".to_string() },
        props: Props::new(),
        area: LayerArea::Canvas,
        blend: BlendMode::Normal,
        opacity: 100,
        visible: true,
        mask: None,
    }
}

/// The counter counts: without this the frame test below would pass on a counter that
/// counts nothing.
#[test]
fn the_counter_counts_an_allocation() {
    ALLOCATIONS.with(|count| count.set(0));
    let allocated = vec![7u8; 64];
    std::hint::black_box(&allocated);
    let allocations = ALLOCATIONS.with(|count| count.get());
    assert!(allocations >= 1, "the counter counted {allocations} allocations");
}

/// #407: a frame of the compositor allocates nothing.
///
/// The first frame builds the layer canvases and resolves the areas, and those allocations
/// are the setup's, not the frame's: the second frame is the one that runs every frame.
#[test]
fn a_frame_of_the_compositor_allocates_nothing() {
    let devices = vec![device()];
    let stack = Stack {
        id: "stack:Budget".into(),
        layers: vec![layer("l1"), layer("l2")],
    };
    // One layer answers the column and the other the cell, so the stack answers the cell and
    // the picture is drawn over the area's own cells: the measured frame does that work.
    let leaves: LeafBuilder = Arc::new(|id: &str| {
        let column = id == "gradient";
        Ok(Box::new(Gradient { column }) as Box<dyn Effect>)
    });
    let (_lights, plan) = canvas_daemon::stack::zone_group(&devices);
    let map = AreaMap::build_for_group(&plan, &_lights, canvas_proto::layout::SamplingMode::AreaAverage, 1);
    let areas = Arc::new(std::sync::RwLock::new(AreaMaps {
        version: 1,
        groups: [("g".to_string(), map)].into_iter().collect(),
    }));
    let live = Arc::new(std::sync::RwLock::new(LiveMap::from_stack(&stack, 1)));
    let mut effect = StackEffect::new(&stack, leaves, live, areas).with_group("g");
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(canvas_core::SCENE_RENDER_SCALE);
    let inputs = Inputs::default();
    // The first frame sets the canvases up and resolves the areas.
    effect.update(1.0 / 30.0, &mut canvas, &inputs);
    ALLOCATIONS.with(|count| count.set(0));
    effect.update(1.0 / 30.0, &mut canvas, &inputs);
    let allocations = ALLOCATIONS.with(|count| count.get());
    assert_eq!(allocations, 0, "the frame allocated {allocations} times");
}

/// #427: a per-LED read of a mixed stack allocates nothing. The first call grows the reused
/// buffers; every call after it (one light per frame) must be allocation-free.
#[test]
fn a_per_led_read_of_a_mixed_stack_allocates_nothing() {
    let devices = vec![device()];
    let stack = Stack {
        id: "stack:Budget".into(),
        layers: vec![layer("l1"), layer("l2")],
    };
    let leaves: LeafBuilder = Arc::new(|id: &str| {
        let column = id == "gradient";
        Ok(Box::new(Gradient { column }) as Box<dyn Effect>)
    });
    let (lights, plan) = canvas_daemon::stack::zone_group(&devices);
    let map = AreaMap::build_for_group(&plan, &lights, canvas_proto::layout::SamplingMode::AreaAverage, 1);
    let areas = Arc::new(std::sync::RwLock::new(AreaMaps {
        version: 1,
        groups: [("g".to_string(), map)].into_iter().collect(),
    }));
    let live = Arc::new(std::sync::RwLock::new(LiveMap::from_stack(&stack, 1)));
    let mut effect = StackEffect::new(&stack, leaves, live, areas).with_group("g");
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(canvas_core::SCENE_RENDER_SCALE);
    effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
    let scale = canvas_core::SCENE_RENDER_SCALE;
    let leds: Vec<canvas_proto::geometry::LedSample> = plan.lights[0]
        .leds
        .iter()
        .map(|s| canvas_proto::geometry::LedSample {
            center_x: s.center_x * scale,
            center_y: s.center_y * scale,
            bounds: s.bounds.map(|v| v * scale),
            sampled: s.sampled,
        })
        .collect();
    let light = LightLeds { id: &lights[0].id, arrangement: lights[0].arrangement, leds: &leds };
    let mut out = Vec::new();
    assert!(effect.led_colours(light, &mut out), "a mixed stack answers a strip light");
    ALLOCATIONS.with(|count| count.set(0));
    for _ in 0..100 {
        let light = LightLeds { id: &lights[0].id, arrangement: lights[0].arrangement, leds: &leds };
        effect.led_colours(light, &mut out);
    }
    let allocations = ALLOCATIONS.with(|count| count.get());
    assert_eq!(allocations, 0, "the per-LED read allocated {allocations} times");
}

/// #427: the cost of one frame of a 240-LED strip in a group playing eight mixed layers: the
/// compositor's `update` plus its per-LED read for the one strip light, against the 30 fps budget
/// of 33.3 ms. Ignored in CI (a timing test never decides a verdict off a clock); run by hand with
/// `cargo test -p canvas-daemon --test stack_frame_alloc -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a pass/fail test; run by hand"]
fn cost_of_a_240_led_strip_with_eight_layers() {
    let mut big = device();
    big.zones[0].led_count = 240;
    big.zones[0].max_leds = 240;
    let devices = vec![big];
    let mut layers = Vec::new();
    for i in 0..8u32 {
        let mut layer = layer(&format!("l{i}"));
        layer.effect = if i % 2 == 0 { "gradient".into() } else { "cell".into() };
        layers.push(layer);
    }
    let stack = Stack { id: "stack:Cost".into(), layers };
    let leaves: LeafBuilder = Arc::new(|id: &str| {
        let column = id == "gradient";
        Ok(Box::new(Gradient { column }) as Box<dyn Effect>)
    });
    let (lights, plan) = canvas_daemon::stack::zone_group(&devices);
    let map = AreaMap::build_for_group(&plan, &lights, canvas_proto::layout::SamplingMode::AreaAverage, 1);
    let areas = Arc::new(std::sync::RwLock::new(AreaMaps {
        version: 1,
        groups: [("g".to_string(), map)].into_iter().collect(),
    }));
    let live = Arc::new(std::sync::RwLock::new(LiveMap::from_stack(&stack, 1)));
    let mut effect = StackEffect::new(&stack, leaves, live, areas).with_group("g");
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(canvas_core::SCENE_RENDER_SCALE);
    let scale = canvas_core::SCENE_RENDER_SCALE;
    let leds: Vec<canvas_proto::geometry::LedSample> = plan.lights[0]
        .leds
        .iter()
        .map(|s| canvas_proto::geometry::LedSample {
            center_x: s.center_x * scale,
            center_y: s.center_y * scale,
            bounds: s.bounds.map(|v| v * scale),
            sampled: s.sampled,
        })
        .collect();
    let light = LightLeds { id: &lights[0].id, arrangement: lights[0].arrangement, leds: &leds };
    let mut out = Vec::new();
    for _ in 0..20 {
        effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        effect.led_colours(light, &mut out);
    }
    const N: u32 = 2000;
    let start = Instant::now();
    for _ in 0..N {
        effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        effect.led_colours(light, &mut out);
    }
    let per_frame = start.elapsed().as_secs_f64() * 1000.0 / f64::from(N);
    println!(
        "#427 cost: {per_frame:.3} ms/frame for 240 LEDs x 8 layers (update + per-LED read), \
         30 fps budget 33.333 ms, {:.1}% of budget",
        per_frame / 33.333 * 100.0
    );
}
