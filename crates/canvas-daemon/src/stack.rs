//! The stack compositor: one `Effect` that draws each layer of a saved stack.
//!
//! A stack is a spine type ([`canvas_proto::stack`]); this module is what runs it. It lives in
//! the daemon because building a layer needs both the effect registry (`canvas_effects::create`)
//! and the HTML builders ([`crate::user_effects`]), and because the engine's render thread is
//! where a frame is drawn.
//!
//! The three pieces:
//!
//! - [`AreaMap`] (the *area book*): every device's and zone's per-LED pixel mask, built from the
//!   same sample plan the engine samples with. It is rebuilt when the scene's plan or the
//!   sampling changes, never per frame. A device area lights the pixels its own LEDs
//!   read, and a second device drawn over those same pixels reads them too: the canvas has no
//!   z-order, so two overlapping footprints share the pixels they share.
//! - [`LiveMap`]: each layer's blend, opacity, visibility and area, read once per frame, so a
//!   slider drag or a blend change costs no rebuild.
//! - [`StackEffect`]: builds one effect per layer, calls each layer's `update` into its own
//!   canvas, and composites that canvas into the frame through the layer's area mask.

use canvas_core::geometry;
use canvas_proto::api::{StackLayerState, StackLayerStatus};
use canvas_proto::color::Rgb8;
use canvas_proto::device::DeviceId;
use canvas_proto::layout::{SamplingMode, CANVAS_HEIGHT, CANVAS_WIDTH};
use canvas_proto::props::{InputNeeds, Props, PropertyDef, PropertySchema};
use canvas_proto::scene::Arrangement;
use canvas_proto::stack::{
    BlendMode, LayerArea, LayerMask, MaskSource, Stack, STACK_ID_PREFIX, STACK_MAX_LAYERS,
};
use canvas_proto::traits::{Canvas, Effect, Inputs, LightLeds, StripRead};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// How an effect id is built for one layer: the lights' builders or the tile's. A leaf builder
/// refuses a `stack:` id outright, so a stack can never contain a stack (D95.4.8).
pub type LeafBuilder = Arc<dyn Fn(&str) -> Result<Box<dyn Effect>, String> + Send + Sync>;

/// The builder a stack's tile builds its layers with: like [`LeafBuilder`], but it says whether
/// a layer's build ran out of its load budget, so the tile path can give the tile up and try it
/// again later instead of caching a picture without that layer (#58). The lights' builder keeps
/// [`LeafBuilder`], so only the tile path carries the flag.
pub type TileLeafBuilder =
    Arc<dyn Fn(&str) -> Result<Box<dyn Effect>, crate::thumbnail::BuildError> + Send + Sync>;

/// The clock the hold is measured on. The daemon uses [`Instant::now`]; the tests move one by
/// hand, so a 250 ms hold is checked without waiting.
pub type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// The daemon's clock.
pub fn real_clock() -> Clock {
    Arc::new(Instant::now)
}

/// A read of a `std::sync::RwLock` that recovers a poisoned lock instead of panicking: the
/// render thread must never die because another thread panicked while holding the data.
pub fn read_recovering<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A write of a `std::sync::RwLock` that recovers a poisoned lock (see [`read_recovering`]).
pub fn write_recovering<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The message a panic payload carries, for a log line and for the status a faulted mask
/// reports (#256).
fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "panicked".to_string()
    }
}

// ---------------------------------------------------------------- the live map

/// A layer's mask as the compositor reads it every frame (#256): which channel is read, whether
/// it is flipped, how strongly it applies, and the values the mask effect runs with.
///
/// The values are live, not structural: a slider drag on Amount, or a mask property, re-initialises
/// the mask effect and rebuilds nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveMask {
    pub source: MaskSource,
    pub invert: bool,
    pub amount: u8,
    pub props: Props,
}

impl LiveMask {
    /// The values a file's mask asks for.
    pub fn from_mask(mask: &LayerMask) -> Self {
        Self {
            source: mask.source,
            invert: mask.invert,
            amount: mask.amount.min(100),
            props: mask.props.clone(),
        }
    }
}

/// A layer's live settings, as the compositor reads them every frame.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveLayer {
    pub blend: BlendMode,
    pub opacity: u8,
    pub visible: bool,
    pub area: LayerArea,
    /// The layer's mask, or `None` when it draws whole.
    pub mask: Option<LiveMask>,
}

impl LiveLayer {
    /// The values a file's layer asks for.
    pub fn from_layer(layer: &canvas_proto::stack::StackLayer) -> Self {
        Self {
            blend: layer.blend,
            opacity: layer.opacity.min(100),
            visible: layer.visible,
            area: layer.area.clone(),
            mask: layer.mask.as_ref().map(LiveMask::from_mask),
        }
    }
}

/// Everything that is a layer's *value* and not its structure (D95.1.8): the compositor reads it
/// once per frame, so changing any of it rebuilds nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveMap {
    /// Bumped by whoever writes a new map; the compositor re-resolves areas only when it moves.
    pub version: u64,
    pub layers: BTreeMap<String, LiveLayer>,
}

impl LiveMap {
    /// The map a stack's file asks for.
    pub fn from_stack(stack: &Stack, version: u64) -> Self {
        let layers = stack
            .layers
            .iter()
            .map(|layer| (layer.id.clone(), LiveLayer::from_layer(layer)))
            .collect();
        Self { version, layers }
    }
}

/// The live map of the stack on the engine.
pub type LiveStack = Arc<RwLock<LiveMap>>;

/// Rebuilds a running effect's live map from `stack` and swaps it in, bumping the version so
/// the effect re-resolves its layers on its next frame.
///
/// The new map is built outside the lock and the old one is swapped in under it, as a save does
/// (`Stacks::update_live`), so the render thread never waits for a map to be built.
pub fn replace_live(live: &LiveStack, stack: &Stack) {
    let mut next = LiveMap::from_stack(stack, 0);
    let mut map = write_recovering(live);
    next.version = map.version.wrapping_add(1);
    *map = next;
}

/// The area book every `StackEffect` resolves its layers against: the masks of every group of
/// the active scene, by group id.
pub type AreaBook = Arc<RwLock<AreaMaps>>;

// ---------------------------------------------------------------- the area book

/// Grows a bounding box so it covers one pixel.
fn grow(extent: &mut Option<[f32; 4]>, fx: f32, fy: f32) {
    *extent = Some(match *extent {
        Some([x0, y0, x1, y1]) => [x0.min(fx), y0.min(fy), x1.max(fx + 1.0), y1.max(fy + 1.0)],
        None => [fx, fy, fx + 1.0, fy + 1.0],
    });
}

/// One area's pixels, in render pixels: where the picture is mapped, and which pixels it may
/// paint. A bit is set where the area covers the pixel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mask {
    /// The bounding box of the area's rectangles, in render pixels: the layer's whole picture is
    /// mapped over this box.
    pub dest: [f32; 4],
    /// The same box read as a column (#407): the strip LEDs' own columns over the full height
    /// of the canvas. A stack that answers the column maps its picture over this box, so the
    /// strip reads the whole column and not only the cell at mid-height.
    pub column_dest: [f32; 4],
    /// The bounding box of the pixels the mask covers, in render pixels. It can reach past
    /// `dest`: a ring's cell at the edge of a fan reads pixels just outside the fan's rectangle,
    /// and those must be painted or the LED would average black into it (D95.2.3).
    extent: Option<[f32; 4]>,
    /// The same box read as a column (#407): the strip LEDs' own columns over the full height
    /// of the canvas, and nothing else. A light that is not a strip has no column.
    column_extent: Option<[f32; 4]>,
    words: Vec<u64>,
    /// The area's cells, read as a column (#407): the strip LEDs' columns only. A fan's or a
    /// keyboard's cell is not in it: those lights read their cell and never the column.
    column_words: Vec<u64>,
    width: u32,
    height: u32,
}

impl Mask {
    /// An empty mask over a canvas of `width` by `height` render pixels.
    pub fn new(width: u32, height: u32) -> Self {
        let bits = (width as usize) * (height as usize);
        Self {
            dest: [0.0, 0.0, 0.0, 0.0],
            column_dest: [0.0, 0.0, 0.0, 0.0],
            extent: None,
            column_extent: None,
            words: vec![0; bits.div_ceil(64)],
            column_words: vec![0; bits.div_ceil(64)],
            width,
            height,
        }
    }

    /// The bounding box of the pixels the mask covers, or `None` when it covers none.
    pub fn extent(&self) -> Option<[f32; 4]> {
        self.extent
    }

    /// The bounding box of the pixels the mask covers when it is read as a column (#407), or
    /// `None` when it covers none.
    pub fn column_extent(&self) -> Option<[f32; 4]> {
        self.column_extent
    }

    /// True when the mask covers no pixel at all.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    /// True when the mask covers the pixel at `x`, `y`.
    #[inline]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        self.covers(&self.words, x, y)
    }

    /// True when the column read covers the pixel at `x`, `y` (#407).
    #[inline]
    pub fn contains_column(&self, x: u32, y: u32) -> bool {
        self.covers(&self.column_words, x, y)
    }

    /// True when the bits cover the pixel at `x`, `y`.
    #[inline]
    fn covers(&self, words: &[u64], x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let bit = (y as usize) * (self.width as usize) + (x as usize);
        words[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    /// Sets the bit for one pixel; out-of-canvas pixels are ignored.
    ///
    /// The cell read and the column read are two different pictures of the area (#407): the
    /// cell read is every LED's own cell, the column read is the strip LEDs' columns over the
    /// full height. A cell pixel is not a column pixel: a fan or a keyboard reads its cell and
    /// never the column, so the two bitmaps stay apart.
    #[inline]
    fn set(&mut self, x: i64, y: i64) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let bit = (y as usize) * (self.width as usize) + (x as usize);
        self.words[bit / 64] |= 1u64 << (bit % 64);
        let (fx, fy) = (x as f32, y as f32);
        grow(&mut self.extent, fx, fy);
    }

    /// Sets the bit for one pixel of the column read only (#407): the strip LED's own x-range
    /// over the full height of the canvas.
    #[inline]
    fn set_column(&mut self, x: i64, y: i64) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let bit = (y as usize) * (self.width as usize) + (x as usize);
        self.column_words[bit / 64] |= 1u64 << (bit % 64);
        let (fx, fy) = (x as f32, y as f32);
        grow(&mut self.column_extent, fx, fy);
    }

    /// Adds the pixels one LED's sampling mode reads (D95.2.3).
    ///
    /// `column` says the LED is a strip's: the column read (#407) takes its own x-range over
    /// the full height of the canvas, so the area covers that column as well.
    fn add_led(&mut self, led: &geometry::LedSample, mode: SamplingMode, column: bool) {
        if !led.sampled || self.width == 0 || self.height == 0 {
            return;
        }
        let max_x = i64::from(self.width) - 1;
        let max_y = i64::from(self.height) - 1;
        let nearest = || {
            (
                (led.center_x.round() as i64).clamp(0, max_x),
                (led.center_y.round() as i64).clamp(0, max_y),
            )
        };
        match mode {
            SamplingMode::Nearest => {
                let (x, y) = nearest();
                self.set(x, y);
            }
            SamplingMode::Bilinear => {
                let fx = led.center_x.clamp(0.0, max_x as f32);
                let fy = led.center_y.clamp(0.0, max_y as f32);
                let x0 = fx.floor() as i64;
                let y0 = fy.floor() as i64;
                let x1 = (x0 + 1).min(max_x);
                let y1 = (y0 + 1).min(max_y);
                for y in [y0, y1] {
                    for x in [x0, x1] {
                        self.set(x, y);
                    }
                }
            }
            SamplingMode::AreaAverage => {
                let [bx0, by0, bx1, by1] = led.bounds;
                // The pixels whose centres fall in the region, the rule sampling::sample_area
                // averages over.
                let x0 = ((bx0 - 0.5).ceil() as i64).clamp(0, max_x);
                let y0 = ((by0 - 0.5).ceil() as i64).clamp(0, max_y);
                let x1 = ((bx1 - 0.5).floor() as i64).clamp(0, max_x).max(x0);
                let y1 = ((by1 - 0.5).floor() as i64).clamp(0, max_y).max(y0);
                // `sampling::sample_area` raises an empty axis to its first pixel, one axis at a
                // time, so a cell thinner than a pixel in one direction (a strip LED, #372) still
                // reads the pixel it is over in the other. The mask is those pixels, not the
                // nearest one on both axes.
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        self.set(x, y);
                    }
                }
            }
        }
        if column {
            self.add_column(led);
        }
    }

    /// Adds the pixels the column read of one LED covers (#407): its own x-range over the full
    /// height of the canvas, the range `sampling::sample_led_columns` averages over.
    ///
    /// A layer in an area the strip reads as a column is painted over that column, so the strip
    /// reads the whole picture and not only the cell at mid-height.
    fn add_column(&mut self, led: &geometry::LedSample) {
        if !led.sampled || self.width == 0 || self.height == 0 {
            return;
        }
        let max_x = i64::from(self.width) - 1;
        let x0 = ((led.bounds[0] - 0.5).ceil() as i64).clamp(0, max_x);
        let x1 = ((led.bounds[2] - 0.5).floor() as i64).clamp(0, max_x).max(x0);
        for x in x0..=x1 {
            for y in 0..i64::from(self.height) {
                self.set_column(x, y);
            }
        }
    }
}

/// Which pixels of an area a layer paints, read with the area's own reading (#407).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Coverage {
    /// The cells the area's LEDs read.
    Cell,
    /// The columns the area's strip LEDs read: their own x-range over the full height.
    Column,
    /// The cells the column read does not cover: the lights that are not a strip, a fan or a
    /// keyboard. They read their cell and never the column, so they are painted over the area's
    /// own box and read exactly as they read before.
    CellOnly,
}

impl Coverage {
    /// True when the area's mask covers the pixel, read this way.
    fn covers(self, mask: &Mask, x: u32, y: u32) -> bool {
        match self {
            Coverage::Cell => mask.contains(x, y),
            Coverage::Column => mask.contains_column(x, y),
            Coverage::CellOnly => mask.contains(x, y) && !mask.contains_column(x, y),
        }
    }
}

/// Where one layer's picture is mapped, and which pixels it may paint (#407).
#[derive(Debug, Clone, Copy)]
struct PaintBox {
    /// The box the picture is mapped over.
    dest: [f32; 4],
    /// The box it may paint: the pixels the area's mask covers.
    paint: [f32; 4],
    /// Which of the area's pixels it may paint, read with the layer's own reading.
    coverage: Coverage,
}

/// Where a layer paints, by its own strip read. The picture path (`composite_layer`) and the
/// per-LED path (`collect_led_colours`) both go through this, so a layer maps its picture over the
/// same area box on the strip as on the stage (#427 review). The whole-canvas case uses the
/// target's own size, exactly as the picture path always did.
fn paint_box(area: &OwnedArea, column: bool, width: f32, height: f32) -> PaintBox {
    let dest = if area.whole_canvas {
        [0.0, 0.0, width, height]
    } else if column {
        area.column_dest
    } else {
        area.dest
    };
    let paint = match &area.mask {
        Some(mask) => {
            if column { mask.column_extent().unwrap_or(dest) } else { mask.extent().unwrap_or(dest) }
        }
        None => dest,
    };
    let coverage = if column { Coverage::Column } else { Coverage::Cell };
    PaintBox { dest, paint, coverage }
}

/// One layer's paint, resolved the way the compositor resolves it: the picture it draws, the
/// pixels the area lets it paint, and the alpha the area's mask and the layer's own mask effect
/// allow. Both the picture's `paint_area` and the per-LED scratch fill call `pixel_at`, so they
/// cannot disagree about what a layer paints at any pixel (#427 review).
struct LayerPaint<'a> {
    box_: PaintBox,
    area_mask: Option<&'a Mask>,
    src: &'a Canvas,
    layer_mask: Option<&'a MaskLayer>,
    alpha: f32,
}

impl<'a> LayerPaint<'a> {
    /// The integer pixel box to walk, clamped to `target`, or `None` when this paints nothing.
    fn bounds(&self, target: &Canvas) -> Option<(u32, u32, u32, u32)> {
        let dest = self.box_.dest;
        let (dw, dh) = (dest[2] - dest[0], dest[3] - dest[1]);
        if !dest.iter().all(|v| v.is_finite()) || dw <= 0.0 || dh <= 0.0 {
            return None;
        }
        if self.src.width == 0 || self.src.height == 0 {
            return None;
        }
        let paint = self.box_.paint;
        let x0 = (paint[0].floor().max(0.0) as u32).min(target.width);
        let y0 = (paint[1].floor().max(0.0) as u32).min(target.height);
        let x1 = (paint[2].ceil().max(0.0) as u32).min(target.width);
        let y1 = (paint[3].ceil().max(0.0) as u32).min(target.height);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some((x0, y0, x1, y1))
    }

    /// The source pixel this layer contributes at `(x, y)` and how much of it shows, or `None`
    /// where the area's mask keeps the layer out or the mask effect fully covers it.
    fn pixel_at(&self, x: u32, y: u32) -> Option<([f32; 3], f32)> {
        if let Some(mask) = self.area_mask {
            if !self.box_.coverage.covers(mask, x, y) {
                return None;
            }
        }
        let [dx0, dy0, dx1, dy1] = self.box_.dest;
        let (dw, dh) = (dx1 - dx0, dy1 - dy0);
        // The layer's whole picture is mapped over the area's bounding box.
        let sx = (((x as f32 + 0.5 - dx0) / dw) * self.src.width as f32).floor();
        let sy = (((y as f32 + 0.5 - dy0) / dh) * self.src.height as f32).floor();
        let sx = (sx.max(0.0) as u32).min(self.src.width - 1);
        let sy = (sy.max(0.0) as u32).min(self.src.height - 1);
        let source = self.src.get(sx, sy);
        let alpha = match self.layer_mask {
            Some(mask) => {
                let pixel = mask.canvas.get(sx, sy);
                self.alpha
                    * masked_coverage(mask.values.source, pixel, mask.values.invert, mask.values.amount)
            }
            None => self.alpha,
        };
        if alpha <= 0.0 {
            return None;
        }
        Some((source, alpha))
    }
}

/// Builds one layer's paint from the compositor's own state. A mask that does not apply, or whose
/// canvas no longer matches the picture, shapes nothing: the layer draws whole, as it does for the
/// picture in `paint_area`.
fn layer_paint<'a>(
    layers: &'a [Layer],
    index: usize,
    area: &'a OwnedArea,
    box_: PaintBox,
    alpha: f32,
) -> LayerPaint<'a> {
    let layer = &layers[index];
    let layer_mask = layer.mask.as_ref().filter(|mask| mask.applies()).filter(|mask| {
        mask.canvas.width == layer.canvas.width && mask.canvas.height == layer.canvas.height
    });
    LayerPaint { box_, area_mask: area.mask.as_ref(), src: &layer.canvas, layer_mask, alpha }
}

/// Every device's and zone's mask, at one group, render scale and sampling mode.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AreaMap {
    /// Bumped whenever a new map is published; the compositor rebuilds its resolved masks only
    /// when it moves.
    pub version: u64,
    pub devices: BTreeMap<DeviceId, Mask>,
    pub zones: BTreeMap<(DeviceId, u32), Mask>,
    /// The group's render scale, as a factor.
    pub scale: f32,
    pub sampling: SamplingMode,
    /// The render size the masks are built for.
    pub width: u32,
    pub height: u32,
}

impl AreaMap {
    /// Builds the masks of one group: its members' LEDs, arranged on the group's canvas.
    ///
    /// The plan is the arrangement (Task 3): where each member's LEDs read the group's canvas.
    /// The samples are in the group's canvas units, so they are scaled to the render size the
    /// masks are built at, the same scale the engine renders the group at.
    ///
    /// A device's mask is the union of the samples of every light of that device in the group;
    /// a zone's is the union of the samples of every light on that zone. A device or a zone the
    /// group does not hold gets no mask: an area over it paints nothing.
    pub fn build_for_group(
        plan: &canvas_core::arrangement::GroupPlan,
        lights: &[canvas_proto::scene::Light],
        sampling: SamplingMode,
        version: u64,
    ) -> Self {
        let scale = canvas_core::SCENE_RENDER_SCALE;
        let width = ((CANVAS_WIDTH as f32 * scale).round() as u32).max(1);
        let height = ((CANVAS_HEIGHT as f32 * scale).round() as u32).max(1);
        let mut map = Self {
            version,
            devices: BTreeMap::new(),
            zones: BTreeMap::new(),
            scale,
            sampling,
            width,
            height,
        };
        for samples in plan.lights.iter() {
            let Some(light) = lights.iter().find(|l| l.id == samples.light) else { continue };
            // A strip LED reads the column when the group's effect asks for it (#407); the mask
            // is built for both reads, and the compositor picks the one the stack answers.
            let column = light.arrangement == Arrangement::Strip;
            let device_mask = map.devices.entry(light.device_id.clone()).or_insert_with(|| Mask::new(width, height));
            let zone_mask = map
                .zones
                .entry((light.device_id.clone(), light.zone_index))
                .or_insert_with(|| Mask::new(width, height));
            for led in &samples.leds {
                let scaled = scale_sample(led, scale);
                device_mask.add_led(&scaled, sampling, column);
                zone_mask.add_led(&scaled, sampling, column);
            }
        }
        // A device or zone with no LED of its own paints nothing, and has no box.
        map.devices.retain(|_, mask| !mask.is_empty());
        map.zones.retain(|_, mask| !mask.is_empty());
        for mask in map.devices.values_mut().chain(map.zones.values_mut()) {
            mask.dest = mask.extent().unwrap_or([0.0, 0.0, 0.0, 0.0]);
            mask.column_dest = mask.column_extent().unwrap_or([0.0, 0.0, 0.0, 0.0]);
        }
        map
    }

    /// The mask for the area a layer names, or `None` when it covers nothing.
    pub fn resolve(&self, area: &LayerArea) -> Option<ResolvedArea<'_>> {
        // A map nobody published yet (a library tile has no layout) still resolves the whole
        // canvas and a rectangle, at the logical size and 100 %; a device or a zone has no
        // mask then, so it covers nothing.
        let (paint_w, paint_h, paint_scale) = if self.width == 0 || self.height == 0 {
            (CANVAS_WIDTH, CANVAS_HEIGHT, 1.0)
        } else {
            (self.width, self.height, self.scale)
        };
        match area {
            LayerArea::Canvas => Some(ResolvedArea {
                dest: [0.0, 0.0, paint_w as f32, paint_h as f32],
                column_dest: [0.0, 0.0, paint_w as f32, paint_h as f32],
                mask: None,
                whole_canvas: true,
            }),
            LayerArea::Rect { x, y, width, height } => {
                let dest = rect_px(*x, *y, *width, *height, paint_scale, paint_w, paint_h)?;
                Some(ResolvedArea { dest, column_dest: dest, mask: None, whole_canvas: false })
            }
            LayerArea::Device { device } => {
                let mask = self.devices.get(device)?;
                Some(ResolvedArea {
                    dest: mask.dest,
                    column_dest: mask.column_dest,
                    mask: Some(mask),
                    whole_canvas: false,
                })
            }
            LayerArea::Zone { device, zone } => {
                let mask = self.zones.get(&(device.clone(), *zone))?;
                Some(ResolvedArea {
                    dest: mask.dest,
                    column_dest: mask.column_dest,
                    mask: Some(mask),
                    whole_canvas: false,
                })
            }
        }
    }

    /// True when the area covers nothing at all, for the status the API reports.
    pub fn is_empty_area(&self, area: &LayerArea) -> bool {
        self.resolve(area).is_none()
    }
}

/// A sample in the group's canvas units, as a sample in render pixels.
fn scale_sample(sample: &geometry::LedSample, scale: f32) -> geometry::LedSample {
    geometry::LedSample {
        center_x: sample.center_x * scale,
        center_y: sample.center_y * scale,
        bounds: sample.bounds.map(|v| v * scale),
        sampled: sample.sampled,
    }
}

/// The masks of every group, at one version.
///
/// A stack resolves its layers in the group it plays in, so the book holds one map per group.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AreaMaps {
    /// Bumped whenever a new map is published; the compositor rebuilds its resolved masks only
    /// when it moves.
    pub version: u64,
    /// The masks of each group of the active scene, by group id.
    pub groups: BTreeMap<String, AreaMap>,
}

impl AreaMaps {
    /// The mask for the area a layer names, or `None` when it covers nothing.
    ///
    /// `group` is the group the stack plays in: its areas resolve in that group's canvas. A group
    /// the book does not hold resolves to nothing ("not in this group"). With no group (a library
    /// tile) a whole-canvas area and a rectangle still resolve, at the logical size and 100 %; a
    /// device or a zone has no mask then, so it covers nothing.
    pub fn resolve(&self, group: Option<&str>, area: &LayerArea) -> Option<ResolvedArea<'_>> {
        match group {
            Some(id) => self.groups.get(id)?.resolve(area),
            // A tile renders a stack for the library: it has no group and so no masks.
            None => match area {
                LayerArea::Canvas => Some(ResolvedArea {
                    dest: [0.0, 0.0, CANVAS_WIDTH as f32, CANVAS_HEIGHT as f32],
                    column_dest: [0.0, 0.0, CANVAS_WIDTH as f32, CANVAS_HEIGHT as f32],
                    mask: None,
                    whole_canvas: true,
                }),
                LayerArea::Rect { x, y, width, height } => {
                    let dest = rect_px(*x, *y, *width, *height, 1.0, CANVAS_WIDTH, CANVAS_HEIGHT)?;
                    Some(ResolvedArea { dest, column_dest: dest, mask: None, whole_canvas: false })
                }
                LayerArea::Device { .. } | LayerArea::Zone { .. } => None,
            },
        }
    }

    /// True when the area covers nothing at all, for the status the API reports.
    pub fn is_empty_area(&self, group: Option<&str>, area: &LayerArea) -> bool {
        self.resolve(group, area).is_none()
    }
}

/// Where one layer's picture is mapped, and the pixels it may paint.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedArea<'a> {
    /// The bounding box of the area's rectangles, in render pixels.
    pub dest: [f32; 4],
    /// The same box read as a column (#407): the strip LEDs' own x-range over the full height of
    /// the canvas. The compositor maps the picture over this box when the stack answers the
    /// column.
    pub column_dest: [f32; 4],
    /// The per-LED mask, or `None` for a whole-canvas or rectangular area (whose bounding box is
    /// the whole area).
    pub mask: Option<&'a Mask>,
    /// True for a whole-canvas area: its picture is mapped over the frame's own canvas, whatever
    /// size the area book was built at (a library tile has no layout at all).
    pub whole_canvas: bool,
}

/// The resolved area the compositor keeps between frames: the mask copied out of the area book,
/// so the book's lock is not held while a layer's update runs.
#[derive(Debug, Clone, PartialEq)]
struct OwnedArea {
    dest: [f32; 4],
    column_dest: [f32; 4],
    mask: Option<Mask>,
    whole_canvas: bool,
}

impl OwnedArea {
    fn from_resolved(area: ResolvedArea<'_>) -> Self {
        Self {
            dest: area.dest,
            column_dest: area.column_dest,
            mask: area.mask.cloned(),
            whole_canvas: area.whole_canvas,
        }
    }
}

/// One layer's resolved area and the live values they were resolved from. The frame reads both
/// from here, so it never copies the whole live map every frame (D95.4.6).
#[derive(Debug, Clone, PartialEq)]
struct ResolvedLayer {
    values: LiveLayer,
    /// `None` is "paints nothing".
    area: Option<OwnedArea>,
}


/// A rectangle in logical canvas units as render pixels, clamped to the canvas, or `None` when
/// it covers nothing. A NaN or an infinity is "no area" (D95.3.6).
fn rect_px(x: f32, y: f32, w: f32, h: f32, scale: f32, width: u32, height: u32) -> Option<[f32; 4]> {
    if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() {
        return None;
    }
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let x0 = (x * scale).clamp(0.0, width as f32);
    let y0 = (y * scale).clamp(0.0, height as f32);
    let x1 = ((x + w) * scale).clamp(0.0, width as f32);
    let y1 = ((y + h) * scale).clamp(0.0, height as f32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some([x0, y0, x1, y1])
}

// ---------------------------------------------------------------- the blend maths

/// One pixel of one layer, composited over what is under it (D95.3.3).
///
/// Each mode is `d + a·(blend(d, s) âˆ’ d)`, with Add's blend clamped before the opacity is
/// folded in: at `d = 0.5, s = 0.8, a = 0.5` the result is 0.75, not 0.90.
pub fn blend_pixel(mode: BlendMode, d: [f32; 3], s: [f32; 3], a: f32) -> [f32; 3] {
    // A NaN has no value to clamp, so it is 0; an infinity clamps like any other number.
    let unit = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let a = unit(a);
    let mut out = [0.0f32; 3];
    for channel in 0..3 {
        let dst = unit(d[channel]);
        let src = unit(s[channel]);
        let blended = match mode {
            BlendMode::Normal => src,
            BlendMode::Add => (dst + src).min(1.0),
            BlendMode::Multiply => dst * src,
            BlendMode::Screen => 1.0 - (1.0 - dst) * (1.0 - src),
        };
        out[channel] = (dst + a * (blended - dst)).clamp(0.0, 1.0);
    }
    out
}

// ---------------------------------------------------------------- the schema

/// The schema of a stack, derived from its layers rather than stored (D95.4.3).
///
/// `resolve` answers a layer's effect schema; a layer whose effect resolves to nothing, or that
/// names another stack, contributes no property (it is skipped at build and reported instead).
pub fn schema_for(stack: &Stack, resolve: &dyn Fn(&str) -> Option<PropertySchema>) -> PropertySchema {
    let mut properties: Vec<PropertyDef> = Vec::new();
    let mut needs = InputNeeds::default();
    let mut names: Vec<String> = Vec::new();
    for layer in &stack.layers {
        let schema = if layer.effect.starts_with(STACK_ID_PREFIX) {
            None
        } else {
            resolve(&layer.effect)
        };
        // A mask's effect reads its own inputs too (#256): a screen or audio effect used as a
        // mask needs the capture the same way a layer does.
        if let Some(mask) = &layer.mask {
            if let Some(mask_schema) = resolve(&mask.effect) {
                needs.screen |= mask_schema.needs.screen;
                needs.audio |= mask_schema.needs.audio;
            }
        }
        match schema {
            Some(schema) => {
                needs.screen |= schema.needs.screen;
                needs.audio |= schema.needs.audio;
                names.push(schema.name.clone());
                for def in &schema.properties {
                    properties.push(PropertyDef {
                        key: format!("{}.{}", layer.id, def.key),
                        label: format!("{} · {}", layer.name, def.label),
                        kind: def.kind.clone(),
                    });
                }
            }
            None => names.push(layer.effect.clone()),
        }
    }
    let description = describe(&names);
    PropertySchema {
        id: format!("{STACK_ID_PREFIX}{}", stack.name()),
        name: stack.name().to_string(),
        description,
        needs,
        properties,
        // A stack's presets are `effects.json` presets under its id (D95.4.3).
        presets: Vec::new(),
        thumbnail: None,
    }
}

/// The library's one-line description of a stack: "3 layers: Gradient, Fire, Rain".
fn describe(names: &[String]) -> String {
    let count = names.len();
    let noun = if count == 1 { "layer" } else { "layers" };
    if names.is_empty() {
        return "No layers yet".to_string();
    }
    format!("{count} {noun}: {}", names.join(", "))
}

/// Every layer's props, namespaced `{layer.id}.{key}` and filled from that layer's defaults
/// (D95.4.3): what activating a stack takes before the request's overrides.
///
/// A layer whose effect is gone keeps its own values, so a file that loses an effect and gets it
/// back does not lose the values with it.
pub fn stack_props(stack: &Stack, resolve: &dyn Fn(&str) -> Option<PropertySchema>) -> Props {
    let mut props = Props::new();
    for layer in &stack.layers {
        if let Some(schema) = resolve(&layer.effect) {
            for (key, value) in schema.defaults() {
                props.insert(format!("{}.{key}", layer.id), value);
            }
        }
        for (key, value) in &layer.props {
            props.insert(format!("{}.{key}", layer.id), value.clone());
        }
    }
    props
}

/// A layer's props out of the merged map: the keys under its own id, un-namespaced, plus the
/// reserved tile seed, which every layer gets (D95.4.10).
fn layer_props(merged: &Props, id: &str, seed: Option<&canvas_proto::PropValue>) -> Props {
    let prefix = format!("{id}.");
    let mut props = Props::new();
    for (key, value) in merged {
        if let Some(rest) = key.strip_prefix(&prefix) {
            props.insert(rest.to_string(), value.clone());
        }
    }
    if let Some(seed) = seed {
        props.insert(canvas_effects::TILE_SEED_PROP.to_string(), seed.clone());
    }
    props
}

/// The reserved tile seed in a props map, if the tile path put one there.
fn tile_seed(props: &Props) -> Option<canvas_proto::PropValue> {
    props.get(canvas_effects::TILE_SEED_PROP).cloned()
}

// ---------------------------------------------------------------- structure identity

/// A stack's structure: the layer ids, their order, their effect ids and each HTML layer's
/// source, and each mask's effect and HTML source (#256), and nothing else (D95.4.6). Props,
/// blend, opacity, visibility, area and the mask's own values are values.
pub fn structure(
    stack: &Stack,
    source_of: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, String, String, String)> {
    let hash_of = |effect: &str| {
        if effect.starts_with(crate::user_effects::USER_EFFECT_PREFIX) {
            source_of(effect)
                .map(|source| crate::user_effects::source_hash(&source))
                .unwrap_or_default()
        } else {
            String::new()
        }
    };
    stack
        .layers
        .iter()
        .map(|layer| {
            let mask = layer
                .mask
                .as_ref()
                .map(|mask| format!("{}|{}", mask.effect, hash_of(&mask.effect)))
                .unwrap_or_default();
            (layer.id.clone(), layer.effect.clone(), hash_of(&layer.effect), mask)
        })
        .collect()
}

/// The hash of a structure. FNV-1a over the canonical JSON, through the hash this repository
/// already uses for an effect's source: it is internal, never written to a file and never shown.
pub fn structure_hash(parts: &[(String, String, String, String)]) -> String {
    let text = serde_json::to_string(parts).unwrap_or_default();
    crate::user_effects::source_hash(&text)
}

// ---------------------------------------------------------------- hold state

/// The longest one HTML layer's `update` may take, at `fps`.
///
/// `min(8 ms, 40 % of the frame)`: the cap binds at 50 fps and below, and the share above it
/// (D95.3.5).
pub fn html_budget(fps: u32) -> Duration {
    let frame_ms = 1000.0 / f64::from(fps.clamp(1, 1000));
    Duration::from_secs_f64((frame_ms * 0.4).min(8.0) / 1000.0)
}

/// How long each consecutive hold lasts: 250 ms, 500 ms, 1 s, then the layer is faulted.
const HOLD_STEPS_MS: [u64; 3] = [250, 500, 1000];

/// The hold an HTML effect's measured update leaves it in: a trial frame that fits resets the
/// escalation; one that overruns takes the next step, and a fourth overrun faults it. The second
/// value is the measured time to report with a hold, in milliseconds.
fn overrun_hold(before: Hold, elapsed: Duration, budget: Duration, now: Instant) -> (Hold, Option<f32>) {
    if elapsed <= budget {
        return (Hold::Running, None);
    }
    let level = match before {
        Hold::Held { level, .. } => level.saturating_add(1),
        _ => 1,
    };
    let ms = Some(elapsed.as_secs_f32() * 1000.0);
    if level as usize > HOLD_STEPS_MS.len() {
        (Hold::Faulted, ms)
    } else {
        let hold = Duration::from_millis(HOLD_STEPS_MS[level as usize - 1]);
        (Hold::Held { until: now + hold, level }, ms)
    }
}

/// What a layer's slow updates have done to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// Updating every frame.
    Running,
    /// Not updating until this instant, then a trial frame. `level` counts the holds so far.
    Held { until: Instant, level: u8 },
    /// Three holds whose trials all overran: frozen on its last picture, never updated again.
    Faulted,
}

/// One built layer.
struct Layer {
    id: String,
    name: String,
    /// The effect id, as the file names it.
    effect: String,
    /// `None` when the effect did not build or names a stack: the layer is skipped.
    built: Option<Box<dyn Effect>>,
    /// The layer's own picture, at the frame's render size.
    canvas: Canvas,
    /// What the file says the layer's values are, for a live map that does not know the layer.
    values: LiveLayer,
    hold: Hold,
    /// The measured time of the last update that overran, in milliseconds.
    last_ms: Option<f32>,
    /// True when a panic was already logged for this layer: a faulted layer logs once.
    panic_logged: bool,
    /// True when the layer's effect panicked while it was asked how a strip reads it (#407).
    /// Such a layer is faulted for good: the engine does not call a failed effect again until
    /// it is replaced, and the stack is replaced when it is built again.
    strip_read_faulted: bool,
    /// The layer's mask, built and run beside it (#256), or `None` when it draws whole.
    mask: Option<MaskLayer>,
    /// Why the layer's mask is not applied: its effect did not build. The layer draws unmasked.
    mask_error: Option<String>,
}

/// One built mask: another effect, run every frame over the layer's own area, whose picture
/// decides how much of the layer shows (#256).
struct MaskLayer {
    /// The effect id, as the file names it.
    effect: String,
    built: Box<dyn Effect>,
    /// The mask's picture, at the frame's render size (the layer canvas's size).
    canvas: Canvas,
    /// The values the file asks for, for a live map that does not know the layer.
    values: LiveMask,
    /// The values the effect was last initialised with, so a live change re-initialises it once.
    applied: LiveMask,
    hold: Hold,
    last_ms: Option<f32>,
    panic_logged: bool,
    /// Why the mask is not applied any more: its effect panicked. The layer draws unmasked.
    fault: Option<String>,
}

impl MaskLayer {
    /// True while the mask shapes the layer: it built, it has not faulted (a panic or an overrun
    /// that faulted it), and its values apply. A mask that is merely held keeps its last picture
    /// and still shapes the layer; a faulted one leaves the layer unmasked (#256).
    fn applies(&self) -> bool {
        self.fault.is_none() && !matches!(self.hold, Hold::Faulted)
    }
}

/// The coverage one mask pixel gives, 0 (hidden) to 1 (whole), before invert and amount.
///
/// Luma is Rec. 709's weights on the picture's **encoded sRGB** values
/// (`0.2126 R + 0.7152 G + 0.0722 B`): the video-luma convention. Nothing here linearises — the
/// canvas holds encoded values, exactly as `Rgb8::to_f32` divides its channels by 255 with no
/// transfer function — so this is a perceptual-enough brightness, not linear luminance. The
/// compositor's canvases are RGB with no alpha channel, so Alpha reads the picture's strongest
/// channel: a pixel the mask effect left black is transparent, and any colour it drew is opaque at
/// its strongest channel's strength.
pub fn mask_coverage(source: MaskSource, pixel: [f32; 3]) -> f32 {
    let channel = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let [r, g, b] = pixel.map(channel);
    match source {
        MaskSource::Luma => 0.2126 * r + 0.7152 * g + 0.0722 * b,
        MaskSource::Alpha => r.max(g).max(b),
    }
}

/// The coverage a mask's pixel applies to the layer, after its invert and amount: `amount` blends
/// between 1 (the layer draws whole) and the pixel's own coverage.
pub fn masked_coverage(source: MaskSource, pixel: [f32; 3], invert: bool, amount: u8) -> f32 {
    let raw = mask_coverage(source, pixel);
    let raw = if invert { 1.0 - raw } else { raw };
    let amount = f32::from(amount.min(100)) / 100.0;
    1.0 - (1.0 - raw) * amount
}

/// The compositor: one effect built from a stack's layers.
pub struct StackEffect {
    stack_id: String,
    layers: Vec<Layer>,
    live: LiveStack,
    areas: AreaBook,
    /// The group this stack plays in: its layers resolve in that group's canvas. `None` for the
    /// legacy layout, until the layout is gone (Task 12).
    group: Option<String>,
    clock: Clock,
    /// The frame rate the daemon is configured for; the HTML budget comes from it.
    fps: Arc<AtomicU32>,
    /// Where the held and faulted layers are reported to the API.
    statuses: Option<Arc<Mutex<Vec<StackLayerStatus>>>>,
    /// The versions the resolved areas were built at.
    live_version: u64,
    area_version: u64,
    /// What each layer resolved to at those versions, with the values it resolved with.
    resolved: Vec<ResolvedLayer>,
    /// The tile seed forwarded to every layer, when the tile path set one.
    seed: Option<canvas_proto::PropValue>,
    /// The render size the layer canvases are built for.
    frame_size: (u32, u32),
    /// True once the compositor's own frame work panicked: it logs once and keeps going.
    frame_panicked: bool,
    /// True once the compositor's own start panicked: it logs once and keeps going.
    init_panicked: bool,
    /// How the stack itself is read on a strip (#407): the column when every layer that paints
    /// answers the column, the cell otherwise.
    ///
    /// The answer is read once, when the effect is set, so it cannot follow a layer that
    /// comes and goes. It is the column only when the layers agree on it: then the picture the
    /// compositor draws is the picture the engine reads over the full height. When the layers
    /// disagree the stack answers the cell, and the strip shows the picture's cell at
    /// mid-height.
    read: StripRead,
    /// Each layer's own strip read (#427), in layer order. The per-LED path reads a layer in its
    /// own shape: a column layer over the full height, a cell layer over its cell.
    layer_reads: Vec<StripRead>,
    /// True when at least one painting layer answers the column and at least one answers the cell
    /// (#427): only then does a strip need the per-LED path. A stack whose layers agree keeps the
    /// single picture read, so a single Fire or a stack of all-column layers is unchanged.
    mixed_reads: bool,
    /// Scratch buffers for the per-LED path (#427), grown once and reused so the hot path
    /// allocates nothing. `led_acc` is the accumulator of blended reads; `led_read` and
    /// `led_alpha_read` are one light's samples of a layer's premultiplied source and coverage.
    led_acc: Vec<[f32; 3]>,
    led_read: Vec<Rgb8>,
    led_alpha_read: Vec<Rgb8>,
    /// One frame's painted layers, indexed by layer index: `src` holds each layer's source
    /// premultiplied by its coverage, `alpha` the coverage itself. Painted once after `update`
    /// on the first per-LED read of the frame and reused for every other strip light (#427 fix
    /// pass 2). The canvases are allocated at construction and grown with the frame size, never
    /// per frame, so the hot path still allocates nothing.
    led_painted_src: Vec<Canvas>,
    led_painted_alpha: Vec<Canvas>,
    /// True when `led_painted_*` holds the current frame's layers. `frame` clears it; the first
    /// per-LED read of a frame paints and sets it.
    led_cache_valid: bool,
    /// True once the per-LED read panicked: it logs once and the strip falls back to sampling.
    led_panicked: bool,
}

impl StackEffect {
    /// Builds every layer of `stack`.
    ///
    /// `leaf` builds one layer's effect and refuses `stack:` ids; a layer whose effect does not
    /// build is kept, skipped, and reported missing.
    pub fn new(stack: &Stack, leaf: LeafBuilder, live: LiveStack, areas: AreaBook) -> Self {
        let mut layers: Vec<Layer> = stack
            .layers
            .iter()
            .take(STACK_MAX_LAYERS)
            .map(|layer| {
                let built = if layer.effect.starts_with(STACK_ID_PREFIX) {
                    tracing::warn!(
                        stack = %stack.id,
                        layer = %layer.id,
                        effect = %layer.effect,
                        "a stack cannot contain a stack; the layer is skipped"
                    );
                    None
                } else {
                    match leaf(&layer.effect) {
                        Ok(effect) => Some(effect),
                        Err(error) => {
                            tracing::warn!(
                                stack = %stack.id,
                                layer = %layer.id,
                                effect = %layer.effect,
                                %error,
                                "a layer's effect did not build; the layer is skipped"
                            );
                            None
                        }
                    }
                };
                let (mask, mask_error) = match &layer.mask {
                    None => (None, None),
                    Some(spec) => {
                        let values = LiveMask::from_mask(spec);
                        if spec.effect.starts_with(STACK_ID_PREFIX) {
                            let reason = format!(
                                "{} is a stack, and a stack cannot contain a stack",
                                spec.effect
                            );
                            tracing::warn!(
                                stack = %stack.id,
                                layer = %layer.id,
                                mask = %spec.effect,
                                "a layer's mask names a stack; the layer draws unmasked"
                            );
                            (None, Some(reason))
                        } else {
                            match leaf(&spec.effect) {
                                Ok(effect) => (
                                    Some(MaskLayer {
                                        effect: spec.effect.clone(),
                                        built: effect,
                                        canvas: Canvas::new(1, 1),
                                        applied: values.clone(),
                                        values,
                                        hold: Hold::Running,
                                        last_ms: None,
                                        panic_logged: false,
                                        fault: None,
                                    }),
                                    None,
                                ),
                                Err(error) => {
                                    tracing::warn!(
                                        stack = %stack.id,
                                        layer = %layer.id,
                                        mask = %spec.effect,
                                        %error,
                                        "a layer's mask did not build; the layer draws unmasked"
                                    );
                                    (None, Some(error))
                                }
                            }
                        }
                    }
                };
                Layer {
                    id: layer.id.clone(),
                    name: layer.name.clone(),
                    effect: layer.effect.clone(),
                    built,
                    canvas: Canvas::new(1, 1),
                    values: LiveLayer::from_layer(layer),
                    hold: Hold::Running,
                    last_ms: None,
                    panic_logged: false,
                    strip_read_faulted: false,
                    mask,
                    mask_error,
                }
            })
            .collect();
        // Each layer is read on its own (#407): a layer whose meaning is how high its picture
        // reaches (Fire, Audio Spectrum) is read over the LED's column, every other layer over
        // the LED's cell. The stack answers the column only when every layer that paints
        // answers it, and the cell otherwise.
        let mut layer_reads: Vec<StripRead> = Vec::with_capacity(layers.len());
        for layer in layers.iter_mut() {
            let Some(effect) = layer.built.as_ref() else {
                // A layer that did not build paints nothing: it does not answer anything.
                layer_reads.push(StripRead::Cell);
                continue;
            };
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| effect.strip_read())) {
                Ok(read) => layer_reads.push(read),
                Err(payload) => {
                    // The engine's contract for the same call (`canvas-core`'s engine): an effect
                    // that panics while it is asked how a strip reads it is installed as failed,
                    // with the text of the panic, and is not called again until it is replaced
                    // (#407). The layer is faulted here: its picture is black, it is not run
                    // again, and the rest of the stack runs.
                    let reason = panic_text(&payload);
                    layer.hold = Hold::Faulted;
                    layer.panic_logged = true;
                    layer.strip_read_faulted = true;
                    layer.canvas.fill([0.0, 0.0, 0.0]);
                    tracing::error!(
                        stack = %stack.id,
                        layer = %layer.id,
                        effect = %layer.effect,
                        reason = %reason,
                        "a layer's effect panicked while it was asked how a strip reads it; it is faulted, its picture is black and the rest of the stack runs"
                    );
                    layer_reads.push(StripRead::Cell);
                }
            }
        }
        // The stack answers the column when every layer that paints answers it (#407): the
        // picture the compositor draws is then the picture the engine reads over the full
        // height, and a flame keeps its height. A layer that is hidden, that has no opacity or
        // that did not build paints nothing, so it does not hold the answer back.
        let mut painting = 0usize;
        let mut every_painting_layer_answers_the_column = true;
        let mut any_column = false;
        let mut any_cell = false;
        for (layer, read) in layers.iter().zip(layer_reads.iter()) {
            let paints = layer.values.visible
                && layer.values.opacity > 0
                && layer.built.is_some()
                && !matches!(layer.hold, Hold::Faulted);
            if !paints {
                continue;
            }
            painting += 1;
            if *read != StripRead::Column {
                every_painting_layer_answers_the_column = false;
                any_cell = true;
            } else {
                any_column = true;
            }
        }
        let read = if painting > 0 && every_painting_layer_answers_the_column {
            StripRead::Column
        } else {
            StripRead::Cell
        };
        // A mixed stack is the one case that needs the per-LED path (#427).
        let mixed_reads = any_column && any_cell;
        Self {
            stack_id: stack.id.clone(),
            layers,
            layer_reads,
            mixed_reads,
            led_acc: Vec::new(),
            led_read: Vec::new(),
            led_alpha_read: Vec::new(),
            led_painted_src: (0..STACK_MAX_LAYERS).map(|_| Canvas::new(0, 0)).collect(),
            led_painted_alpha: (0..STACK_MAX_LAYERS).map(|_| Canvas::new(0, 0)).collect(),
            led_cache_valid: false,
            led_panicked: false,
            live,
            areas,
            group: None,
            clock: real_clock(),
            fps: Arc::new(AtomicU32::new(canvas_proto::consts::FPS_DEFAULT)),
            statuses: None,
            live_version: u64::MAX,
            area_version: u64::MAX,
            resolved: Vec::new(),
            seed: None,
            frame_size: (0, 0),
            frame_panicked: false,
            init_panicked: false,
            read,
        }
    }

    /// [`StackEffect::new`] from a tile builder, which carries a layer's load budget question as
    /// data (#58).
    ///
    /// The first layer (or mask) whose build timed out comes back with the stack: the tile path
    /// gives the tile up and tries it again later, like a user effect's tile whose load timed
    /// out, instead of caching a picture the stack does not draw. Any other build failure is
    /// still only a layer that is skipped and reported missing, as it is for the lights.
    pub fn new_for_tile(
        stack: &Stack,
        leaf: TileLeafBuilder,
        live: LiveStack,
        areas: AreaBook,
    ) -> (Self, Option<crate::thumbnail::BuildError>) {
        // The tile builder's answer is turned into the text [`StackEffect::new`] takes, and the
        // first timed-out build is kept here on the way past. One build runs at most
        // `STACK_MAX_LAYERS` leaves, so a lock on a plain `Mutex` is enough.
        let timed_out: Arc<Mutex<Option<crate::thumbnail::BuildError>>> = Arc::new(Mutex::new(None));
        let leaves: LeafBuilder = {
            let leaf = Arc::clone(&leaf);
            let timed_out = Arc::clone(&timed_out);
            Arc::new(move |id: &str| {
                leaf(id).map_err(|error| {
                    if error.timed_out {
                        let mut slot = timed_out.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        slot.get_or_insert_with(|| {
                            crate::thumbnail::BuildError::timeout(format!(
                                "the layer {id} ran out of its load budget: {}",
                                error.reason
                            ))
                        });
                    }
                    error.reason
                })
            })
        };
        let effect = Self::new(stack, leaves, live, areas);
        let timed_out = timed_out.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        (effect, timed_out)
    }

    /// The clock the hold is measured on.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// The group this stack plays in: its layers resolve in that group's canvas.
    pub fn with_group(mut self, group: &str) -> Self {
        self.group = Some(group.to_string());
        self
    }

    /// The frame rate the HTML budget is derived from.
    pub fn with_fps(mut self, fps: Arc<AtomicU32>) -> Self {
        self.fps = fps;
        self
    }

    /// Where the held and faulted states are reported.
    pub fn with_status_sink(mut self, sink: Arc<Mutex<Vec<StackLayerStatus>>>) -> Self {
        self.statuses = Some(sink);
        self
    }

    /// The layers built, for a test that counts what was built.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// The capacity of each layer's canvas, in pixels: the compositor's own allocation test
    /// checks that a reused canvas is never reallocated after the first frame (D95.6 test 14).
    pub fn layer_canvas_capacities(&self) -> Vec<usize> {
        self.layers.iter().map(|layer| layer.canvas.pixels.capacity()).collect()
    }

    /// The status of one layer, for a test: the state it reports and its last measured time.
    pub fn layer_status(&self, id: &str) -> Option<(StackLayerState, Option<f32>)> {
        self.layers
            .iter()
            .find(|layer| layer.id == id)
            .map(|layer| (state_of(layer.hold), layer.last_ms))
    }

    /// Starts a faulted layer again: a reactivation or a structure change clears it.
    fn clear_faults(&mut self) {
        for layer in &mut self.layers {
            // A layer whose `strip_read` panicked stays faulted: the engine does not call a
            // failed effect again until it is replaced, and the stack is replaced when it is
            // built again (#407).
            if !layer.strip_read_faulted {
                layer.hold = Hold::Running;
                layer.last_ms = None;
                layer.panic_logged = false;
            }
            if let Some(mask) = &mut layer.mask {
                mask.hold = Hold::Running;
                mask.last_ms = None;
                mask.panic_logged = false;
                mask.fault = None;
            }
        }
    }

    /// The render size the layer canvases are built for, from the area book.
    fn area_scale(&self) -> f32 {
        let areas = read_recovering(&self.areas);
        let map = self.group.as_deref().and_then(|id| areas.groups.get(id));
        match map {
            Some(map) if map.width > 0 => map.scale,
            _ => 1.0,
        }
    }

    /// Rebuilds a layer's canvas when the frame's size changed (a render-scale change).
    fn fit_canvases(&mut self, width: u32, height: u32) {
        if self.frame_size == (width, height) {
            return;
        }
        self.frame_size = (width, height);
        for layer in &mut self.layers {
            if layer.canvas.width != width || layer.canvas.height != height {
                layer.canvas = Canvas::new(width, height);
            }
            if let Some(mask) = &mut layer.mask {
                if mask.canvas.width != width || mask.canvas.height != height {
                    mask.canvas = Canvas::new(width, height);
                }
            }
        }
        // The per-LED path paints every layer into these the way the picture paints it, so they
        // are the frame's size too. They grow here with the layer canvases and are reused after
        // that: the first per-LED read of a frame fills them, every other strip light samples
        // them (#427 fix pass 2).
        for src in &mut self.led_painted_src {
            if src.width != width || src.height != height {
                *src = Canvas::new(width, height);
            }
        }
        for alpha in &mut self.led_painted_alpha {
            if alpha.width != width || alpha.height != height {
                *alpha = Canvas::new(width, height);
            }
        }
        self.led_cache_valid = false;
    }

    /// Blanks every layer's canvas: what a stack whose own start panicked shows.
    fn blacken_canvases(&mut self) {
        for layer in &mut self.layers {
            layer.canvas.fill([0.0, 0.0, 0.0]);
            if let Some(mask) = &mut layer.mask {
                mask.canvas.fill([0.0, 0.0, 0.0]);
            }
        }
    }

    /// Re-resolves every layer's area when the live map or the area book moved.
    ///
    /// The masks are copied out of the book, so its lock is not held while a layer's update runs.
    fn resolve_areas(&mut self) {
        let (live_version, area_version) = {
            let live = read_recovering(&self.live);
            let areas = read_recovering(&self.areas);
            (live.version, areas.version)
        };
        if live_version == self.live_version && area_version == self.area_version {
            return;
        }
        self.live_version = live_version;
        self.area_version = area_version;
        // The values are copied once, when the live map or the area book moved, and kept with
        // the area they resolved to: the frame reads them without touching either lock.
        let live = read_recovering(&self.live);
        let areas = read_recovering(&self.areas);
        let mut resolved = Vec::with_capacity(self.layers.len());
        let mut masks: Vec<Option<LiveMask>> = Vec::with_capacity(self.layers.len());
        for layer in &self.layers {
            let values = live
                .layers
                .get(&layer.id)
                .cloned()
                .unwrap_or_else(|| layer.values.clone());
            let area = areas.resolve(self.group.as_deref(), &values.area).map(OwnedArea::from_resolved);
            masks.push(values.mask.clone());
            resolved.push(ResolvedLayer { values, area });
        }
        drop(live);
        drop(areas);
        self.resolved = resolved;
        // A mask's own values are live too (#256): a slider drag or a mask property change
        // re-initialises the mask effect and rebuilds nothing. The lock is dropped first.
        for (index, mask) in masks.into_iter().enumerate() {
            if let Some(mask) = mask {
                self.apply_live_mask(index, mask);
            }
        }
    }

    /// Gives one layer's mask effect the live values it does not already hold. A panic in the
    /// mask's `init` faults the mask, and the layer draws unmasked.
    fn apply_live_mask(&mut self, index: usize, values: LiveMask) {
        let Some(layer) = self.layers.get_mut(index) else { return };
        let Some(mask) = layer.mask.as_mut() else { return };
        if mask.applied == values {
            return;
        }
        mask.values = values.clone();
        if mask.applied.props != values.props && mask.fault.is_none() {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                mask.built.init(&values.props);
            }));
            if let Err(payload) = outcome {
                let reason = panic_text(&payload);
                mask.fault = Some(reason.clone());
                mask.canvas.fill([0.0, 0.0, 0.0]);
                tracing::error!(
                    stack = %self.stack_id,
                    layer = %layer.id,
                    mask = %mask.effect,
                    reason = %reason,
                    "a layer's mask panicked while it started; the layer draws unmasked"
                );
            }
        }
        mask.applied = values;
    }

    /// Runs one layer's `update` when its hold lets it. `None` when it did not run.
    fn run_layer(&mut self, index: usize, dt: f32, inputs: &Inputs) -> Option<Duration> {
        let started = (self.clock)();
        let layer = &mut self.layers[index];
        let effect = layer.built.as_mut().expect("a built layer");
        // A panic in one layer's effect is a bug, not a dead render thread (D95.3.6).
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            effect.update(dt, &mut layer.canvas, inputs);
        }));
        let elapsed = (self.clock)().saturating_duration_since(started);
        match outcome {
            Ok(()) => Some(elapsed),
            Err(payload) => {
                // A tile's HTML layer unwinds with a `TileFailure` by design, and the tile
                // render catches it to refuse the tile; that one belongs to the caller.
                if payload.downcast_ref::<crate::thumbnail::TileFailure>().is_some() {
                    std::panic::resume_unwind(payload);
                }
                let first = !layer.panic_logged;
                layer.panic_logged = true;
                layer.hold = Hold::Faulted;
                // A panic can leave any part of the picture behind; a faulted layer composites
                // nothing visible, so the half-written canvas goes.
                layer.canvas.fill([0.0, 0.0, 0.0]);
                if first {
                    tracing::error!(
                        stack = %self.stack_id,
                        layer = %layer.id,
                        effect = %layer.effect,
                        "a layer's effect panicked; it is faulted, its picture is black and the rest of the stack runs"
                    );
                }
                None
            }
        }
    }

    /// Runs one layer's mask `update` when its hold lets it. `None` when it did not run.
    fn run_mask(&mut self, index: usize, dt: f32, inputs: &Inputs) -> Option<Duration> {
        let started = (self.clock)();
        let layer = &mut self.layers[index];
        let mask = layer.mask.as_mut()?;
        // A panic in a mask is a bug, not a dead render thread: the mask faults, the layer draws
        // unmasked, and the rest of the stack runs (#256, the #99 handling).
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mask.built.update(dt, &mut mask.canvas, inputs);
        }));
        let elapsed = (self.clock)().saturating_duration_since(started);
        match outcome {
            Ok(()) => Some(elapsed),
            Err(payload) => {
                // A tile's HTML mask unwinds with a `TileFailure` by design, and the tile
                // render catches it to refuse the tile; that one belongs to the caller.
                if payload.downcast_ref::<crate::thumbnail::TileFailure>().is_some() {
                    std::panic::resume_unwind(payload);
                }
                let reason = panic_text(&payload);
                let first = !mask.panic_logged;
                mask.panic_logged = true;
                mask.fault = Some(reason.clone());
                // A half-written mask must not shape anything: the layer draws unmasked.
                mask.canvas.fill([0.0, 0.0, 0.0]);
                if first {
                    tracing::error!(
                        stack = %self.stack_id,
                        layer = %layer.id,
                        mask = %mask.effect,
                        reason = %reason,
                        "a layer's mask panicked; the layer draws unmasked and the rest of the stack runs"
                    );
                }
                None
            }
        }
    }

    /// The pixels of one layer's canvas, composited into `target`.
    ///
    /// The stack answers one read for its whole picture (#407), and the picture is drawn the
    /// way that read reads it:
    ///
    /// - the stack answers the **column**: the picture is mapped over the area's columns, the
    ///   strip LEDs' own x-range over the full height of the canvas, and the strip reads the
    ///   whole column. A flame (Fire, Audio Spectrum) keeps its height.
    /// - the stack answers the **cell**: the picture is mapped over the area's own box and
    ///   painted where the area's LEDs read, exactly as before #407.
    ///
    /// A light that is not a strip (a fan, a keyboard) reads its cell and never the column, so
    /// when the stack answers the column those cells are painted over the area's own box, as
    /// they always were: the fan reads the layer exactly as it read before.
    fn composite_layer(
        &self,
        index: usize,
        area: &OwnedArea,
        values: &LiveLayer,
        target: &mut Canvas,
    ) {
        if values.opacity == 0 || !values.visible {
            return;
        }
        let column = self.read == StripRead::Column;
        let box_ = paint_box(area, column, target.width as f32, target.height as f32);
        // A masked area paints every pixel its LEDs read, which for a ring can reach just past
        // the rectangle the picture is mapped over (D95.2.3): that is `paint_box`'s paint box.
        self.paint_area(index, area, values, target, box_);
        if column {
            if let Some(mask) = &area.mask {
                // The lights that are not a strip read their cells, and the columns do not
                // cover them: the layer paints them over its own box, as it always did.
                let dest = area.dest;
                let paint = mask.extent().unwrap_or(dest);
                self.paint_area(
                    index,
                    area,
                    values,
                    target,
                    PaintBox { dest, paint, coverage: Coverage::CellOnly },
                );
            }
        }
    }

    /// Paints one layer's picture over the box, where the area's mask allows it.
    fn paint_area(
        &self,
        index: usize,
        area: &OwnedArea,
        values: &LiveLayer,
        target: &mut Canvas,
        box_: PaintBox,
    ) {
        let a = f32::from(values.opacity.min(100)) / 100.0;
        let paint = layer_paint(&self.layers, index, area, box_, a);
        let Some((x0, y0, x1, y1)) = paint.bounds(target) else { return };
        for y in y0..y1 {
            for x in x0..x1 {
                if let Some((source, alpha)) = paint.pixel_at(x, y) {
                    let i = (y as usize) * (target.width as usize) + x as usize;
                    target.pixels[i] = blend_pixel(values.blend, target.pixels[i], source, alpha);
                }
            }
        }
    }

    /// Writes the held and faulted layers to the status sink. A mask that is missing, held or
    /// faulted is reported on its layer, because a mask has no id of its own (#256).
    fn report_statuses(&self) {
        let Some(sink) = &self.statuses else { return };
        let mut out = Vec::new();
        for layer in &self.layers {
            let (mut state, mut last_ms) = (state_of(layer.hold), layer.last_ms);
            if state == StackLayerState::Ok {
                if layer.mask_error.is_some() {
                    state = StackLayerState::MissingEffect;
                } else if let Some(mask) = &layer.mask {
                    if mask.fault.is_some() {
                        state = StackLayerState::Faulted;
                    } else {
                        let mask_state = state_of(mask.hold);
                        if mask_state != StackLayerState::Ok {
                            state = mask_state;
                            last_ms = mask.last_ms;
                        }
                    }
                }
            }
            if matches!(
                state,
                StackLayerState::Held | StackLayerState::Faulted | StackLayerState::MissingEffect
            ) {
                out.push(StackLayerStatus {
                    id: layer.id.clone(),
                    status: state,
                    last_ms,
                });
            }
        }
        *sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = out;
    }
}

/// The API state a hold is reported as.
fn state_of(hold: Hold) -> StackLayerState {
    match hold {
        Hold::Running => StackLayerState::Ok,
        Hold::Held { .. } => StackLayerState::Held,
        Hold::Faulted => StackLayerState::Faulted,
    }
}

impl Effect for StackEffect {
    fn schema(&self) -> PropertySchema {
        let mut properties: Vec<PropertyDef> = Vec::new();
        let mut needs = InputNeeds::default();
        let mut names = Vec::new();
        for layer in &self.layers {
            if let Some(mask) = &layer.mask {
                // A mask's effect reads its own inputs too (#256).
                let mask_schema = mask.built.schema();
                needs.screen |= mask_schema.needs.screen;
                needs.audio |= mask_schema.needs.audio;
            }
            match layer.built.as_ref() {
                Some(effect) => {
                    let schema = effect.schema();
                    needs.screen |= schema.needs.screen;
                    needs.audio |= schema.needs.audio;
                    names.push(schema.name);
                    for def in schema.properties {
                        properties.push(PropertyDef {
                            key: format!("{}.{}", layer.id, def.key),
                            label: format!("{} · {}", layer.name, def.label),
                            kind: def.kind,
                        });
                    }
                }
                None => names.push(layer.effect.clone()),
            }
        }
        PropertySchema {
            id: self.stack_id.clone(),
            name: self
                .stack_id
                .strip_prefix(STACK_ID_PREFIX)
                .unwrap_or(&self.stack_id)
                .to_string(),
            description: describe(&names),
            needs,
            properties,
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    /// A stack answers the column when every layer that paints answers it, and the cell
    /// otherwise (#407).
    ///
    /// The stack draws one picture, so one read has to serve every layer: a cell at mid-height
    /// misses a flame (Fire, Audio Spectrum answer the column), and a column over the whole
    /// height averages a thin moving line (Rain) away by about thirty times. The compositor
    /// therefore draws the picture the way the read reads it: over the strip LEDs' columns when
    /// the stack answers the column, and over the area's own cells when it answers the cell.
    ///
    /// The answer is read once, when the effect is set, and it does not depend on the props.
    fn strip_read(&self) -> StripRead {
        self.read
    }

    /// #427: the colours a strip light shows when this stack's layers read it differently.
    ///
    /// Only a strip light of a mixed stack is answered here: a fan or a keyboard keeps its cell
    /// from the canvas, and a stack whose layers agree keeps its single picture read. The canvas
    /// is never touched, so the stage picture and every other light are exactly as before.
    fn led_colours(&mut self, light: LightLeds<'_>, out: &mut Vec<Rgb8>) -> bool {
        if !self.mixed_reads || light.arrangement != Arrangement::Strip {
            return false;
        }
        let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.collect_led_colours(light, out)
        }));
        match handled {
            Ok(answer) => answer,
            Err(payload) => {
                if !self.led_panicked {
                    self.led_panicked = true;
                    tracing::error!(
                        stack = %self.stack_id,
                        reason = %panic_text(&payload),
                        "the stack's per-LED read panicked; the strip falls back to sampling the canvas"
                    );
                }
                false
            }
        }
    }

    fn init(&mut self, props: &Props) {
        // The stack's own start is guarded as each layer's is: `fit_canvases`, `clear_faults`,
        // `resolve_areas` and the status report are a panic turned into a black stack, logged
        // once, and it never unwinds into the engine (D95.3.6).
        let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.start(props)));
        if let Err(payload) = started {
            // A tile's HTML layer unwinds with a `TileFailure` by design, and the tile
            // render catches it to refuse the tile; that one belongs to the caller.
            if payload.downcast_ref::<crate::thumbnail::TileFailure>().is_some() {
                std::panic::resume_unwind(payload);
            }
            self.blacken_canvases();
            // The start may not have reached the area resolution; the first frame resolves.
            self.resolved.clear();
            self.live_version = u64::MAX;
            self.area_version = u64::MAX;
            if !self.init_panicked {
                self.init_panicked = true;
                tracing::error!(
                    stack = %self.stack_id,
                    "the stack's own start panicked; its layers stay black and the stack keeps running"
                );
            }
        }
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
        // The compositor's own work is guarded as each layer's update is: a panic in the area
        // resolution, the canvas fit or the composite is a black frame for this frame, logged
        // once, and the stack keeps running (D95.3.6). It never unwinds into the engine.
        let frame = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.frame(dt, canvas, inputs);
        }));
        if let Err(payload) = frame {
            // A panic may have left the resolved areas taken out of the compositor; the next
            // frame resolves them again.
            self.resolved.clear();
            self.live_version = u64::MAX;
            self.area_version = u64::MAX;
            // A layer's tile failure is handed on to the tile render, which refuses the tile.
            if payload.downcast_ref::<crate::thumbnail::TileFailure>().is_some() {
                std::panic::resume_unwind(payload);
            }
            canvas.fill([0.0, 0.0, 0.0]);
            if !self.frame_panicked {
                self.frame_panicked = true;
                tracing::error!(
                    stack = %self.stack_id,
                    "the stack's own frame work panicked; the frame is black and the stack keeps running"
                );
            }
        }
    }
}

impl StackEffect {
    /// The body of [`Effect::init`], run inside its guard.
    fn start(&mut self, props: &Props) {
        self.seed = tile_seed(props);
        // A render-scale change rebuilds a layer's canvas; the areas know the scale.
        let scale = self.area_scale();
        let width = ((CANVAS_WIDTH as f32 * scale).round() as u32).max(1);
        let height = ((CANVAS_HEIGHT as f32 * scale).round() as u32).max(1);
        self.fit_canvases(width, height);
        // A reactivation clears a fault (D95.3.5); a layer whose start panics below faults again.
        self.clear_faults();
        let seed = self.seed.clone();
        for layer in &mut self.layers {
            let Some(effect) = layer.built.as_mut() else { continue };
            let layer_props = layer_props(props, &layer.id, seed.as_ref());
            // A panic in one layer's start is a faulted layer, not a dead render thread
            // (D95.3.6): the rest of the stack starts and runs.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                effect.init(&layer_props);
            }));
            if let Err(payload) = outcome {
                // A tile's HTML layer unwinds with a `TileFailure` by design, and the tile
                // render catches it to refuse the tile; that one belongs to the caller.
                if payload.downcast_ref::<crate::thumbnail::TileFailure>().is_some() {
                    std::panic::resume_unwind(payload);
                }
                layer.hold = Hold::Faulted;
                layer.panic_logged = true;
                // A half-started effect can leave anything behind; a faulted layer composites
                // nothing visible, so its canvas goes.
                layer.canvas.fill([0.0, 0.0, 0.0]);
                tracing::error!(
                    stack = %self.stack_id,
                    layer = %layer.id,
                    effect = %layer.effect,
                    "a layer's effect panicked while it started; it is faulted, its picture is black and the rest of the stack runs"
                );
            }
            // The mask starts after its layer (#256). A panic in the mask's start faults the
            // mask, not the layer: the layer draws unmasked and the rest of the stack runs.
            if let Some(mask) = layer.mask.as_mut() {
                let mask_props = mask.values.props.clone();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    mask.built.init(&mask_props);
                }));
                if let Err(payload) = outcome {
                    if payload.downcast_ref::<crate::thumbnail::TileFailure>().is_some() {
                        std::panic::resume_unwind(payload);
                    }
                    let reason = panic_text(&payload);
                    mask.panic_logged = true;
                    mask.fault = Some(reason.clone());
                    mask.canvas.fill([0.0, 0.0, 0.0]);
                    tracing::error!(
                        stack = %self.stack_id,
                        layer = %layer.id,
                        mask = %mask.effect,
                        reason = %reason,
                        "a layer's mask panicked while it started; the layer draws unmasked"
                    );
                }
            }
        }
        self.resolve_areas();
        self.report_statuses();
    }

    /// Paints every painting layer once per frame into reused per-layer buffers (#427 fix pass 2).
    /// [`StackEffect::collect_led_colours`] calls this on the first per-LED read after `update`;
    /// the other strip lights of that frame only sample these buffers. Each layer's source is
    /// premultiplied by its coverage so a mask that varies inside a read gives the stage's mean
    /// of `source * coverage`, not `(mean source) * (mean coverage)`.
    fn paint_led_layers(&mut self) {
        let (w, h) = self.frame_size;
        let StackEffect {
            ref layers,
            ref resolved,
            ref layer_reads,
            ref mut led_painted_src,
            ref mut led_painted_alpha,
            ..
        } = *self;
        for index in 0..layers.len() {
            let src = &mut led_painted_src[index];
            let alpha = &mut led_painted_alpha[index];
            src.fill([0.0; 3]);
            alpha.fill([0.0; 3]);
            let Some(entry) = resolved.get(index) else { continue };
            let values = &entry.values;
            if !values.visible || values.opacity == 0 || layers[index].built.is_none() {
                continue;
            }
            // A faulted layer keeps its last picture and is still composited on the stage
            // (`composite_layer`), so the per-LED read must draw it too: only the update is
            // skipped for a faulted layer, never its picture (#427 fix pass 2).
            let Some(area) = entry.area.as_ref() else { continue };
            let column = layer_reads[index] == StripRead::Column;
            let a = f32::from(values.opacity.min(100)) / 100.0;
            let paint = layer_paint(layers, index, area, paint_box(area, column, w as f32, h as f32), a);
            if let Some((x0, y0, x1, y1)) = paint.bounds(src) {
                for y in y0..y1 {
                    for x in x0..x1 {
                        if let Some((source, coverage)) = paint.pixel_at(x, y) {
                            let i = (y as usize) * (w as usize) + x as usize;
                            src.pixels[i] = [
                                source[0] * coverage,
                                source[1] * coverage,
                                source[2] * coverage,
                            ];
                            alpha.pixels[i] = [coverage, coverage, coverage];
                        }
                    }
                }
            }
        }
    }

    /// Reads each painting layer its own way and blends the reads into one colour per LED
    /// (#427). Called by [`Effect::led_colours`] for a strip light of a mixed stack, after the
    /// frame has resolved its layers. The painted layers are built once per frame and shared by
    /// every strip light; the sample buffers are reused. Nothing is allocated here and the
    /// group's picture canvas is never touched.
    fn collect_led_colours(&mut self, light: LightLeds<'_>, out: &mut Vec<Rgb8>) -> bool {
        let leds = light.leds;
        let n = leds.len();
        if n == 0 {
            out.clear();
            return true;
        }
        if self.resolved.is_empty() {
            // No frame has resolved the layers yet; let the engine sample the canvas.
            return false;
        }
        if !self.led_cache_valid {
            self.paint_led_layers();
            self.led_cache_valid = true;
        }
        let StackEffect {
            ref layers,
            ref resolved,
            ref layer_reads,
            ref mut led_acc,
            ref mut led_read,
            ref mut led_alpha_read,
            ref led_painted_src,
            ref led_painted_alpha,
            ..
        } = *self;
        led_acc.clear();
        led_acc.resize(n, [0.0; 3]);
        out.clear();
        out.resize(n, Rgb8::BLACK);
        let (w, h) = (led_painted_src[0].width, led_painted_src[0].height);
        for index in 0..layers.len() {
            let Some(entry) = resolved.get(index) else { continue };
            let values = &entry.values;
            if !values.visible || values.opacity == 0 || layers[index].built.is_none() {
                continue;
            }
            if entry.area.is_none() {
                continue;
            }
            let column = layer_reads[index] == StripRead::Column;
            let src = &led_painted_src[index];
            let cov = &led_painted_alpha[index];

            // Sample what the layer painted in its own shape: a column layer over the full
            // height, a cell layer over its cell, exactly as the engine samples a picture. Both
            // the premultiplied source and its coverage are sampled the same way, so their means
            // are taken over exactly the same pixels (#427 fix pass 2).
            if column {
                canvas_core::sampling::sample_led_columns(&src.pixels, w, h, leds, led_read);
                canvas_core::sampling::sample_led_columns(&cov.pixels, w, h, leds, led_alpha_read);
            } else {
                canvas_core::sampling::sample_leds(
                    &src.pixels, w, h, leds, canvas_core::SCENE_SAMPLING, led_read,
                );
                canvas_core::sampling::sample_leds(
                    &cov.pixels, w, h, leds, canvas_core::SCENE_SAMPLING, led_alpha_read,
                );
            }
            for i in 0..n {
                if !leds[i].sampled {
                    continue;
                }
                let alpha = f32::from(led_alpha_read[i].r) / 255.0;
                let premultiplied = led_read[i].to_f32();
                // Recover the source from the two means before blending, so the blend folds the
                // coverage back in and lands on the stage's mean of source * coverage. At zero
                // coverage the layer contributes nothing and the source does not matter.
                let source = if alpha > 0.0 {
                    [
                        premultiplied[0] / alpha,
                        premultiplied[1] / alpha,
                        premultiplied[2] / alpha,
                    ]
                } else {
                    [0.0; 3]
                };
                led_acc[i] = blend_pixel(values.blend, led_acc[i], source, alpha);
            }
        }
        for i in 0..n {
            out[i] = Rgb8::from_f32(led_acc[i]);
        }
        true
    }

    /// One frame of the compositor's own work, run inside [`StackEffect::update`]'s guard.
    fn frame(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
        // Every update makes new layer pictures, so the first strip light of this frame must
        // repaint the per-layer buffers the remaining strip lights sample (#427 fix pass 2).
        self.led_cache_valid = false;
        self.fit_canvases(canvas.width, canvas.height);
        self.resolve_areas();
        // The frame starts black: a stack with nothing to show paints black, as every effect
        // with no picture does.
        canvas.fill([0.0, 0.0, 0.0]);

        // The live values and the resolved areas were read together by `resolve_areas`, which
        // copies them only when the live map or the area book moved, so nothing is copied here.
        let budget = html_budget(self.fps.load(Ordering::Relaxed));
        let now = (self.clock)();
        let mut changed = false;
        // The resolved layers are taken out for the loop, so a layer's own update can borrow the
        // effect mutably while its values and its area are still in hand.
        let resolved = std::mem::take(&mut self.resolved);

        for index in 0..self.layers.len() {
            let Some(entry) = resolved.get(index) else { continue };
            let values = &entry.values;
            if !values.visible || values.opacity == 0 {
                continue;
            }
            let Some(area) = entry.area.as_ref() else {
                // The area covers nothing: a device or zone that is gone or unplaced.
                continue;
            };
            if self.layers[index].built.is_none() {
                continue;
            }
            let is_html = self.layers[index]
                .effect
                .starts_with(crate::user_effects::USER_EFFECT_PREFIX);
            let before = self.layers[index].hold;
            let mut ran = false;
            let mut elapsed = Duration::ZERO;
            match before {
                Hold::Faulted => {}
                Hold::Held { until, .. } if now < until => {}
                _ => {
                    if let Some(took) = self.run_layer(index, dt, inputs) {
                        ran = true;
                        elapsed = took;
                    }
                }
            }
            if ran && is_html {
                let (hold, last_ms) = overrun_hold(before, elapsed, budget, now);
                self.layers[index].hold = hold;
                self.layers[index].last_ms = last_ms;
                if let Some(ms) = last_ms {
                    if matches!(hold, Hold::Faulted) {
                        tracing::warn!(
                            stack = %self.stack_id,
                            layer = %self.layers[index].id,
                            effect = %self.layers[index].effect,
                            ms,
                            "a layer is too slow: it is faulted and keeps its last picture"
                        );
                    } else {
                        tracing::debug!(
                            stack = %self.stack_id,
                            layer = %self.layers[index].id,
                            ms,
                            "a layer overran its budget and is held"
                        );
                    }
                }
            }
            if state_of(self.layers[index].hold) != state_of(before) {
                changed = true;
            }
            // The layer's mask runs with it (#256): one more effect update per frame. A held
            // layer holds its mask too, and a faulted mask leaves the layer unmasked.
            if ran {
                let mask_state_before = self.layers[index].mask.as_ref().map(|mask| {
                    if mask.fault.is_some() {
                        StackLayerState::Faulted
                    } else {
                        state_of(mask.hold)
                    }
                });
                if let Some(mask_before) = self.layers[index].mask.as_ref().map(|mask| mask.hold) {
                    let mut mask_ran = false;
                    let mut mask_elapsed = Duration::ZERO;
                    match mask_before {
                        Hold::Faulted => {}
                        Hold::Held { until, .. } if now < until => {}
                        _ => {
                            if let Some(took) = self.run_mask(index, dt, inputs) {
                                mask_ran = true;
                                mask_elapsed = took;
                            }
                        }
                    }
                    let mask_is_html = self.layers[index]
                        .mask
                        .as_ref()
                        .is_some_and(|mask| mask.effect.starts_with(crate::user_effects::USER_EFFECT_PREFIX));
                    if mask_ran && mask_is_html {
                        let (hold, last_ms) = overrun_hold(mask_before, mask_elapsed, budget, now);
                        if let Some(mask) = self.layers[index].mask.as_mut() {
                            mask.hold = hold;
                            mask.last_ms = last_ms;
                        }
                        if let Some(ms) = last_ms {
                            if matches!(hold, Hold::Faulted) {
                                tracing::warn!(
                                    stack = %self.stack_id,
                                    layer = %self.layers[index].id,
                                    ms,
                                    "a layer's mask is too slow: it is faulted and the layer draws unmasked"
                                );
                            } else {
                                tracing::debug!(
                                    stack = %self.stack_id,
                                    layer = %self.layers[index].id,
                                    ms,
                                    "a layer's mask overran its budget and is held"
                                );
                            }
                        }
                    }
                    let mask_state_after = self.layers[index].mask.as_ref().map(|mask| {
                        if mask.fault.is_some() {
                            StackLayerState::Faulted
                        } else {
                            state_of(mask.hold)
                        }
                    });
                    if mask_state_after != mask_state_before {
                        changed = true;
                    }
                }
            }
            // A held layer keeps its last good canvas and is composited as usual; a layer that
            // panicked had its canvas blacked as it faulted, so it adds nothing to the frame.
            self.composite_layer(index, area, values, canvas);
        }
        self.resolved = resolved;
        // The frame is the composite of the layers and nothing else (#407): the stack writes
        // into the group's canvas only where its own layers paint, so the picture the editor
        // shows is the picture the lights read.
        if changed {
            self.report_statuses();
        }
    }
}

// ---------------------------------------------------------------- the group a test paints

/// Every zone of every device as one light, and the plan that lays them on a group's canvas.
///
/// The tests that check what an area paints use this: one group holding every zone, each in its
/// own slot, which is what the engine renders. The plan's samples are in the group's canvas
/// units, the same ones [`AreaMap::build_for_group`] scales.
pub fn zone_group(
    devices: &[canvas_proto::DeviceInfo],
) -> (Vec<canvas_proto::scene::Light>, canvas_core::arrangement::GroupPlan) {
    use canvas_proto::scene::{zone_light_id, Arrangement, GroupMode, Light};
    let lights: Vec<Light> = devices
        .iter()
        .flat_map(|device| {
            device
                .zones
                .iter()
                .filter(|zone| zone.led_count > 0)
                .map(move |zone| Light {
                    id: zone_light_id(&device.id, zone.index),
                    device_id: device.id.clone(),
                    zone_index: zone.index,
                    leds: None,
                    name: None,
                    arrangement: Arrangement::default_for(&zone.kind),
                    reversed: false,
                    kind: None,
                    copies: None,
                    first_led: None,
                })
        })
        .collect();
    let members: Vec<String> = lights.iter().map(|l| l.id.clone()).collect();
    let plan = canvas_core::arrangement::build_group_plan(&members, GroupMode::FlowAcross, &lights, devices);
    (lights, plan)
}

/// Samples every light's LEDs from a painted group canvas, as the engine does: per device, in
/// zone order, with a zone the group does not hold left black.
///
/// The tests compare the compositor's frame with this: the pixels a layer painted are the ones
/// the lights of that area read.
pub fn sample_group(
    plan: &canvas_core::arrangement::GroupPlan,
    lights: &[canvas_proto::scene::Light],
    devices: &[canvas_proto::DeviceInfo],
    canvas: &Canvas,
) -> BTreeMap<DeviceId, Vec<canvas_proto::Rgb8>> {
    let mut out: BTreeMap<DeviceId, Vec<canvas_proto::Rgb8>> = devices
        .iter()
        .map(|device| (device.id.clone(), Vec::new()))
        .collect();
    for samples in plan.lights.iter() {
        let Some(light) = lights.iter().find(|l| l.id == samples.light) else {
            continue;
        };
        let frame = out.entry(light.device_id.clone()).or_default();
        let start = samples.frame_start as usize;
        if frame.len() < start + samples.leds.len() {
            frame.resize(start + samples.leds.len(), canvas_proto::Rgb8::BLACK);
        }
        let mut colors = Vec::new();
        // The plan's samples are in the group's canvas units; the canvas is the scaled one.
        let scaled: Vec<canvas_core::geometry::LedSample> = samples
            .leds
            .iter()
            .map(|sample| canvas_core::geometry::LedSample {
                center_x: sample.center_x * canvas_core::SCENE_RENDER_SCALE,
                center_y: sample.center_y * canvas_core::SCENE_RENDER_SCALE,
                bounds: sample.bounds.map(|v| v * canvas_core::SCENE_RENDER_SCALE),
                sampled: sample.sampled,
            })
            .collect();
        canvas_core::sampling::sample_leds(
            &canvas.pixels,
            canvas.width,
            canvas.height,
            &scaled,
            canvas_core::SCENE_SAMPLING,
            &mut colors,
        );
        frame[start..start + colors.len()].copy_from_slice(&colors);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
    use canvas_proto::props::PropValue;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    /// The blend table of D95.3.3, every mode at opacity 0, 50 and 100.
    #[test]
    fn the_blend_table_is_the_specs() {
        // d, s, and the a = 1 result of normal, add, multiply, screen.
        let cases = [
            (0.5f32, 0.8f32, 0.8f32, 1.0f32, 0.4f32, 0.9f32),
            (0.2, 0.5, 0.5, 0.7, 0.1, 0.6),
            (0.9, 0.4, 0.4, 1.0, 0.36, 0.94),
        ];
        for (d, s, normal, add, multiply, screen) in cases {
            // At opacity 0 every mode leaves what is under it alone.
            for mode in [BlendMode::Normal, BlendMode::Add, BlendMode::Multiply, BlendMode::Screen] {
                let out = blend_pixel(mode, [d; 3], [s; 3], 0.0);
                assert!(near(out[0], d), "{mode:?} at a=0 changed {d} to {}", out[0]);
                // Half opacity is halfway between d and the mode's own result.
                let at_one = blend_pixel(mode, [d; 3], [s; 3], 1.0);
                let half = blend_pixel(mode, [d; 3], [s; 3], 0.5);
                assert!(
                    near(half[0], d + 0.5 * (at_one[0] - d)),
                    "{mode:?} at a=0.5 is not halfway between d and a=1: {} vs {}",
                    half[0],
                    at_one[0]
                );
            }
            for (mode, want) in [
                (BlendMode::Normal, normal),
                (BlendMode::Add, add),
                (BlendMode::Multiply, multiply),
                (BlendMode::Screen, screen),
            ] {
                let out = blend_pixel(mode, [d; 3], [s; 3], 1.0);
                assert!(near(out[0], want), "{mode:?} at a=1: {d}/{s} gave {} want {want}", out[0]);
            }
        }
        // Add at d = 0.5, s = 0.8, a = 0.5 is 0.75: the blend is clamped before opacity.
        let out = blend_pixel(BlendMode::Add, [0.5; 3], [0.8; 3], 0.5);
        assert!(near(out[0], 0.75), "add at 0.5/0.8/0.5 is 0.75, got {}", out[0]);
        // A black part of the layer, at full opacity: normal hides what is under, add and
        // screen leave it, multiply blacks it out.
        for (mode, want) in [
            (BlendMode::Normal, 0.0f32),
            (BlendMode::Add, 0.5),
            (BlendMode::Multiply, 0.0),
            (BlendMode::Screen, 0.5),
        ] {
            let out = blend_pixel(mode, [0.5; 3], [0.0; 3], 1.0);
            assert!(near(out[0], want), "{mode:?} at s=0 gave {} want {want}", out[0]);
        }
        // The same at half opacity: normal and multiply fade, add and screen leave it.
        for (mode, want) in [
            (BlendMode::Normal, 0.25f32),
            (BlendMode::Add, 0.5),
            (BlendMode::Multiply, 0.25),
            (BlendMode::Screen, 0.5),
        ] {
            let out = blend_pixel(mode, [0.5; 3], [0.0; 3], 0.5);
            assert!(near(out[0], want), "{mode:?} at s=0, a=0.5 gave {} want {want}", out[0]);
        }
    }

    #[test]
    fn every_value_is_clamped_and_a_nan_is_zero() {
        // At opacity 0 nothing is applied, so what is under the layer is visible, clamped.
        let out = blend_pixel(BlendMode::Normal, [f32::NAN, 2.0, -1.0], [0.5; 3], 0.0);
        assert_eq!(out, [0.0, 1.0, 0.0]);
        // An infinite source is white, and Normal at half opacity moves halfway to it.
        let out = blend_pixel(BlendMode::Normal, [0.5; 3], [f32::INFINITY; 3], 0.5);
        assert_eq!(out, [0.75, 0.75, 0.75]);
        let out = blend_pixel(BlendMode::Add, [0.8; 3], [0.8; 3], 1.0);
        assert!(near(out[0], 1.0), "add clamps at 1, got {}", out[0]);
    }

    #[test]
    fn the_html_budget_is_the_cap_or_the_share() {
        assert_eq!(html_budget(10), Duration::from_secs_f64(0.008));
        assert_eq!(html_budget(30), Duration::from_secs_f64(0.008));
        assert_eq!(html_budget(50), Duration::from_secs_f64(0.008));
        // 40 % of 16.67 ms is 6.67 ms at 60 fps.
        let at_60 = html_budget(60).as_secs_f64() * 1000.0;
        assert!((at_60 - 6.6667).abs() < 0.01, "{at_60}");
    }

    fn zone(index: u32, kind: ZoneKind, led_count: u32) -> ZoneInfo {
        ZoneInfo {
            index,
            name: format!("zone{index}"),
            kind,
            led_count,
            min_leds: led_count,
            max_leds: led_count,
            resizable: false,
        }
    }

    fn device(id: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(id.into()),
            name: id.into(),
            vendor: "test".into(),
            kind: DeviceKind::Other,
            adapter: "fake".into(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones,
            connected: true,
        }
    }

    // ---------------------------------------------------------------- the group's areas

    /// A light of the hardware map.
    fn light(id: &str, device: &str, zone: u32) -> canvas_proto::scene::Light {
        canvas_proto::scene::Light {
            id: id.into(),
            device_id: DeviceId(device.into()),
            zone_index: zone,
            leds: None,
            name: None,
            arrangement: canvas_proto::scene::Arrangement::Strip,
            reversed: false,
            kind: None,
            copies: None,
            first_led: None,
        }
    }

    /// The masks of one group: its members' lights, arranged on the group's canvas.
    fn group_map(
        members: &[&str],
        lights: &[canvas_proto::scene::Light],
        devices: &[DeviceInfo],
        mode: canvas_proto::scene::GroupMode,
    ) -> AreaMaps {
        let plan = canvas_core::arrangement::build_group_plan(
            &members.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
            mode,
            lights,
            devices,
        );
        AreaMaps {
            version: 1,
            groups: [(
                "g".to_string(),
                AreaMap::build_for_group(&plan, lights, SamplingMode::AreaAverage, 1),
            )]
            .into_iter()
            .collect(),
        }
    }

    /// A cell thinner than a pixel in one direction but not the other (a strip LED of #372) is
    /// masked at the pixel `sampling::sample_leds` reads, not at the nearest pixel.
    #[test]
    fn a_cell_thin_in_one_direction_masks_the_pixel_the_sampler_reads() {
        // Centre (24.69, 25.0); x spans 24.49..24.90 (holds pixel 24's centre, 24.5), y spans
        // 24.80..25.20 (holds no pixel centre). The sampler reads (24, 25); the nearest pixel is (25, 25).
        let led = geometry::LedSample { center_x: 24.69, center_y: 25.0, bounds: [24.49, 24.80, 24.90, 25.20], sampled: true };
        let mut mask = Mask::new(80, 50);
        mask.add_led(&led, SamplingMode::AreaAverage, false);
        assert!(mask.contains(24, 25), "the pixel the sampler reads");
        assert!(!mask.contains(25, 25), "not the nearest pixel");
        let mut canvas = vec![[0.0f32; 3]; 80 * 50];
        canvas[25 * 80 + 24] = [1.0, 0.0, 0.0];
        let mut out = Vec::new();
        canvas_core::sampling::sample_leds(&canvas, 80, 50, &[led], SamplingMode::AreaAverage, &mut out);
        assert_eq!(out[0].r, 255, "the sampler reads that pixel");
    }

    /// #407: a strip LED's mask is built for the column as well: the column read covers the
    /// LED's own x-range over the full height of the canvas, and the cell read only the cell at
    /// mid-height. A layer in that area is painted over the column, so the strip reads the whole
    /// picture.
    #[test]
    fn a_strip_leds_mask_covers_its_whole_column_for_the_column_read() {
        let devices = vec![device("a", vec![zone(0, ZoneKind::Linear, 4)])];
        let lights = vec![light("a#z0", "a", 0)];
        let map = group_map(&["a#z0"], &lights, &devices, canvas_proto::scene::GroupMode::FlowAcross);
        let area = LayerArea::Device { device: DeviceId("a".into()) };
        let resolved = map
            .resolve(Some("g"), &area)
            .expect("the device has a mask");
        let mask = resolved.mask.expect("the device has a mask");
        // The column reaches the whole canvas, the cell only the band at mid-height.
        let column = mask.column_extent().expect("the column has a box");
        assert!(column[1] <= 0.0, "the column starts at the top: {column:?}");
        assert!(
            (column[3] - 50.0).abs() < 0.001,
            "the column reaches the bottom: {column:?}"
        );
        let cell = mask.extent().expect("the cell has a box");
        assert!(cell[1] > 0.0, "the cell is not at the top: {cell:?}");
        assert!(cell[3] < 50.0, "the cell is not at the bottom: {cell:?}");
        // A pixel of the leftmost column above the cell is covered by the column read only.
        let x = column[0] as u32;
        assert!(mask.contains_column(x, 0), "the column read reaches the top");
        assert!(!mask.contains(x, 0), "the cell read does not");
        // The strip's own cell is inside the column: the column is the full height of the
        // strip's own x-range.
        for y in 0..50 {
            for x in 0..80 {
                if mask.contains(x, y) {
                    assert!(
                        mask.contains_column(x, y),
                        "the column covers the strip's cell pixel ({x}, {y})"
                    );
                }
            }
        }
    }

    /// #407: a light that is not a strip reads its cell and never the column, so its cell is
    /// not in the column read: the fan in the same group reads the layer as it did before.
    #[test]
    fn a_fans_cell_is_not_in_the_column_read() {
        let devices = vec![device(
            "a",
            vec![zone(0, ZoneKind::Linear, 4), zone(1, ZoneKind::Linear, 4)],
        )];
        let mut lights = vec![light("a#z0", "a", 0)];
        // The second light of the device is a fan: a ring is not a strip.
        let mut fan = light("a#z1", "a", 1);
        fan.arrangement = canvas_proto::scene::Arrangement::Ring;
        lights.push(fan);
        let map = group_map(
            &["a#z0", "a#z1"],
            &lights,
            &devices,
            canvas_proto::scene::GroupMode::FlowAcross,
        );
        let area = LayerArea::Device { device: DeviceId("a".into()) };
        let resolved = map
            .resolve(Some("g"), &area)
            .expect("the device has a mask");
        let mask = resolved.mask.expect("the device has a mask");
        // The fan's cell is in the cell read.
        let mut fan_cells = 0;
        for y in 0..50 {
            for x in 0..80 {
                if mask.contains(x, y) && !mask.contains_column(x, y) {
                    fan_cells += 1;
                }
            }
        }
        assert!(fan_cells > 0, "the fan's cell is in the cell read");
    }

    /// A group the book does not hold resolves to nothing: "not in this group".
    #[test]
    fn an_unknown_group_resolves_to_nothing() {
        let devices = vec![device("a", vec![zone(0, ZoneKind::Linear, 4)])];
        let lights = vec![light("a#z0", "a", 0)];
        let map = group_map(&["a#z0"], &lights, &devices, canvas_proto::scene::GroupMode::FlowAcross);
        let area = LayerArea::Device { device: DeviceId("a".into()) };
        assert!(
            map.resolve(Some("not-a-group"), &area).is_none(),
            "a group the book does not hold paints nothing"
        );
        assert!(map.is_empty_area(Some("not-a-group"), &area));
        // The group the book holds still resolves.
        assert!(map.resolve(Some("g"), &area).is_some());
    }

    /// A device's area covers every light of that device in the group, and no other device's.
    #[test]
    fn a_device_area_covers_every_light_of_that_device_in_the_group() {
        let devices = vec![
            device("a", vec![zone(0, ZoneKind::Linear, 4), zone(1, ZoneKind::Linear, 4)]),
            device("b", vec![zone(0, ZoneKind::Linear, 4)]),
        ];
        let lights = vec![light("a#z0", "a", 0), light("a#z1", "a", 1), light("b#z0", "b", 0)];
        // The group holds both lights of a and none of b.
        let map = group_map(&["a#z0", "a#z1"], &lights, &devices, canvas_proto::scene::GroupMode::FlowAcross);
        let area = LayerArea::Device { device: DeviceId("a".into()) };
        let resolved = map.resolve(Some("g"), &area).expect("device a is in the group");
        let mask = resolved.mask.expect("a device area has a mask");
        // Both zones of a are covered: the mask holds every LED of the device.
        let plan = canvas_core::arrangement::build_group_plan(
            &["a#z0".to_string(), "a#z1".to_string()],
            canvas_proto::scene::GroupMode::FlowAcross,
            &lights,
            &devices,
        );
        for sample in plan.lights.iter().flat_map(|l| l.leds.iter()) {
            let x = (sample.center_x * canvas_core::SCENE_RENDER_SCALE).round() as u32;
            let y = (sample.center_y * canvas_core::SCENE_RENDER_SCALE).round() as u32;
            assert!(mask.contains(x, y), "the LED at {x},{y} is covered");
        }
        // And the device's own box is the box of those LEDs.
        assert!(resolved.dest[2] > resolved.dest[0], "the device has a box: {:?}", resolved.dest);
    }

    /// A device the group does not hold covers nothing, and the layer says so.
    #[test]
    fn an_area_for_a_device_outside_the_group_lights_nothing_and_says_so() {
        let devices = vec![
            device("a", vec![zone(0, ZoneKind::Linear, 4)]),
            device("b", vec![zone(0, ZoneKind::Linear, 4)]),
        ];
        let lights = vec![light("a#z0", "a", 0), light("b#z0", "b", 0)];
        let map = group_map(&["a#z0"], &lights, &devices, canvas_proto::scene::GroupMode::FlowAcross);
        let area = LayerArea::Device { device: DeviceId("b".into()) };
        assert!(map.resolve(Some("g"), &area).is_none(), "device b is not in the group");
        assert!(map.is_empty_area(Some("g"), &area), "the area covers nothing");
        // A zone of a device the group does not hold, too.
        let zone = LayerArea::Zone { device: DeviceId("b".into()), zone: 0 };
        assert!(map.resolve(Some("g"), &zone).is_none(), "zone 0 of b is not in the group");
    }

    /// A rectangle is a rectangle of the group's canvas: the same 320x200 numbers.
    #[test]
    fn a_rect_area_is_a_rectangle_of_the_group_canvas() {
        let devices = vec![device("a", vec![zone(0, ZoneKind::Linear, 4)])];
        let lights = vec![light("a#z0", "a", 0)];
        let map = group_map(&["a#z0"], &lights, &devices, canvas_proto::scene::GroupMode::FlowAcross);
        let area = LayerArea::Rect { x: 16.0, y: 150.0, width: 110.0, height: 46.0 };
        let resolved = map.resolve(Some("g"), &area).expect("a rectangle covers the canvas");
        assert!(resolved.mask.is_none(), "a rectangle has no per-LED mask");
        let scale = canvas_core::SCENE_RENDER_SCALE;
        assert_eq!(resolved.dest, [16.0 * scale, 150.0 * scale, 126.0 * scale, 196.0 * scale]);
        // The whole canvas is the whole canvas.
        let whole = map.resolve(Some("g"), &LayerArea::Canvas).expect("the canvas");
        assert!(whole.whole_canvas);
        assert_eq!(whole.dest, [0.0, 0.0, 80.0, 50.0], "the group canvas at the scene's render scale");
    }

    /// A device's mask covers its own LEDs and no other device's.
    #[test]
    fn a_device_mask_covers_its_own_leds_and_no_other_device() {
        let devices = vec![
            device("a", vec![zone(0, ZoneKind::Linear, 4)]),
            device("b", vec![zone(0, ZoneKind::Linear, 4)]),
        ];
        let lights = vec![light("a#z0", "a", 0), light("b#z0", "b", 0)];
        let map = group_map(
            &["a#z0", "b#z0"],
            &lights,
            &devices,
            canvas_proto::scene::GroupMode::FlowAcross,
        );
        let group = map.groups.get("g").expect("the group's map");
        let a = group.devices.get(&DeviceId("a".into())).expect("device a has a mask");
        let b = group.devices.get(&DeviceId("b".into())).expect("device b has a mask");
        // The pixels a device's mask holds are the ones its LEDs' cells cover, and an LED
        // reads the pixel at its own centre (#372: the cell is a small square at the LED,
        // so the mask holds the pixel the sampler reads).
        let plan = canvas_core::arrangement::build_group_plan(
            &["a#z0".to_string(), "b#z0".to_string()],
            canvas_proto::scene::GroupMode::FlowAcross,
            &lights,
            &devices,
        );
        let scale = canvas_core::SCENE_RENDER_SCALE;
        let mut covered = 0;
        for samples in plan.lights.iter().filter(|l| l.device_id == DeviceId("a".into())) {
            for led in &samples.leds {
                let x = (led.center_x * scale).round() as u32;
                let y = (led.center_y * scale).round() as u32;
                assert!(a.contains(x, y), "device a's own LED at {x},{y} is covered");
                assert!(!b.contains(x, y), "device b is outside a's mask");
                covered += 1;
            }
        }
        assert_eq!(covered, 4, "every one of device a's LEDs was checked");
        // The area itself, not only the LED's centre: every pixel the LED's cell covers is
        // the device's area, under the rule the sampler reads (#372). The cell is a small
        // square at the LED, so the area is the square the sampler averages over.
        for samples in plan.lights.iter().filter(|l| l.device_id == DeviceId("a".into())) {
            for led in &samples.leds {
                let [bx0, by0, bx1, by1] = led.bounds;
                let x0 = (((bx0 * scale) - 0.5).ceil() as i64).clamp(0, 79) as u32;
                let y0 = (((by0 * scale) - 0.5).ceil() as i64).clamp(0, 49) as u32;
                let x1 = ((((bx1 * scale) - 0.5).floor() as i64).clamp(0, 79) as u32).max(x0);
                let y1 = ((((by1 * scale) - 0.5).floor() as i64).clamp(0, 49) as u32).max(y0);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        assert!(a.contains(x, y), "device a's area holds the pixel {x},{y} its LED reads");
                        assert!(!b.contains(x, y), "device b is outside a's area");
                    }
                }
            }
        }
        assert!((group.width, group.height) == (80, 50), "the group canvas at the scene's scale");
    }

    #[test]
    fn a_rect_with_a_nan_is_no_area_and_a_negative_one_too() {
        // A library tile has no group, so no masks: a canvas and a rectangle still resolve.
        let book = AreaMaps::default();
        assert!(book
            .resolve(None, &LayerArea::Rect { x: f32::NAN, y: 0.0, width: 10.0, height: 10.0 })
            .is_none());
        assert!(book
            .resolve(None, &LayerArea::Rect { x: 0.0, y: 0.0, width: -4.0, height: 10.0 })
            .is_none());
        assert!(book.resolve(None, &LayerArea::Canvas).is_some());
    }

    /// A guard (#382): `copies` only counts. The plan and the sampled frame of a mirrored header
    /// (one light of 8 LEDs, shown 8 times) are the same with `copies` 8 and with none, and both
    /// are 8 LEDs long: the engine writes the run once, never 64.
    #[test]
    fn the_frames_of_a_mirrored_header_are_the_same_with_copies_eight_and_with_none() {
        use canvas_proto::layout::LedSlice;
        let devices = vec![device("hdr", vec![zone(0, ZoneKind::Linear, 8)])];
        let mut once = light("hdr#z0p1", "hdr", 0);
        once.leds = Some(LedSlice { start: 0, count: 8 });
        let mut mirrored = once.clone();
        mirrored.copies = Some(8);
        let members = vec![once.id.clone()];
        let plan_of = |l: &canvas_proto::scene::Light| {
            canvas_core::arrangement::build_group_plan(
                &members,
                canvas_proto::scene::GroupMode::FlowAcross,
                std::slice::from_ref(l),
                &devices,
            )
        };
        let (plan_once, plan_mirrored) = (plan_of(&once), plan_of(&mirrored));
        assert_eq!(plan_once, plan_mirrored, "copies changes nothing in the plan");
        assert_eq!(plan_mirrored.lights[0].leds.len(), 8, "the run is planned once, not 8 times");

        // A canvas that is a ramp, so every LED reads its own colour.
        let mut canvas = Canvas::new(64, 64);
        for (i, px) in canvas.pixels.iter_mut().enumerate() {
            let v = (i % 64) as f32 / 63.0;
            *px = [v, 1.0 - v, 0.5];
        }
        let frame_once = sample_group(&plan_once, std::slice::from_ref(&once), &devices, &canvas);
        let frame_mirrored =
            sample_group(&plan_mirrored, std::slice::from_ref(&mirrored), &devices, &canvas);
        assert_eq!(frame_once, frame_mirrored, "the frames are byte for byte the same");
        assert_eq!(frame_mirrored[&DeviceId("hdr".into())].len(), 8, "8 LEDs, not 64");
    }

    #[test]
    fn props_are_namespaced_and_unnamespaced_again() {
        let mut merged = Props::new();
        merged.insert("l1.speed".into(), PropValue::Number(9.0));
        merged.insert("l2.speed".into(), PropValue::Number(3.0));
        merged.insert(canvas_effects::TILE_SEED_PROP.into(), PropValue::Number(7.0));
        let layer = layer_props(&merged, "l1", Some(&PropValue::Number(7.0)));
        assert_eq!(layer.get("speed"), Some(&PropValue::Number(9.0)));
        assert_eq!(
            layer.get(canvas_effects::TILE_SEED_PROP),
            Some(&PropValue::Number(7.0)),
            "the tile seed reaches every layer"
        );
        assert_eq!(layer.len(), 2, "another layer's keys do not leak in");
    }
}
