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
//!   same sample plan the engine samples with. It is rebuilt when the layout, the render scale or
//!   the sampling mode changes, never per frame. A device area lights the pixels its own LEDs
//!   read, and a second device drawn over those same pixels reads them too: the canvas has no
//!   z-order, so two overlapping footprints share the pixels they share.
//! - [`LiveMap`]: each layer's blend, opacity, visibility and area, read once per frame, so a
//!   slider drag or a blend change costs no rebuild.
//! - [`StackEffect`]: builds one effect per layer, calls each layer's `update` into its own
//!   canvas, and composites that canvas into the frame through the layer's area mask.

use canvas_core::geometry;
use canvas_proto::api::{StackLayerState, StackLayerStatus};
use canvas_proto::device::DeviceId;
use canvas_proto::layout::{Layout, PlacedZone, SamplingMode, CANVAS_HEIGHT, CANVAS_WIDTH};
use canvas_proto::props::{InputNeeds, Props, PropertyDef, PropertySchema};
use canvas_proto::stack::{
    BlendMode, LayerArea, LayerMask, MaskSource, Stack, STACK_ID_PREFIX, STACK_MAX_LAYERS,
};
use canvas_proto::traits::{Canvas, Effect, Inputs};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// How an effect id is built for one layer: the lights' builders or the tile's. A leaf builder
/// refuses a `stack:` id outright, so a stack can never contain a stack (D95.4.8).
pub type LeafBuilder = Arc<dyn Fn(&str) -> Result<Box<dyn Effect>, String> + Send + Sync>;

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

/// The area book every `StackEffect` resolves its layers against.
pub type AreaBook = Arc<RwLock<AreaMap>>;

// ---------------------------------------------------------------- the area book

/// One area's pixels, in render pixels: where the picture is mapped, and which pixels it may
/// paint. A bit is set where the area covers the pixel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mask {
    /// The bounding box of the area's rectangles, in render pixels: the layer's whole picture is
    /// mapped over this box.
    pub dest: [f32; 4],
    /// The bounding box of the pixels the mask covers, in render pixels. It can reach past
    /// `dest`: a ring's cell at the edge of a fan reads pixels just outside the fan's rectangle,
    /// and those must be painted or the LED would average black into it (D95.2.3).
    extent: Option<[f32; 4]>,
    words: Vec<u64>,
    width: u32,
    height: u32,
}

impl Mask {
    /// An empty mask over a canvas of `width` by `height` render pixels.
    pub fn new(width: u32, height: u32) -> Self {
        let bits = (width as usize) * (height as usize);
        Self {
            dest: [0.0, 0.0, 0.0, 0.0],
            extent: None,
            words: vec![0; bits.div_ceil(64)],
            width,
            height,
        }
    }

    /// The bounding box of the pixels the mask covers, or `None` when it covers none.
    pub fn extent(&self) -> Option<[f32; 4]> {
        self.extent
    }

    /// True when the mask covers no pixel at all.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    /// True when the mask covers the pixel at `x`, `y`.
    #[inline]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let bit = (y as usize) * (self.width as usize) + (x as usize);
        self.words[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    /// Sets the bit for one pixel; out-of-canvas pixels are ignored.
    #[inline]
    fn set(&mut self, x: i64, y: i64) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let bit = (y as usize) * (self.width as usize) + (x as usize);
        self.words[bit / 64] |= 1u64 << (bit % 64);
        let (fx, fy) = (x as f32, y as f32);
        self.extent = Some(match self.extent {
            Some([x0, y0, x1, y1]) => [x0.min(fx), y0.min(fy), x1.max(fx + 1.0), y1.max(fy + 1.0)],
            None => [fx, fy, fx + 1.0, fy + 1.0],
        });
    }

    /// Adds the pixels one LED's sampling mode reads (D95.2.3).
    fn add_led(&mut self, led: &geometry::LedSample, mode: SamplingMode) {
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
                let x1 = ((bx1 - 0.5).floor() as i64).clamp(0, max_x);
                let y1 = ((by1 - 0.5).floor() as i64).clamp(0, max_y);
                if x1 < x0 || y1 < y0 {
                    // An empty region averages the nearest pixel, so the mask is that pixel.
                    let (x, y) = nearest();
                    self.set(x, y);
                    return;
                }
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        self.set(x, y);
                    }
                }
            }
        }
    }
}

/// Every device's and zone's mask, at one layout, render scale and sampling mode.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AreaMap {
    /// Bumped whenever a new map is published; the compositor rebuilds its resolved masks only
    /// when it moves.
    pub version: u64,
    pub devices: BTreeMap<DeviceId, Mask>,
    pub zones: BTreeMap<(DeviceId, u32), Mask>,
    /// The layout's render scale, as a factor.
    pub scale: f32,
    pub sampling: SamplingMode,
    /// The render size the masks are built for.
    pub width: u32,
    pub height: u32,
}

impl AreaMap {
    /// Builds the masks for `layout` over `devices`.
    ///
    /// A device's mask is the union, over every LED of every zone it has, of the pixels that
    /// LED's sampling mode reads; a zone's is the union over that zone's LEDs. A device with no
    /// placements, or whose zones are all unplaced, gets no mask: an area over it paints nothing.
    pub fn build(layout: &Layout, devices: &[canvas_proto::DeviceInfo], version: u64) -> Self {
        let scale = layout.render_scale.factor();
        let width = ((CANVAS_WIDTH as f32 * scale).round() as u32).max(1);
        let height = ((CANVAS_HEIGHT as f32 * scale).round() as u32).max(1);
        let plan = geometry::build_plan(layout, devices);
        let mut map = Self {
            version,
            devices: BTreeMap::new(),
            zones: BTreeMap::new(),
            scale,
            sampling: layout.sampling,
            width,
            height,
        };

        for device in &plan.devices {
            let mut mask = Mask::new(width, height);
            for zone in &device.zones {
                let mut zone_mask = Mask::new(width, height);
                for led in &zone.leds {
                    mask.add_led(led, layout.sampling);
                    zone_mask.add_led(led, layout.sampling);
                }
                if let Some(bounds) = placed_bounds(layout, |p| {
                    p.device_id == device.device_id && p.zone_index == zone.zone_index
                }) {
                    zone_mask.dest = bounds.map(|v| v * scale);
                }
                if !zone_mask.is_empty() {
                    map.zones.insert((device.device_id.clone(), zone.zone_index), zone_mask);
                }
            }
            if let Some(bounds) = placed_bounds(layout, |p| p.device_id == device.device_id) {
                mask.dest = bounds.map(|v| v * scale);
            }
            if !mask.is_empty() {
                map.devices.insert(device.device_id.clone(), mask);
            }
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
                mask: None,
                whole_canvas: true,
            }),
            LayerArea::Rect { x, y, width, height } => {
                let dest = rect_px(*x, *y, *width, *height, paint_scale, paint_w, paint_h)?;
                Some(ResolvedArea { dest, mask: None, whole_canvas: false })
            }
            LayerArea::Device { device } => {
                let mask = self.devices.get(device)?;
                Some(ResolvedArea { dest: mask.dest, mask: Some(mask), whole_canvas: false })
            }
            LayerArea::Zone { device, zone } => {
                let mask = self.zones.get(&(device.clone(), *zone))?;
                Some(ResolvedArea { dest: mask.dest, mask: Some(mask), whole_canvas: false })
            }
        }
    }

    /// True when the area covers nothing at all, for the status the API reports.
    pub fn is_empty_area(&self, area: &LayerArea) -> bool {
        self.resolve(area).is_none()
    }
}

/// Where one layer's picture is mapped, and the pixels it may paint.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedArea<'a> {
    /// The bounding box of the area's rectangles, in render pixels.
    pub dest: [f32; 4],
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
    mask: Option<Mask>,
    whole_canvas: bool,
}

impl OwnedArea {
    fn from_resolved(area: ResolvedArea<'_>) -> Self {
        Self { dest: area.dest, mask: area.mask.cloned(), whole_canvas: area.whole_canvas }
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

/// The bounding box, in logical canvas units, of every visible placement of the zones `which`
/// chooses: the four corners after rotation.
fn placed_bounds(layout: &Layout, which: impl Fn(&PlacedZone) -> bool) -> Option<[f32; 4]> {
    let mut out: Option<[f32; 4]> = None;
    for placed in layout.zones.iter().filter(|p| p.visible && which(p)) {
        let center = (placed.x + placed.width / 2.0, placed.y + placed.height / 2.0);
        let corners = [
            (placed.x, placed.y),
            (placed.x + placed.width, placed.y),
            (placed.x + placed.width, placed.y + placed.height),
            (placed.x, placed.y + placed.height),
        ];
        for (x, y) in corners.map(|p| rotate(p, center, placed.rotation_deg)) {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            out = Some(match out {
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
                None => [x, y, x, y],
            });
        }
    }
    out
}

/// Rotates `p` clockwise around `center` by `deg` degrees, the rule the desk draws with.
fn rotate(p: (f32, f32), center: (f32, f32), deg: f32) -> (f32, f32) {
    if deg == 0.0 {
        return p;
    }
    let rad = deg.to_radians();
    let (sin, cos) = rad.sin_cos();
    let dx = p.0 - center.0;
    let dy = p.1 - center.1;
    (center.0 + dx * cos - dy * sin, center.1 + dx * sin + dy * cos)
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
/// Each mode is `d + a·(blend(d, s) − d)`, with Add's blend clamped before the opacity is
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
    /// True while the mask shapes the layer: it built, it has not faulted, and its values apply.
    fn applies(&self) -> bool {
        self.fault.is_none()
    }
}

/// The coverage one mask pixel gives, 0 (hidden) to 1 (whole), before invert and amount.
///
/// Luma is Rec. 709 on the picture's 8-bit-ish channels (`0.2126 R + 0.7152 G + 0.0722 B`); it is
/// linear-enough for a light mask and needs no colour-space conversion. The compositor's canvases
/// are RGB with no alpha channel, so Alpha reads the picture's strongest channel: a pixel the mask
/// effect left black is transparent, and any colour it drew is opaque at its strongest channel's
/// strength.
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
}

impl StackEffect {
    /// Builds every layer of `stack`.
    ///
    /// `leaf` builds one layer's effect and refuses `stack:` ids; a layer whose effect does not
    /// build is kept, skipped, and reported missing.
    pub fn new(stack: &Stack, leaf: LeafBuilder, live: LiveStack, areas: AreaBook) -> Self {
        let layers = stack
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
                    mask,
                    mask_error,
                }
            })
            .collect();
        Self {
            stack_id: stack.id.clone(),
            layers,
            live,
            areas,
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
        }
    }

    /// The clock the hold is measured on.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
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
            layer.hold = Hold::Running;
            layer.last_ms = None;
            layer.panic_logged = false;
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
        if areas.width > 0 {
            areas.scale
        } else {
            1.0
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
            let area = areas.resolve(&values.area).map(OwnedArea::from_resolved);
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
    fn composite_layer(
        &self,
        index: usize,
        area: &OwnedArea,
        values: &LiveLayer,
        target: &mut Canvas,
    ) {
        let layer = &self.layers[index];
        let a = f32::from(values.opacity.min(100)) / 100.0;
        if a <= 0.0 || !values.visible {
            return;
        }
        let dest = if area.whole_canvas {
            [0.0, 0.0, target.width as f32, target.height as f32]
        } else {
            area.dest
        };
        if !dest.iter().all(|v| v.is_finite()) {
            return;
        }
        let [dx0, dy0, dx1, dy1] = dest;
        let (dw, dh) = (dx1 - dx0, dy1 - dy0);
        if dw <= 0.0 || dh <= 0.0 {
            return;
        }
        // A masked area paints every pixel its LEDs read, which for a ring can reach just past
        // the rectangle the picture is mapped over (D95.2.3).
        let paint = match &area.mask {
            Some(mask) => mask.extent().unwrap_or(dest),
            None => dest,
        };
        let x0 = (paint[0].floor().max(0.0) as u32).min(target.width);
        let y0 = (paint[1].floor().max(0.0) as u32).min(target.height);
        let x1 = (paint[2].ceil().max(0.0) as u32).min(target.width);
        let y1 = (paint[3].ceil().max(0.0) as u32).min(target.height);
        let src = &layer.canvas;
        if src.width == 0 || src.height == 0 || x1 <= x0 || y1 <= y0 {
            return;
        }
        // A mask that applies shapes this layer only (#256): its picture is mapped over the same
        // box as the layer's, and its coverage multiplies the layer's alpha per pixel.
        let mask = layer.mask.as_ref().filter(|mask| mask.applies()).filter(|mask| {
            mask.canvas.width == src.width && mask.canvas.height == src.height
        });
        for y in y0..y1 {
            for x in x0..x1 {
                if let Some(area_mask) = &area.mask {
                    if !area_mask.contains(x, y) {
                        continue;
                    }
                }
                // The layer's whole picture is mapped over the area's bounding box.
                let sx = (((x as f32 + 0.5 - dx0) / dw) * src.width as f32).floor();
                let sy = (((y as f32 + 0.5 - dy0) / dh) * src.height as f32).floor();
                let sx = (sx.max(0.0) as u32).min(src.width - 1);
                let sy = (sy.max(0.0) as u32).min(src.height - 1);
                let source = src.get(sx, sy);
                let alpha = match mask {
                    Some(mask) => {
                        let pixel = mask.canvas.get(sx, sy);
                        a * masked_coverage(
                            mask.values.source,
                            pixel,
                            mask.values.invert,
                            mask.values.amount,
                        )
                    }
                    None => a,
                };
                if alpha <= 0.0 {
                    continue;
                }
                let i = (y as usize) * (target.width as usize) + x as usize;
                target.pixels[i] = blend_pixel(values.blend, target.pixels[i], source, alpha);
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

    /// The status of a layer's mask, for a test: its state and why it is not applied.
    pub fn mask_status(&self, id: &str) -> Option<(StackLayerState, Option<String>)> {
        let layer = self.layers.iter().find(|layer| layer.id == id)?;
        if let Some(error) = &layer.mask_error {
            return Some((StackLayerState::MissingEffect, Some(error.clone())));
        }
        layer.mask.as_ref().map(|mask| {
            let state = if mask.fault.is_some() { StackLayerState::Faulted } else { state_of(mask.hold) };
            (state, mask.fault.clone())
        })
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

    /// One frame of the compositor's own work, run inside [`StackEffect::update`]'s guard.
    fn frame(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
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
        if changed {
            self.report_statuses();
        }
    }
}

// ---------------------------------------------------------------- the sample plan for tests

/// The sample plan a build reads, for the area tests: the same plan the engine samples with.
pub fn sample_plan(layout: &Layout, devices: &[canvas_proto::DeviceInfo]) -> geometry::LayoutPlan {
    geometry::build_plan(layout, devices)
}

/// Samples every device's LEDs from `canvas`, as the engine does every frame.
pub fn sample_devices(
    plan: &geometry::LayoutPlan,
    canvas: &Canvas,
    mode: SamplingMode,
) -> Vec<Vec<canvas_proto::Rgb8>> {
    let mut out = Vec::with_capacity(plan.devices.len());
    for device in &plan.devices {
        let mut colors = Vec::new();
        for zone in &device.zones {
            canvas_core::sampling::sample_leds(
                &canvas.pixels,
                canvas.width,
                canvas.height,
                &zone.leds,
                mode,
                &mut colors,
            );
        }
        colors.resize(device.led_count as usize, canvas_proto::Rgb8::BLACK);
        out.push(colors);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
    use canvas_proto::layout::{RenderScale, ZoneShape};
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

    fn placed(device: &str, zone: u32, x: f32, y: f32, w: f32, h: f32) -> PlacedZone {
        PlacedZone {
            device_id: DeviceId(device.into()),
            zone_index: zone,
            x,
            y,
            width: w,
            height: h,
            rotation_deg: 0.0,
            reversed: false,
            visible: true,
            shape: ZoneShape::Auto,
            label: None,
            leds: None,
            part: None,
            locked: false,
        }
    }

    fn layout(zones: Vec<PlacedZone>, scale: RenderScale, sampling: SamplingMode) -> Layout {
        Layout {
            id: "l".into(),
            name: "l".into(),
            render_scale: scale,
            sampling,
            zones,
            effect: None,
            case_form: canvas_proto::layout::CaseForm::None,
            landmarks: Vec::new(),
        }
    }

    #[test]
    fn a_device_mask_covers_its_own_leds_and_no_other_device() {
        let devices = vec![
            device("a", vec![zone(0, ZoneKind::Linear, 4)]),
            device("b", vec![zone(0, ZoneKind::Linear, 4)]),
        ];
        let lay = layout(
            vec![placed("a", 0, 0.0, 0.0, 40.0, 20.0), placed("b", 0, 200.0, 0.0, 40.0, 20.0)],
            RenderScale::P100,
            SamplingMode::AreaAverage,
        );
        let map = AreaMap::build(&lay, &devices, 1);
        let a = map.devices.get(&DeviceId("a".into())).expect("device a has a mask");
        assert!(a.contains(5, 10), "LED 0's cell is lit");
        assert!(!a.contains(205, 10), "device b is outside a's mask");
        assert_eq!((map.width, map.height), (320, 200));
    }

    #[test]
    fn a_rotated_cell_is_covered_by_its_own_bounding_box() {
        let devices = vec![device("a", vec![zone(0, ZoneKind::Single, 1)])];
        let mut p = placed("a", 0, 20.0, 20.0, 40.0, 10.0);
        p.rotation_deg = 30.0;
        let lay = layout(vec![p], RenderScale::P100, SamplingMode::AreaAverage);
        let map = AreaMap::build(&lay, &devices, 1);
        let mask = map.devices.get(&DeviceId("a".into())).unwrap();
        let led = sample_plan(&lay, &devices).devices[0].zones[0].leds[0];
        // Every pixel whose centre falls in the rotated cell's bounding box is covered.
        let [x0, y0, x1, y1] = led.bounds;
        for y in (y0.ceil() as u32)..(y1.floor() as u32) {
            for x in (x0.ceil() as u32)..(x1.floor() as u32) {
                assert!(mask.contains(x, y), "a rotated cell's own pixel {x},{y} is uncovered");
            }
        }
    }

    #[test]
    fn a_rect_with_a_nan_is_no_area_and_a_negative_one_too() {
        let map = AreaMap::build(&layout(vec![], RenderScale::P100, SamplingMode::Nearest), &[], 1);
        assert!(map
            .resolve(&LayerArea::Rect { x: f32::NAN, y: 0.0, width: 10.0, height: 10.0 })
            .is_none());
        assert!(map
            .resolve(&LayerArea::Rect { x: 0.0, y: 0.0, width: -4.0, height: 10.0 })
            .is_none());
        assert!(map.resolve(&LayerArea::Canvas).is_some());
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
