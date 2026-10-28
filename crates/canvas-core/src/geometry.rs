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

use canvas_proto::device::{DeviceId, DeviceInfo, ZoneInfo, ZoneKind};
use canvas_proto::layout::{Layout, PlacedZone, SamplingMode, ZoneShape};

/// One LED's sample region, in render pixels.
///
/// `center` is where nearest/bilinear sample. `bounds` is the axis-aligned region
/// area-average averages over (already the rotated cell's bounding box when the zone
/// is rotated).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LedSample {
    /// Sample point in render pixels (float, may be fractional for bilinear).
    pub center_x: f32,
    pub center_y: f32,
    /// Area-average region, in render pixels, as `[x0, y0, x1, y1]`.
    pub bounds: [f32; 4],
}

/// One zone's LEDs, in zone order.
#[derive(Debug, Clone)]
pub struct ZoneSamples {
    /// The zone's index inside its device.
    pub zone_index: u32,
    /// True when the layout places this zone visibly. An unplaced or invisible zone
    /// is never sampled: its LEDs are black.
    pub placed: bool,
    /// One entry per LED of the zone, in zone order.
    pub leds: Vec<LedSample>,
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
                    format!("z{}:{}{}", z.zone_index, z.leds.len(), state)
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
            let placed = layout.zones.iter().find(|p| {
                p.device_id == device.id && p.zone_index == zone.index
            });
            let visible = placed.is_some_and(|p| p.visible);
            let leds = match placed {
                Some(p) if p.visible => build_zone_leds(p, zone, scale),
                _ => vec![LedSample::default(); zone.led_count as usize],
            };
            zone_samples.push(ZoneSamples {
                zone_index: zone.index,
                placed: visible,
                leds,
            });
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

impl Default for LedSample {
    fn default() -> Self {
        LedSample {
            center_x: 0.0,
            center_y: 0.0,
            bounds: [0.0, 0.0, 0.0, 0.0],
        }
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
    let mut leds = vec![LedSample::default(); led_count];
    if led_count == 0 {
        return leds;
    }

    let shape = match (placed.shape, &zone.kind) {
        (ZoneShape::Auto, ZoneKind::Single) => ZoneShape::Point,
        (ZoneShape::Auto, ZoneKind::Linear) => ZoneShape::Strip,
        (ZoneShape::Auto, ZoneKind::Matrix { width, height, map }) => {
            fill_matrix(&mut leds, placed, *width as usize, *height as usize, map, scale);
            return leds;
        }
        (other, _) => other,
    };

    for (i, led) in leds.iter_mut().enumerate() {
        // `reversed` flips which end of the shape LED 0 sits at.
        let slot = if placed.reversed { led_count - 1 - i } else { i };
        let cell = shape_cell(placed, shape, slot, led_count);
        *led = sample_from_corners(&cell, scale);
    }
    leds
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::DeviceKind;

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
        assert!((leds[0].center_x - 12.5).abs() < 0.01);
        assert!((leds[1].center_x - 37.5).abs() < 0.01);
        assert!((leds[2].center_x - 62.5).abs() < 0.01);
        assert!((leds[3].center_x - 87.5).abs() < 0.01);
        assert!((leds[0].center_y - 10.0).abs() < 0.01);
    }

    #[test]
    fn tall_linear_zone_splits_vertically() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 3)]);
        let lay = layout(vec![placed("d0", 0, 10.0, 0.0, 20.0, 90.0)]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        assert!((leds[0].center_y - 15.0).abs() < 0.01);
        assert!((leds[1].center_y - 45.0).abs() < 0.01);
        assert!((leds[2].center_y - 75.0).abs() < 0.01);
    }

    #[test]
    fn reversed_linear_zone_flips_the_order() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 3)]);
        let mut p = placed("d0", 0, 0.0, 0.0, 90.0, 10.0);
        p.reversed = true;
        let lay = layout(vec![p]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        assert!((leds[0].center_x - 75.0).abs() < 0.01);
        assert!((leds[2].center_x - 15.0).abs() < 0.01);
    }

    #[test]
    fn matrix_zone_maps_cells_to_led_indices() {
        let map = vec![Some(0), Some(1), Some(2), Some(3)];
        let dev = device("d0", vec![zone(0, ZoneKind::Matrix { width: 2, height: 2, map }, 4)]);
        let lay = layout(vec![placed("d0", 0, 0.0, 0.0, 100.0, 100.0)]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // LED 0 is cell (0,0): top-left.
        assert!((leds[0].center_x - 25.0).abs() < 0.01);
        assert!((leds[0].center_y - 25.0).abs() < 0.01);
        // LED 1 is cell (1,0): top-right.
        assert!((leds[1].center_x - 75.0).abs() < 0.01);
        assert!((leds[1].center_y - 25.0).abs() < 0.01);
        // LED 2 is cell (0,1): bottom-left.
        assert!((leds[2].center_x - 25.0).abs() < 0.01);
        assert!((leds[2].center_y - 75.0).abs() < 0.01);
        // LED 3 is cell (1,1): bottom-right.
        assert!((leds[3].center_x - 75.0).abs() < 0.01);
        assert!((leds[3].center_y - 75.0).abs() < 0.01);
    }

    #[test]
    fn matrix_gaps_leave_their_led_black() {
        // map[2] is a gap, so LED 2 has no cell.
        let map = vec![Some(0), Some(1), None, Some(3)];
        let dev = device("d0", vec![zone(0, ZoneKind::Matrix { width: 2, height: 2, map }, 4)]);
        let lay = layout(vec![placed("d0", 0, 0.0, 0.0, 100.0, 100.0)]);
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        assert_eq!(leds[2].center_x, 0.0);
        assert_eq!(leds[2].center_y, 0.0);
        // LED 3 still lands in cell (1,1).
        assert!((leds[3].center_x - 75.0).abs() < 0.01);
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
        assert!((led.center_x - 120.0).abs() < 0.01);
        assert!((led.center_y - 110.0).abs() < 0.01);
        // But the axis-aligned bounding box swaps width and height.
        assert!((led.bounds[2] - led.bounds[0] - 20.0).abs() < 0.01);
        assert!((led.bounds[3] - led.bounds[1] - 40.0).abs() < 0.01);
    }

    #[test]
    fn render_scale_multiplies_the_sample_coordinates() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 2)]);
        let mut lay = layout(vec![placed("d0", 0, 0.0, 0.0, 320.0, 200.0)]);
        lay.render_scale = canvas_proto::layout::RenderScale::P50;
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // Logical 80 -> render 40 at 50%.
        assert!((leds[0].center_x - 40.0).abs() < 0.01);
        assert!((leds[0].center_y - 50.0).abs() < 0.01);
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
        assert!((leds[0].center_x - 40.0).abs() < 0.01);
        assert!((leds[0].bounds[0] - 0.0).abs() < 0.01);
        assert!((leds[0].bounds[2] - 80.0).abs() < 0.01);

        // 50%: the same zone covers render pixels 0..40.
        let mut lay = layout(vec![base.clone()]);
        lay.render_scale = canvas_proto::layout::RenderScale::P50;
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        assert_eq!(plan.scale, 0.5);
        let leds = &plan.devices[0].zones[0].leds;
        assert!((leds[0].center_x - 20.0).abs() < 0.01);
        assert!((leds[0].bounds[0] - 0.0).abs() < 0.01);
        assert!((leds[0].bounds[2] - 40.0).abs() < 0.01);

        // 25%: the same zone covers render pixels 0..20.
        let mut lay = layout(vec![base]);
        lay.render_scale = canvas_proto::layout::RenderScale::P25;
        let plan = build_plan(&lay, std::slice::from_ref(&dev));
        assert_eq!(plan.scale, 0.25);
        let leds = &plan.devices[0].zones[0].leds;
        assert!((leds[0].center_x - 10.0).abs() < 0.01);
        assert!((leds[0].bounds[0] - 0.0).abs() < 0.01);
        assert!((leds[0].bounds[2] - 20.0).abs() < 0.01);
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
        assert!(z0.iter().all(|l| l.center_x == 0.0 && l.center_y == 0.0));
        assert!(z1.iter().all(|l| l.center_x == 0.0 && l.center_y == 0.0));
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
            assert!(near(led.center_x, 110.0) && near(led.center_y, 55.0));
            assert_eq!(led.bounds, [100.0, 50.0, 120.0, 60.0]);
        }
    }

    #[test]
    fn point_shape_makes_every_led_sample_the_whole_rect() {
        let leds = shaped(ZoneShape::Point, ZoneKind::Linear, 5, (10.0, 20.0, 40.0, 30.0));
        for led in &leds {
            assert!(near(led.center_x, 30.0) && near(led.center_y, 35.0));
            assert_eq!(led.bounds, [10.0, 20.0, 50.0, 50.0]);
        }
    }

    #[test]
    fn strip_shape_forces_a_strip_on_a_single_zone() {
        let leds = shaped(ZoneShape::Strip, ZoneKind::Single, 4, (0.0, 0.0, 100.0, 20.0));
        assert!(near(leds[0].center_x, 12.5) && near(leds[3].center_x, 87.5));
        assert!(near(leds[0].center_y, 10.0));
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
        assert!(near(a[0].center_y, 15.0) && near(a[5].center_y, 115.0));
    }

    #[test]
    fn ring_places_leds_clockwise_from_the_top() {
        // Square 100x100 at the origin, 4 LEDs: top, right, bottom, left.
        let leds = shaped(ZoneShape::Ring, ZoneKind::Linear, 4, (0.0, 0.0, 100.0, 100.0));
        assert!(near(leds[0].center_x, 50.0) && near(leds[0].center_y, 0.0));
        assert!(near(leds[1].center_x, 100.0) && near(leds[1].center_y, 50.0));
        assert!(near(leds[2].center_x, 50.0) && near(leds[2].center_y, 100.0));
        assert!(near(leds[3].center_x, 0.0) && near(leds[3].center_y, 50.0));
    }

    #[test]
    fn ring_follows_an_ellipse_when_the_rect_is_not_square() {
        let leds = shaped(ZoneShape::Ring, ZoneKind::Linear, 4, (0.0, 0.0, 200.0, 100.0));
        assert!(near(leds[1].center_x, 200.0) && near(leds[1].center_y, 50.0));
        assert!(near(leds[0].center_x, 100.0) && near(leds[0].center_y, 0.0));
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
        assert!(near(leds[0].center_x, 0.0) && near(leds[0].center_y, 50.0));
        assert!(near(leds[1].center_x, 50.0) && near(leds[1].center_y, 100.0));
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
        assert!(near(leds[0].center_x, 100.0) && near(leds[0].center_y, 50.0));
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
            assert!(near(led.center_x, x) && near(led.center_y, y), "{led:?} vs {x},{y}");
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
        assert!(near(leds[0].center_x, 0.0) && near(leds[0].center_y, 100.0));
        assert!(near(leds[3].center_x, 0.0) && near(leds[3].center_y, 0.0));
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
            assert!(near(led.center_x, x) && near(led.center_y, y));
        }
        assert_eq!(leds[0].bounds, [0.0, 0.0, 30.0, 30.0]);
    }

    #[test]
    fn grid_with_a_ragged_last_row_and_zero_columns_stays_in_bounds() {
        let leds =
            shaped(ZoneShape::Grid { columns: 4 }, ZoneKind::Linear, 5, (0.0, 0.0, 80.0, 40.0));
        assert_eq!(leds.len(), 5);
        assert!(near(leds[4].center_x, 10.0) && near(leds[4].center_y, 30.0));
        let one =
            shaped(ZoneShape::Grid { columns: 0 }, ZoneKind::Linear, 3, (0.0, 0.0, 10.0, 30.0));
        assert!(near(one[2].center_y, 25.0) && near(one[2].center_x, 5.0));
    }

    #[test]
    fn matrix_zone_with_a_shape_overrides_its_map() {
        let map = vec![Some(0), None, None, Some(1)];
        let kind = ZoneKind::Matrix { width: 2, height: 2, map };
        let auto = shaped(ZoneShape::Auto, kind.clone(), 2, (0.0, 0.0, 100.0, 100.0));
        assert!(near(auto[1].center_x, 75.0) && near(auto[1].center_y, 75.0));
        let strip = shaped(ZoneShape::Strip, kind, 2, (0.0, 0.0, 100.0, 100.0));
        // Width >= height, so the strip runs along x: LED 1 is the right half.
        assert!(near(strip[1].center_x, 75.0) && near(strip[1].center_y, 50.0));
    }

    #[test]
    fn rotated_strip_keeps_rotation() {
        let dev = device("d0", vec![zone(0, ZoneKind::Linear, 2)]);
        let mut p = placed("d0", 0, 0.0, 0.0, 100.0, 20.0);
        p.rotation_deg = 90.0;
        let plan = build_plan(&layout(vec![p]), std::slice::from_ref(&dev));
        let leds = &plan.devices[0].zones[0].leds;
        // Centre (50,10); LED 0 at (25,10) rotates clockwise a quarter to (50,-15).
        assert!(near(leds[0].center_x, 50.0) && near(leds[0].center_y, -15.0));
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
            assert!(leds[0].bounds[2] > leds[0].bounds[0], "{shape:?}");
        }
    }
}
