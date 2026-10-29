//! Layout → per-LED sample regions.
//!
//! Everything in here runs at *layout time*, never per frame. The layout gives zones
//! as rectangles in logical canvas units (0..320 x 0..200); the device gives zone
//! topology (linear, single, matrix). This module turns the two into, for every LED
//! of every zone, a sample region in *render pixels* plus the LED's position inside
//! its device's frame buffer.
//!
//! The rules (decided by the spine):
//! - The placed zone's `shape` decides where the LEDs sit; `Auto` follows the device's zone
//!   kind: `Linear` = `Strip`, `Single` = `Point` (every LED samples the whole rect), `Matrix` =
//!   its own map. `Strip`, `Ring`, `Outline`, `Point` and `Grid` override any kind and place
//!   the LEDs in zone order. `reversed` flips the order.
//! - `Strip` with n LEDs: LED i sits at the center of cell i when the rect is split into n
//!   equal cells along its longer axis.
//! - `Matrix { width, height, map }` with `Auto`: the rect is a w×h grid; LED `map[r*w+c]`
//!   sits in cell (c, r). `None` entries are gaps.
//! - Rotation is clockwise around the rect center.
//! - Rotated cells: the sample region for area average is the axis-aligned bounding
//!   box of the rotated cell; nearest and bilinear use the rotated center.
//! - A zone can have several placements. A placement with no `leds` slice owns the whole
//!   zone, as it always did. A placement with a `leds` slice owns only that slice: it lays its
//!   own LEDs along its own shape, and the cells are sized by the slice, not by the zone.
//!   The LEDs of a split zone that no placement owns are black.
//! - A matrix zone is never split: its map owns every LED.

use canvas_proto::device::{DeviceId, DeviceInfo, ZoneInfo, ZoneKind};
use canvas_proto::layout::{Layout, PlacedZone, SamplingMode, ZoneShape};
use tracing::warn;

/// One LED's sample region, in render pixels.
///
/// `center` is where nearest/bilinear sample. `bounds` is the axis-aligned region
/// area-average averages over (already the rotated cell's bounding box when the zone
/// is rotated).
///
/// A LED with no owner is black: `sampled` is false, and it is never sampled. Before
/// parts existed an unclaimed LED still read the canvas's top-left pixel, which is
/// what the adversarial review called out; this flag is what stops it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LedSample {
    /// Sample point in render pixels (float, may be fractional for bilinear).
    pub center_x: f32,
    /// Sample point in render pixels (float, may be fractional for bilinear).
    pub center_y: f32,
    /// Area-average region, in render pixels, as `[x0, y0, x1, y1]`.
    pub bounds: [f32; 4],
    /// False when the LED has no owner: it is black and is never sampled.
    pub sampled: bool,
}

impl LedSample {
    /// The LED's sample region, or `None` when it has no owner and is black.
    pub fn region(&self) -> Option<(f32, f32, [f32; 4])> {
        if self.sampled {
            Some((self.center_x, self.center_y, self.bounds))
        } else {
            None
        }
    }
}

impl Default for LedSample {
    /// A LED with no owner: black, and never sampled.
    fn default() -> Self {
        LedSample { center_x: 0.0, center_y: 0.0, bounds: [0.0; 4], sampled: false }
    }
}

/// One zone's LEDs, in zone order.
#[derive(Debug, Clone)]
pub struct ZoneSamples {
    /// The zone's index inside its device.
    pub zone_index: u32,
    /// True when the layout places this zone visibly: at least one of its placements
    /// is visible. An unplaced or invisible zone is never sampled: its LEDs are black.
    pub placed: bool,
    /// One entry per LED of the zone, in zone order. A LED no placement owns is black:
    /// its sample is not sampled.
    pub leds: Vec<LedSample>,
    /// The slices of LEDs the zone's placements own, one per split placement. A zone
    /// that is not split has none.
    pub slices: Vec<SliceSamples>,
}

/// One placement's slice of a zone's LEDs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliceSamples {
    /// The first LED of the zone the slice starts at.
    pub start: u32,
    /// The number of LEDs the slice claims.
    pub count: u32,
    /// The number of those LEDs the zone still has. Less than `count` means the zone
    /// shrank under the slice.
    pub available: u32,
    /// True when the slice is honoured: its placement is visible and nothing else claims
    /// its LEDs.
    pub placed: bool,
    /// True when the zone has fewer LEDs than the slice claims: the slice has to be mapped
    /// again. The slice is never clipped silently.
    pub needs_mapping: bool,
}

/// A device's zones, in device order, with the LED count of each.
#[derive(Debug, Clone)]
pub struct DeviceSamples {
    /// The device these zones belong to.
    pub device_id: DeviceId,
    /// Zones in the device's own zone order (index order).
    pub zones: Vec<ZoneSamples>,
    /// Total LEDs; a frame for this device has exactly this many colors.
    pub led_count: u32,
}

/// The whole layout, precomputed.
#[derive(Debug, Clone, Default)]
pub struct LayoutPlan {
    pub devices: Vec<DeviceSamples>,
    /// Render scale of the layout, as a factor (1.0, 0.5, 0.25).
    pub scale: f32,
    pub sampling: SamplingMode,
}

impl LayoutPlan {
    /// Number of devices in the plan.
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// A short summary of the plan, for the log: one line per device.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for device in &self.devices {
            if !out.is_empty() {
                out.push_str(", ");
            }
            let zones: Vec<String> = device
                .zones
                .iter()
                .map(|z| {
                    let state = if z.placed { "" } else { " (unplaced)" };
                    let slices = if z.slices.is_empty() {
                        String::new()
                    } else {
                        format!(" ({} slices)", z.slices.len())
                    };
                    format!("z{}:{}{}{}", z.zone_index, z.leds.len(), state, slices)
                })
                .collect();
            out.push_str(&format!("{} [{}]", device.device_id, zones.join(" ")));
        }
        out
    }
}

/// Builds the sample plan for `layout` over `devices`.
///
/// Devices in `devices` that the layout does not mention still get an entry, with
/// every LED black (unplaced zones are never sampled). Devices in the layout that
/// `devices` does not know are ignored.
pub fn build_plan(layout: &Layout, devices: &[DeviceInfo]) -> LayoutPlan {
    let scale = layout.render_scale.factor();
    let mut out = Vec::with_capacity(devices.len());

    for device in devices {
        let mut zone_samples = Vec::with_capacity(device.zones.len());
        for zone in &device.zones {
            // Every placement of the zone, not just the first: one zone can drive
            // several parts (three daisy-chained fans on one ARGB header).
            let placements: Vec<&PlacedZone> = layout
                .zones
                .iter()
                .filter(|p| p.device_id == device.id && p.zone_index == zone.index)
                .collect();
            zone_samples.push(build_zone_samples(&device.id, &placements, zone, scale));
        }
        let led_count = device.led_count();
        out.push(DeviceSamples {
            device_id: device.id.clone(),
            zones: zone_samples,
            led_count,
        });
    }

    LayoutPlan { devices: out, scale, sampling: layout.sampling }
}

/// Builds the samples of one zone from every placement of it.
///
/// A placement with no `leds` slice owns the whole zone, as it always did, and the
/// first such placement wins. A zone whose placements all carry a slice is split: every
/// placement owns its own slice, lays it out along its own shape (the cells are sized
/// by the slice, not by the zone), and the LEDs no placement owns are black.
fn build_zone_samples(
    device_id: &DeviceId,
    placements: &[&PlacedZone],
    zone: &ZoneInfo,
    scale: f32,
) -> ZoneSamples {
    let led_count = zone.led_count as usize;
    let is_matrix = matches!(zone.kind, ZoneKind::Matrix { .. });

    // A matrix zone is never split: its map owns every LED. A slice on one is ignored.
    for p in placements {
        if p.leds.is_some() && is_matrix {
            warn!(
                device = %device_id,
                zone = zone.index,
                "a matrix zone is never split; the slice of LEDs on it is ignored"
            );
        }
    }

    // The placement that owns the whole zone, if there is one.
    let whole = placements.iter().copied().find(|p| p.leds.is_none() || is_matrix);
    // The placements that own a slice of the zone's LEDs.
    let split: Vec<&PlacedZone> = placements
        .iter()
        .copied()
        .filter(|p| p.leds.is_some() && !is_matrix)
        .collect();

    let mut slices = Vec::new();
    let mut placed = false;

    if let Some(p) = whole {
        // The zone is not split: the placement owns every LED.
        let leds = if p.visible {
            placed = true;
            build_zone_leds(p, zone, scale)
        } else {
            // An invisible placement: the zone is black.
            vec![LedSample::default(); led_count]
        };
        // A slice beside a whole-zone placement claims LEDs that are already taken.
        for r in &split {
            let s = r.leds.expect("a slice");
            warn!(
                device = %device_id,
                zone = zone.index,
                start = s.start,
                count = s.count,
                "one placement owns the whole zone and another owns a slice of it; the slice is ignored"
            );
            slices.push(SliceSamples {
                start: s.start,
                count: s.count,
                available: 0,
                placed: false,
                needs_mapping: false,
            });
        }
        return ZoneSamples { zone_index: zone.index, placed, leds, slices };
    }

    // The zone is split: every placement owns a slice of its LEDs. The LEDs no
    // placement owns are black.
    let mut leds = vec![LedSample::default(); led_count];
    for p in split {
        let s = p.leds.expect("a slice");
        let start = s.start as usize;
        let count = s.count as usize;
        // The LEDs the zone still has: the zone may have shrunk under the slice.
        let available = count.min(led_count.saturating_sub(start));
        if p.visible && available < count {
            warn!(
                device = %device_id,
                zone = zone.index,
                start = s.start,
                count = s.count,
                available,
                "the zone has fewer LEDs than this slice needs; the slice needs mapping again"
            );
        }
        let honoured = p.visible && available > 0;
        if honoured {
            placed = true;
            // The slice lays its own LEDs along its own shape: the cells are sized by
            // the slice, not by the zone. The shape's slot `j` is the zone's LED
            // `start + j`, and `reversed` flips the slice inside its own shape.
            let shape = resolve_shape(p.shape, &zone.kind);
            let run = build_run_leds(p, shape, count, available, scale);
            for (j, sample) in run.iter().enumerate() {
                let led = start + j;
                if leds[led].region().is_some() {
                    warn!(
                        device = %device_id,
                        zone = zone.index,
                        led,
                        "two placements claim the same LED; the first one wins"
                    );
                    continue;
                }
                leds[led] = *sample;
            }
        }
        slices.push(SliceSamples {
            start: s.start,
            count: s.count,
            available: available as u32,
            placed: honoured,
            needs_mapping: available < count,
        });
    }

    ZoneSamples { zone_index: zone.index, placed, leds, slices }
}

/// The sample of a run of LEDs along its shape.
///
/// The run has `count` LEDs and the shape is sized by that count: a run of 12 on a
/// fan gets a ring of 12, not a ring of the whole zone. Only the first `take` of
/// them are laid out, which is what a zone that shrank under the run leaves.
///
/// The shape is decided once, here, and not once per LED: each arm below knows the
/// shape it draws, so the loop inside it is straight-line code.
fn build_run_leds(
    placed: &PlacedZone,
    shape: ZoneShape,
    count: usize,
    take: usize,
    scale: f32,
) -> Vec<LedSample> {
    let mut leds = Vec::with_capacity(take);
    match shape {
        ZoneShape::Auto | ZoneShape::Strip => {
            for j in 0..take {
                let cell = shape_cell(placed, ZoneShape::Strip, slot_of(placed, count, j), count);
                leds.push(sample_from_corners(&cell, scale));
            }
        }
        ZoneShape::Point => {
            for j in 0..take {
                let cell = shape_cell(placed, ZoneShape::Point, slot_of(placed, count, j), count);
                leds.push(sample_from_corners(&cell, scale));
            }
        }
        ZoneShape::Ring => {
            for j in 0..take {
                let cell = shape_cell(placed, ZoneShape::Ring, slot_of(placed, count, j), count);
                leds.push(sample_from_corners(&cell, scale));
            }
        }
        ZoneShape::Outline => {
            for j in 0..take {
                let cell = shape_cell(placed, ZoneShape::Outline, slot_of(placed, count, j), count);
                leds.push(sample_from_corners(&cell, scale));
            }
        }
        ZoneShape::Grid { columns } => {
            for j in 0..take {
                let cell =
                    shape_cell(placed, ZoneShape::Grid { columns }, slot_of(placed, count, j), count);
                leds.push(sample_from_corners(&cell, scale));
            }
        }
    }
    leds
}

/// The slot of the shape the run's LED `j` sits at. `reversed` flips which end of
/// the shape the run's first LED sits at.
fn slot_of(placed: &PlacedZone, count: usize, j: usize) -> usize {
    if placed.reversed { count - 1 - j } else { j }
}

/// The shape a placement's LEDs sit in. `Auto` follows the device's zone kind.
fn resolve_shape(shape: ZoneShape, kind: &ZoneKind) -> ZoneShape {
    match (shape, kind) {
        (ZoneShape::Auto, ZoneKind::Single) => ZoneShape::Point,
        (ZoneShape::Auto, ZoneKind::Linear) => ZoneShape::Strip,
        (other, _) => other,
    }
}

/// The four corners of one cell, in logical canvas units, after rotation.
#[derive(Debug, Clone, Copy)]
struct CellCorners {
    /// Corner points, clockwise from top-left.
    corners: [(f32, f32); 4],
}

impl CellCorners {
    /// Axis-aligned bounding box of the corners.
    fn bounds(&self) -> [f32; 4] {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for &(x, y) in &self.corners {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        [min_x, min_y, max_x, max_y]
    }

    /// Center of the cell (the mean of the corners, which for a rotated rectangle is
    /// the rotation center of the original cell).
    fn center(&self) -> (f32, f32) {
        let mut sx = 0.0;
        let mut sy = 0.0;
        for &(x, y) in &self.corners {
            sx += x;
            sy += y;
        }
        (sx / 4.0, sy / 4.0)
    }
}

/// Rotates `p` clockwise around `center` by `deg` degrees.
///
/// Screen coordinates grow downward, so with the usual rotation matrix a positive angle
/// turns clockwise on screen.
fn rotate_point(p: (f32, f32), center: (f32, f32), deg: f32) -> (f32, f32) {
    if deg == 0.0 {
        return p;
    }
    let rad = deg.to_radians();
    let (sin, cos) = rad.sin_cos();
    let dx = p.0 - center.0;
    let dy = p.1 - center.1;
    // Clockwise on screen (y down), as CSS rotate() draws it and as the editor mirrors it:
    // x' = x*cos - y*sin, y' = x*sin + y*cos. (The inverse turned the sampled LEDs the
    // opposite way from the zone the editor draws.)
    (center.0 + dx * cos - dy * sin, center.1 + dx * sin + dy * cos)
}

/// Computes the sample region of every LED of `zone` when placed at `placed`.
///
/// `scale` converts logical canvas units to render pixels.
///
/// The placed zone's `shape` decides where the LEDs sit. `Auto` follows the device's zone
/// kind (single = one point, linear = a strip, matrix = its grid map); every other shape
/// overrides it and places the LEDs in zone order, honouring `reversed`.
fn build_zone_leds(placed: &PlacedZone, zone: &ZoneInfo, scale: f32) -> Vec<LedSample> {
    let led_count = zone.led_count as usize;
    if led_count == 0 {
        return Vec::new();
    }

    if let ZoneShape::Auto = placed.shape {
        if let ZoneKind::Matrix { width, height, map } = &zone.kind {
            // A matrix zone's map leaves the LEDs it does not place without a cell:
            // they belong to nobody, so they are black and never sampled. (They used
            // to read the canvas's top-left pixel, which showed the effect's corner on
            // a key the map does not have.)
            let mut leds = vec![LedSample::default(); led_count];
            fill_matrix(&mut leds, placed, *width as usize, *height as usize, map, scale);
            return leds;
        }
    }
    let shape = resolve_shape(placed.shape, &zone.kind);
    build_run_leds(placed, shape, led_count, led_count, scale)
}

/// Fills `leds` from a matrix zone's own map (the `Auto` shape of a matrix zone).
fn fill_matrix(
    leds: &mut [LedSample],
    placed: &PlacedZone,
    w: usize,
    h: usize,
    map: &[Option<u32>],
    scale: f32,
) {
    if w == 0 || h == 0 {
        return;
    }
    for row in 0..h {
        for col in 0..w {
            let slot = row * w + col;
            let Some(Some(led)) = map.get(slot).copied() else {
                continue;
            };
            let Some(led) = usize::try_from(led).ok() else {
                continue;
            };
            if led >= leds.len() {
                continue;
            }
            let cell = matrix_cell(placed, col, row, w, h);
            leds[led] = sample_from_corners(&cell, scale);
        }
    }
}

/// An axis-aligned cell centred on `(cx, cy)`, rotated with the zone.
fn centred_cell(placed: &PlacedZone, cx: f32, cy: f32, cw: f32, ch: f32) -> CellCorners {
    let center = (placed.x + placed.width / 2.0, placed.y + placed.height / 2.0);
    let (x0, y0, x1, y1) = (cx - cw / 2.0, cy - ch / 2.0, cx + cw / 2.0, cy + ch / 2.0);
    let raw = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    CellCorners { corners: raw.map(|p| rotate_point(p, center, placed.rotation_deg)) }
}

/// The sample cell of the LED at `slot` (0-based position along the shape) of `n` LEDs.
fn shape_cell(placed: &PlacedZone, shape: ZoneShape, slot: usize, n: usize) -> CellCorners {
    let (x, y, w, h) = (placed.x, placed.y, placed.width, placed.height);
    let (mx, my) = (x + w / 2.0, y + h / 2.0);
    let nf = n as f32;
    let i = slot as f32;
    match shape {
        // `Auto` is resolved by the caller; treat a stray one as a strip.
        ZoneShape::Auto | ZoneShape::Strip => linear_cell(placed, i, nf, w >= h),
        ZoneShape::Point => centred_cell(placed, mx, my, w, h),
        ZoneShape::Ring => {
            let (a, b) = (w / 2.0, h / 2.0);
            // Ramanujan's approximation of the ellipse perimeter.
            let perimeter = std::f32::consts::PI
                * (3.0 * (a + b) - ((3.0 * a + b) * (a + 3.0 * b)).sqrt());
            let size = (perimeter / nf).min(w.min(h));
            let theta = std::f32::consts::TAU * i / nf;
            // Clockwise from the top, y down.
            centred_cell(placed, mx + a * theta.sin(), my - b * theta.cos(), size, size)
        }
        ZoneShape::Outline => {
            let perimeter = 2.0 * (w + h);
            let size = (perimeter / nf).min(w.min(h));
            let d = perimeter * i / nf;
            let (px, py) = if d < w {
                (x + d, y)
            } else if d < w + h {
                (x + w, y + (d - w))
            } else if d < 2.0 * w + h {
                (x + w - (d - w - h), y + h)
            } else {
                (x, y + h - (d - 2.0 * w - h))
            };
            centred_cell(placed, px, py, size, size)
        }
        ZoneShape::Grid { columns } => {
            let cols = (columns.max(1) as usize).min(n);
            let rows = n.div_ceil(cols);
            let (cw, ch) = (w / cols as f32, h / rows as f32);
            let (col, row) = ((slot % cols) as f32, (slot / cols) as f32);
            centred_cell(placed, x + (col + 0.5) * cw, y + (row + 0.5) * ch, cw, ch)
        }
    }
}

/// The `index`-th cell of a linear zone, split along `horizontal`.
fn linear_cell(placed: &PlacedZone, index: f32, n: f32, horizontal: bool) -> CellCorners {
    let center = (placed.x + placed.width / 2.0, placed.y + placed.height / 2.0);
    let (x0, y0, x1, y1) = if horizontal {
        let cell_w = placed.width / n;
        let sx0 = placed.x + index * cell_w;
        (sx0, placed.y, sx0 + cell_w, placed.y + placed.height)
    } else {
        let cell_h = placed.height / n;
        let sy0 = placed.y + index * cell_h;
        (placed.x, sy0, placed.x + placed.width, sy0 + cell_h)
    };
    let raw = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    CellCorners { corners: raw.map(|p| rotate_point(p, center, placed.rotation_deg)) }
}

/// The `(col, row)` cell of a matrix zone.
fn matrix_cell(placed: &PlacedZone, col: usize, row: usize, w: usize, h: usize) -> CellCorners {
    let center = (placed.x + placed.width / 2.0, placed.y + placed.height / 2.0);
    let cw = placed.width / w as f32;
    let ch = placed.height / h as f32;
    let x0 = placed.x + col as f32 * cw;
    let y0 = placed.y + row as f32 * ch;
    let x1 = x0 + cw;
    let y1 = y0 + ch;
    let raw = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    CellCorners { corners: raw.map(|p| rotate_point(p, center, placed.rotation_deg)) }
}

/// Turns a cell's corners into a sample region in render pixels.
fn sample_from_corners(cell: &CellCorners, scale: f32) -> LedSample {
    let (cx, cy) = cell.center();
    let b = cell.bounds();
    LedSample {
        center_x: cx * scale,
        center_y: cy * scale,
        bounds: [b[0] * scale, b[1] * scale, b[2] * scale, b[3] * scale],
        sampled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::DeviceKind;
    use canvas_proto::layout::LedSlice;

    /// The sample point of a LED, for the tests. A LED with no owner reads (0, 0).
    fn cx(led: &LedSample) -> f32 {
        led.center_x
    }

    /// The sample point of a LED, for the tests. A LED with no owner reads (0, 0).
    fn cy(led: &LedSample) -> f32 {
        led.center_y
    }

    /// The area-average region of a LED, for the tests. A LED with no owner has none.
    fn bounds(led: &LedSample) -> [f32; 4] {
        led.bounds
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
            id: DeviceId(id.to_string()),
            name: id.to_string(),
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

    fn placed(device_id: &str, zone_index: u32, x: f32, y: f32, w: f32, h: f32) -> PlacedZone {
        PlacedZone {
            device_id: DeviceId(device_id.to_string()),
            zone_index,
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

    fn layout(zones: Vec<PlacedZone>) -> Layout {
        Layout {
            id: "l".into(),
            name: "l".into(),
            render_scale: canvas_proto::layout::RenderScale::P100,
            sampling: SamplingMode::Nearest,
            zones,
            effect: None,
            case_form: canvas_proto::layout::CaseForm::None,
            landmarks: Vec::new(),
        }
    }

    #[test]
    fn linear_zone_splits_along_the_long_axis() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 4)]);
        let lay = layout(vec![placed("d0", 0, 0.0, 0.0, 100.0, 20.0)]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        assert_eq!(leds.len(), 4);
        // Four cells of 25 units each across the 100-unit-wide rect.
        assert!((cx(&leds[0]) - 12.5).abs() < 0.01);
        assert!((cx(&leds[1]) - 37.5).abs() < 0.01);
        assert!((cx(&leds[2]) - 62.5).abs() < 0.01);
        assert!((cx(&leds[3]) - 87.5).abs() < 0.01);
        assert!((cy(&leds[0]) - 10.0).abs() < 0.01);
    }

    #[test]
    fn tall_linear_zone_splits_vertically() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 3)]);
        let lay = layout(vec![placed("d0", 0, 10.0, 0.0, 20.0, 90.0)]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        assert!((cy(&leds[0]) - 15.0).abs() < 0.01);
        assert!((cy(&leds[1]) - 45.0).abs() < 0.01);
        assert!((cy(&leds[2]) - 75.0).abs() < 0.01);
    }

    #[test]
    fn reversed_linear_zone_flips_the_order() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 3)]);
        let mut p = placed("d0", 0, 0.0, 0.0, 90.0, 10.0);
        p.reversed = true;
        let lay = layout(vec![p]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        assert!((cx(&leds[0]) - 75.0).abs() < 0.01);
        assert!((cx(&leds[2]) - 15.0).abs() < 0.01);
    }

    #[test]
    fn matrix_zone_maps_cells_to_led_indices() {
        let map = vec![Some(0), Some(1), Some(2), Some(3)];
        let dev = device("d0", vec![zone(0, ZoneKind::Matrix { width: 2, height: 2, map }, 4)]);
        let lay = layout(vec![placed("d0", 0, 0.0, 0.0, 100.0, 100.0)]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // LED 0 is cell (0,0): top-left.
        assert!((cx(&leds[0]) - 25.0).abs() < 0.01);
        assert!((cy(&leds[0]) - 25.0).abs() < 0.01);
        // LED 1 is cell (1,0): top-right.
        assert!((cx(&leds[1]) - 75.0).abs() < 0.01);
        assert!((cy(&leds[1]) - 25.0).abs() < 0.01);
        // LED 2 is cell (0,1): bottom-left.
        assert!((cx(&leds[2]) - 25.0).abs() < 0.01);
        assert!((cy(&leds[2]) - 75.0).abs() < 0.01);
        // LED 3 is cell (1,1): bottom-right.
        assert!((cx(&leds[3]) - 75.0).abs() < 0.01);
        assert!((cy(&leds[3]) - 75.0).abs() < 0.01);
    }

    #[test]
    fn matrix_gaps_leave_their_led_black() {
        // map[2] is a gap, so LED 2 has no cell.
        let map = vec![Some(0), Some(1), None, Some(3)];
        let dev = device("d0", vec![zone(0, ZoneKind::Matrix { width: 2, height: 2, map }, 4)]);
        let lay = layout(vec![placed("d0", 0, 0.0, 0.0, 100.0, 100.0)]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // The LED the map does not place has no sample at all.
        assert_eq!(leds[2].region(), None, "the gap LED has no owner");
        // LED 3 still lands in cell (1,1).
        assert!((cx(&leds[3]) - 75.0).abs() < 0.01);

        // And the colour it gets is black, not the effect's top-left pixel: the
        // canvas here is red everywhere, so a LED that read a pixel would be red.
        let canvas = vec![[1.0f32, 0.0, 0.0]; 16];
        let mut colors = Vec::new();
        crate::sampling::sample_leds(&canvas, 4, 4, leds, SamplingMode::Nearest, &mut colors);
        assert_eq!(colors.len(), 4);
        assert_eq!(colors[2], canvas_proto::color::Rgb8::BLACK, "the gap LED is black");
        assert_eq!(colors[0], canvas_proto::color::Rgb8::new(255, 0, 0), "LED 0 is red");
        assert_eq!(colors[3], canvas_proto::color::Rgb8::new(255, 0, 0), "LED 3 is red");
    }

    #[test]
    fn rotation_moves_the_center_and_grows_the_bounds() {
        let dev = device("d0", vec![zone(0, ZoneKind::Single, 1)]);
        let mut p = placed("d0", 0, 100.0, 100.0, 40.0, 20.0);
        p.rotation_deg = 90.0;
        let lay = layout(vec![p]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let led = plan.devices[0].zones[0].leds[0];
        // The center of a rect rotated around its own center does not move.
        assert!((cx(&led) - 120.0).abs() < 0.01);
        assert!((cy(&led) - 110.0).abs() < 0.01);
        // But the axis-aligned bounding box swaps width and height.
        assert!((bounds(&led)[2] - bounds(&led)[0] - 20.0).abs() < 0.01);
        assert!((bounds(&led)[3] - bounds(&led)[1] - 40.0).abs() < 0.01);
    }

    #[test]
    fn render_scale_multiplies_the_sample_coordinates() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 2)]);
        let mut lay = layout(vec![placed("d0", 0, 0.0, 0.0, 320.0, 200.0)]);
        lay.render_scale = canvas_proto::layout::RenderScale::P50;
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // Logical 80 -> render 40 at 50%.
        assert!((cx(&leds[0]) - 40.0).abs() < 0.01);
        assert!((cy(&leds[0]) - 50.0).abs() < 0.01);
        assert_eq!(plan.scale, 0.5);
    }

    #[test]
    fn every_render_scale_maps_the_same_zone_onto_the_right_pixels() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 2)]);
        // A zone covering the left half of the canvas. It is wider than it is tall,
        // so the linear zone splits along x.
        let base = placed("d0", 0, 0.0, 0.0, 160.0, 100.0);

        // 100%: the cell covers render pixels 0..80.
        let mut lay = layout(vec![base.clone()]);
        lay.render_scale = canvas_proto::layout::RenderScale::P100;
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        assert_eq!(plan.scale, 1.0);
        let leds = &plan.devices[0].zones[0].leds;
        assert!((cx(&leds[0]) - 40.0).abs() < 0.01);
        assert!((bounds(&leds[0])[0] - 0.0).abs() < 0.01);
        assert!((bounds(&leds[0])[2] - 80.0).abs() < 0.01);

        // 50%: the same zone covers render pixels 0..40.
        let mut lay = layout(vec![base.clone()]);
        lay.render_scale = canvas_proto::layout::RenderScale::P50;
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        assert_eq!(plan.scale, 0.5);
        let leds = &plan.devices[0].zones[0].leds;
        assert!((cx(&leds[0]) - 20.0).abs() < 0.01);
        assert!((bounds(&leds[0])[0] - 0.0).abs() < 0.01);
        assert!((bounds(&leds[0])[2] - 40.0).abs() < 0.01);

        // 25%: the same zone covers render pixels 0..20.
        let mut lay = layout(vec![base]);
        lay.render_scale = canvas_proto::layout::RenderScale::P25;
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        assert_eq!(plan.scale, 0.25);
        let leds = &plan.devices[0].zones[0].leds;
        assert!((cx(&leds[0]) - 10.0).abs() < 0.01);
        assert!((bounds(&leds[0])[0] - 0.0).abs() < 0.01);
        assert!((bounds(&leds[0])[2] - 20.0).abs() < 0.01);
    }

    #[test]
    fn the_render_scale_reaches_the_canvas_size() {
        // The canvas is the logical size times the scale factor.
        let full = canvas_proto::traits::Canvas::with_scale(1.0);
        assert_eq!((full.width, full.height), (320, 200));
        let half = canvas_proto::traits::Canvas::with_scale(0.5);
        assert_eq!((half.width, half.height), (160, 100));
        let quarter = canvas_proto::traits::Canvas::with_scale(0.25);
        assert_eq!((quarter.width, quarter.height), (80, 50));
    }

    #[test]
    fn unplaced_and_invisible_zones_get_black() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 2), zone(1, ZoneKind::Linear, 3)]);
        // Zone 1 is not placed at all; zone 0 is placed but invisible.
        let mut p = placed("d0", 0, 0.0, 0.0, 100.0, 10.0);
        p.visible = false;
        let lay = layout(vec![p]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let z0 = &plan.devices[0].zones[0].leds;
        let z1 = &plan.devices[0].zones[1].leds;
        assert!(z0.iter().all(|l| cx(l) == 0.0 && cy(l) == 0.0));
        assert!(z1.iter().all(|l| cx(l) == 0.0 && cy(l) == 0.0));
        assert_eq!(plan.devices[0].led_count, 5);
    }

    #[test]
    fn device_led_count_is_the_sum_over_zones() {
        let dev = device(
            "d0",
            vec![zone(0, ZoneKind::Linear, 2), zone(1, ZoneKind::Linear, 3)],
        );
        let lay = layout(vec![]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        assert_eq!(plan.devices[0].led_count, 5);
    }

    fn shaped(
        shape: ZoneShape,
        kind: ZoneKind,
        n: u32,
        rect: (f32, f32, f32, f32),
    ) -> Vec<LedSample> {
        let dev = device("d0", vec![zone(0, kind, n)]);
        let mut p = placed("d0", 0, rect.0, rect.1, rect.2, rect.3);
        p.shape = shape;
        let plan = build_plan(&layout(vec![p]), std::slice::from_ref(&dev));
        plan.devices[0].zones[0].leds.clone()
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn single_zone_with_many_leds_samples_the_whole_rect_not_pixel_zero() {
        let leds = shaped(ZoneShape::Auto, ZoneKind::Single, 3, (100.0, 50.0, 20.0, 10.0));
        assert_eq!(leds.len(), 3);
        for led in &leds {
            assert!(near(cx(led), 110.0) && near(cy(led), 55.0));
            assert_eq!(bounds(led), [100.0, 50.0, 120.0, 60.0]);
        }
    }

    #[test]
    fn point_shape_makes_every_led_sample_the_whole_rect() {
        let leds = shaped(ZoneShape::Point, ZoneKind::Linear, 5, (10.0, 20.0, 40.0, 30.0));
        for led in &leds {
            assert!(near(cx(led), 30.0) && near(cy(led), 35.0));
            assert_eq!(bounds(led), [10.0, 20.0, 50.0, 50.0]);
        }
    }

    #[test]
    fn strip_shape_forces_a_strip_on_a_single_zone() {
        let leds = shaped(ZoneShape::Strip, ZoneKind::Single, 4, (0.0, 0.0, 100.0, 20.0));
        assert!(near(cx(&leds[0]), 12.5) && near(cx(&leds[3]), 87.5));
        assert!(near(cy(&leds[0]), 10.0));
    }

    #[test]
    fn auto_linear_equals_strip() {
        let a = shaped(ZoneShape::Auto, ZoneKind::Linear, 6, (5.0, 5.0, 16.0, 120.0));
        let b = shaped(ZoneShape::Strip, ZoneKind::Linear, 6, (5.0, 5.0, 16.0, 120.0));
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x, y);
        }
        // A tall stick splits along y.
        assert!(near(cy(&a[0]), 15.0) && near(cy(&a[5]), 115.0));
    }

    #[test]
    fn ring_places_leds_clockwise_from_the_top() {
        // Square 100x100 at the origin, 4 LEDs: top, right, bottom, left.
        let leds = shaped(ZoneShape::Ring, ZoneKind::Linear, 4, (0.0, 0.0, 100.0, 100.0));
        assert!(near(cx(&leds[0]), 50.0) && near(cy(&leds[0]), 0.0));
        assert!(near(cx(&leds[1]), 100.0) && near(cy(&leds[1]), 50.0));
        assert!(near(cx(&leds[2]), 50.0) && near(cy(&leds[2]), 100.0));
        assert!(near(cx(&leds[3]), 0.0) && near(cy(&leds[3]), 50.0));
    }

    #[test]
    fn ring_follows_an_ellipse_when_the_rect_is_not_square() {
        let leds = shaped(ZoneShape::Ring, ZoneKind::Linear, 4, (0.0, 0.0, 200.0, 100.0));
        assert!(near(cx(&leds[1]), 200.0) && near(cy(&leds[1]), 50.0));
        assert!(near(cx(&leds[0]), 100.0) && near(cy(&leds[0]), 0.0));
    }

    #[test]
    fn ring_reversed_runs_counter_clockwise() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 4)]);
        let mut p = placed("d0", 0, 0.0, 0.0, 100.0, 100.0);
        p.shape = ZoneShape::Ring;
        p.reversed = true;
        let plan = build_plan(&layout(vec![p]), std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // LED 0 now sits where the last slot (left) is, LED 1 at the bottom.
        assert!(near(cx(&leds[0]), 0.0) && near(cy(&leds[0]), 50.0));
        assert!(near(cx(&leds[1]), 50.0) && near(cy(&leds[1]), 100.0));
    }

    #[test]
    fn ring_rotation_moves_leds_around_the_centre() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 4)]);
        let mut p = placed("d0", 0, 0.0, 0.0, 100.0, 100.0);
        p.shape = ZoneShape::Ring;
        p.rotation_deg = 90.0;
        let plan = build_plan(&layout(vec![p]), std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // The top LED turns clockwise a quarter: it ends up on the right.
        assert!(near(cx(&leds[0]), 100.0) && near(cy(&leds[0]), 50.0));
    }

    #[test]
    fn outline_walks_the_rect_edge_clockwise_from_top_left() {
        // 100 x 50 rect, 6 LEDs: perimeter 300, spacing 50.
        let leds = shaped(ZoneShape::Outline, ZoneKind::Linear, 6, (0.0, 0.0, 100.0, 50.0));
        let want = [
            (0.0, 0.0),
            (50.0, 0.0),
            (100.0, 0.0),
            (100.0, 50.0),
            (50.0, 50.0),
            (0.0, 50.0),
        ];
        for (led, (x, y)) in leds.iter().zip(want) {
            assert!(near(cx(led), x) && near(cy(led), y), "{led:?} vs {x},{y}");
        }
    }

    #[test]
    fn outline_reversed_starts_at_the_other_end() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 4)]);
        let mut p = placed("d0", 0, 0.0, 0.0, 100.0, 100.0);
        p.shape = ZoneShape::Outline;
        p.reversed = true;
        let plan = build_plan(&layout(vec![p]), std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // Slots: (0,0) (100,0) (100,100) (0,100); reversed LED 0 takes the last.
        assert!(near(cx(&leds[0]), 0.0) && near(cy(&leds[0]), 100.0));
        assert!(near(cx(&leds[3]), 0.0) && near(cy(&leds[3]), 0.0));
    }

    #[test]
    fn grid_fills_row_by_row_top_left_first() {
        // 6 LEDs, 3 columns: 2 rows in a 90 x 60 rect.
        let leds = shaped(
            ZoneShape::Grid { columns: 3 },
            ZoneKind::Linear,
            6,
            (0.0, 0.0, 90.0, 60.0),
        );
        let want = [
            (15.0, 15.0),
            (45.0, 15.0),
            (75.0, 15.0),
            (15.0, 45.0),
            (45.0, 45.0),
            (75.0, 45.0),
        ];
        for (led, (x, y)) in leds.iter().zip(want) {
            assert!(near(cx(led), x) && near(cy(led), y));
        }
        assert_eq!(bounds(&leds[0]), [0.0, 0.0, 30.0, 30.0]);
    }

    #[test]
    fn grid_with_a_ragged_last_row_and_zero_columns_stays_in_bounds() {
        let leds =
            shaped(ZoneShape::Grid { columns: 4 }, ZoneKind::Linear, 5, (0.0, 0.0, 80.0, 40.0));
        assert_eq!(leds.len(), 5);
        assert!(near(cx(&leds[4]), 10.0) && near(cy(&leds[4]), 30.0));
        let one =
            shaped(ZoneShape::Grid { columns: 0 }, ZoneKind::Linear, 3, (0.0, 0.0, 10.0, 30.0));
        assert!(near(cy(&one[2]), 25.0) && near(cx(&one[2]), 5.0));
    }

    #[test]
    fn matrix_zone_with_a_shape_overrides_its_map() {
        let map = vec![Some(0), None, None, Some(1)];
        let kind = ZoneKind::Matrix { width: 2, height: 2, map };
        let auto = shaped(ZoneShape::Auto, kind.clone(), 2, (0.0, 0.0, 100.0, 100.0));
        assert!(near(cx(&auto[1]), 75.0) && near(cy(&auto[1]), 75.0));
        let strip = shaped(ZoneShape::Strip, kind, 2, (0.0, 0.0, 100.0, 100.0));
        // Width >= height, so the strip runs along x: LED 1 is the right half.
        assert!(near(cx(&strip[1]), 75.0) && near(cy(&strip[1]), 50.0));
    }

    #[test]
    fn rotated_strip_keeps_rotation() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 2)]);
        let mut p = placed("d0", 0, 0.0, 0.0, 100.0, 20.0);
        p.rotation_deg = 90.0;
        let plan = build_plan(&layout(vec![p]), std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // Centre (50,10); LED 0 at (25,10) rotates clockwise a quarter to (50,-15).
        assert!(near(cx(&leds[0]), 50.0) && near(cy(&leds[0]), -15.0));
    }

    #[test]
    fn shapes_give_one_sample_per_led_even_for_one_led() {
        for shape in [
            ZoneShape::Auto,
            ZoneShape::Strip,
            ZoneShape::Ring,
            ZoneShape::Outline,
            ZoneShape::Point,
            ZoneShape::Grid { columns: 2 },
        ] {
            let leds = shaped(shape, ZoneKind::Linear, 1, (0.0, 0.0, 50.0, 50.0));
            assert_eq!(leds.len(), 1, "{shape:?}");
            assert!(bounds(&leds[0])[2] > bounds(&leds[0])[0], "{shape:?}");
        }
    }

    // -----------------------------------------------------------------------
    // A zone split into several parts
    // -----------------------------------------------------------------------

    /// A placement that owns a slice of the zone's LEDs.
    fn slice(mut p: PlacedZone, start: u32, count: u32) -> PlacedZone {
        p.leds = Some(LedSlice { start, count });
        p
    }

    /// The 3-slice fixture: one 40-LED header split into three slices of 12, so the
    /// last 4 LEDs belong to nobody.
    fn split_zone() -> (Vec<PlacedZone>, Vec<LedSample>, Vec<SliceSamples>) {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 40)]);
        let lay = layout(vec![
            slice(placed("d0", 0, 0.0, 0.0, 120.0, 20.0), 0, 12),
            slice(placed("d0", 0, 0.0, 30.0, 120.0, 20.0), 12, 12),
            slice(placed("d0", 0, 0.0, 60.0, 120.0, 20.0), 24, 12),
        ]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let zone = &plan.devices[0].zones[0];
        (lay.zones, zone.leds.clone(), zone.slices.clone())
    }

    #[test]
    fn a_split_zone_lays_each_run_out_along_its_own_shape() {
        let (_, leds, slices) = split_zone();
        assert_eq!(leds.len(), 40);
        assert_eq!(slices.len(), 3);
        // The cells are sized by the slice (12), not by the zone (40): a slice of 12 in
        // a 120-unit rect has cells of 10 units, so the centres are 5, 15, ... 115.
        for slice_index in 0..3 {
            let first = slice_index * 12;
            for j in 0..12 {
                let led = &leds[first + j];
                assert!(
                    near(cx(led), 5.0 + 10.0 * j as f32),
                    "slice {slice_index} LED {j} sits at {}",
                    cx(led)
                );
                assert!(
                    near(cy(led), 10.0 + 30.0 * slice_index as f32),
                    "slice {slice_index} LED {j} sits at {}",
                    cy(led)
                );
            }
        }
    }

    #[test]
    fn the_leds_of_a_split_zone_that_nobody_owns_have_no_sample() {
        let (_, leds, _) = split_zone();
        // The 36 LEDs the three slices own are sampled.
        for led in leds.iter().take(36) {
            assert!(led.region().is_some(), "a claimed LED has a sample");
        }
        // The last 4 belong to nobody: they carry the black marker, so they are
        // never sampled and never read the effect's top-left pixel.
        for led in leds.iter().skip(36) {
            assert_eq!(led.region(), None, "an unclaimed LED has no sample");
            assert!(!led.sampled, "an unclaimed LED is never sampled");
        }
    }

    #[test]
    fn a_split_zone_reports_each_run_and_whether_it_is_placed() {
        let (_, _, slices) = split_zone();
        for (i, slice) in slices.iter().enumerate() {
            assert_eq!(slice.start, i as u32 * 12);
            assert_eq!(slice.count, 12);
            assert_eq!(slice.available, 12);
            assert!(slice.placed, "slice {i} is placed");
            assert!(!slice.needs_mapping, "slice {i} fits");
        }
    }

    #[test]
    fn a_run_that_is_reversed_flips_inside_its_own_run() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 24)]);
        let mut second = slice(placed("d0", 0, 0.0, 30.0, 120.0, 20.0), 12, 12);
        second.reversed = true;
        let lay = layout(vec![slice(placed("d0", 0, 0.0, 0.0, 120.0, 20.0), 0, 12), second]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // The first slice is not reversed: LED 0 is at the start of its own shape.
        assert!(near(cx(&leds[0]), 5.0));
        // The second slice is: LED 12 sits at the end of its own 12-LED shape, and
        // LED 23 at its start. The slice is flipped inside itself, not the zone.
        assert!(near(cx(&leds[12]), 115.0));
        assert!(near(cx(&leds[23]), 5.0));
    }

    #[test]
    fn a_run_sizes_its_ring_from_the_run_and_not_from_the_zone() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 40)]);
        let mut p = slice(placed("d0", 0, 0.0, 0.0, 40.0, 40.0), 0, 12);
        p.shape = ZoneShape::Ring;
        let lay = layout(vec![p]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // A ring of 12 in a 40-unit square: the first LED is at the top, the
        // second a twelfth of a turn further round.
        assert!(near(cx(&leds[0]), 20.0) && near(cy(&leds[0]), 0.0));
        let theta = std::f32::consts::TAU / 12.0;
        assert!(near(cx(&leds[1]), 20.0 + 20.0 * theta.sin()));
        assert!(near(cy(&leds[1]), 20.0 - 20.0 * theta.cos()));
    }

    #[test]
    fn a_run_past_the_end_of_the_zone_is_not_clipped_but_flagged() {
        // The zone has 20 LEDs; the slice claims 12 of them starting at 16.
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 20)]);
        let lay = layout(vec![slice(placed("d0", 0, 0.0, 0.0, 120.0, 20.0), 16, 12)]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let zone = &plan.devices[0].zones[0];
        assert_eq!(zone.slices.len(), 1);
        assert_eq!(zone.slices[0].count, 12, "the slice still claims 12 LEDs");
        assert_eq!(zone.slices[0].available, 4, "the zone only has 4 of them left");
        assert!(zone.slices[0].needs_mapping, "the slice has to be mapped again");
        // The four LEDs the zone still has are laid out; the eight that are gone
        // are not silently taken off the slice.
        assert!(zone.leds[16].region().is_some());
        assert!(zone.leds[19].region().is_some());
        assert_eq!(zone.leds.len(), 20);
    }

    #[test]
    fn a_matrix_zone_with_a_run_still_uses_its_map() {
        let map = vec![Some(0), Some(1), Some(2), Some(3)];
        let dev = device("d0", vec![zone(0, ZoneKind::Matrix { width: 2, height: 2, map }, 4)]);
        // A matrix zone is never split: the slice is ignored and the map owns the LEDs.
        let lay = layout(vec![slice(placed("d0", 0, 0.0, 0.0, 100.0, 100.0), 0, 2)]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let zone = &plan.devices[0].zones[0];
        assert!(zone.placed);
        assert!(near(cx(&zone.leds[0]), 25.0) && near(cy(&zone.leds[0]), 25.0));
        assert!(near(cx(&zone.leds[3]), 75.0) && near(cy(&zone.leds[3]), 75.0));
        assert_eq!(zone.slices.len(), 0, "the slice is ignored, so there is no slice");
    }

    #[test]
    fn a_zone_with_a_run_and_a_whole_zone_placement_keeps_the_whole_zone() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 40)]);
        let lay = layout(vec![
            placed("d0", 0, 0.0, 0.0, 120.0, 20.0),
            slice(placed("d0", 0, 0.0, 30.0, 120.0, 20.0), 12, 12),
        ]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let zone = &plan.devices[0].zones[0];
        // The whole zone wins, as it did before slices existed.
        assert!(near(cx(&zone.leds[0]), 1.5), "the whole zone's own cells");
        assert_eq!(zone.slices.len(), 1);
        assert!(!zone.slices[0].placed, "the slice is ignored");
    }
}
