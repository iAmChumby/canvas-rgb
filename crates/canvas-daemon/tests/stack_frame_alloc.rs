//! The stack compositor's own frame work allocates nothing (#407).
//!
//! The compositor draws a stack in the render thread, at 30 frames a second, and a heap
//! allocation there is a stall every layer of every stack pays. This binary counts the
//! allocations of one thread, so the count is the frame's own and no other test's.

use canvas_daemon::stack::{AreaMap, AreaMaps, LeafBuilder, LiveMap, StackEffect};
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::props::{InputNeeds, PropertySchema, Props};
use canvas_proto::stack::{BlendMode, LayerArea, Stack, StackLayer};
use canvas_proto::traits::{Canvas, Effect, Inputs};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

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
/// The gradient matters: a column read and a cell read of it differ, so the compositor
/// paints the strip LEDs (#407) and that work is in the measured frame.
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
    // paints every strip LED the blend of the two reads: the measured frame does that work.
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
