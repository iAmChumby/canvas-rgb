//! Scenes: a group's members as sample points on that group's own canvas.
//!
//! A group is a list of lights in the owner's order. Where each light's LEDs read the
//! group's effect is decided here, once, when the plan is built: the group has one canvas
//! (320x200, the same size the layout path uses) and every member gets a slot in it.
//!
//! Two modes, the group's `mode`:
//! - **Flow across**: the canvas is cut into side-by-side slots, one per member in order.
//!   A slot is as wide as that member's share of the group's LEDs, so a 12-LED light
//!   beside a 36-LED light gets a third of the canvas and the 36-LED light the rest.
//! - **Each its own**: every member's slot is the whole canvas, so every member shows the
//!   whole picture.
//!
//! Inside a slot the light's own arrangement decides (the owner's choice on the Devices
//! page): a strip runs left to right at mid-height, a ring is the largest circle that fits
//! the slot, a grid fills the slot with the zone's own matrix map, and a single LED reads
//! the slot's centre. `reversed` flips the LED order inside the light.
//!
//! Nothing here runs per frame: the plan is built when the scene or the hardware changes,
//! and the engine samples it every frame.
//!
//! The samples are in the group's canvas units (0..320 x 0..200), the same units the layout
//! path's samples are in before a render scale is applied.

use canvas_proto::device::{DeviceId, DeviceInfo, ZoneInfo, ZoneKind};
use canvas_proto::layout::{CANVAS_HEIGHT, CANVAS_WIDTH};
use canvas_proto::scene::{Arrangement, GroupMode, Light, LightId};

use crate::geometry::LedSample;

/// How far the ring's circle stays inside its slot, in canvas units.
const RING_INSET: f32 = 4.0;

/// One light's LEDs, as sample points on the group's canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct LightSamples {
    /// The light's id.
    pub light: LightId,
    /// The device the light lives on.
    pub device_id: DeviceId,
    /// Index of this light's first LED in its device's frame (zone order, then the slice
    /// start).
    pub frame_start: u32,
    /// One sample per LED of the light, in LED order. A LED the arrangement has no cell
    /// for (a matrix gap) carries the black marker and is never sampled.
    pub leds: Vec<LedSample>,
}

/// A group's members, arranged on the group's canvas.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupPlan {
    /// The members that have LEDs, in the group's order.
    pub lights: Vec<LightSamples>,
}

impl GroupPlan {
    /// How many LEDs the plan covers.
    pub fn led_count(&self) -> u32 {
        self.lights.iter().map(|l| l.leds.len() as u32).sum()
    }

    /// The samples of one light, by its id.
    pub fn light(&self, id: &str) -> Option<&LightSamples> {
        self.lights.iter().find(|l| l.light == id)
    }
}

/// Builds the plan of a group: where each member's LEDs read the group's canvas.
///
/// `members` is the group's member list, in the owner's order. A member that names no
/// light, or a light whose device is not in `devices`, is skipped. A light whose device is
/// there but whose zone has no LEDs left gets no slot and no samples.
pub fn build_group_plan(
    members: &[LightId],
    mode: GroupMode,
    lights: &[Light],
    devices: &[DeviceInfo],
) -> GroupPlan {
    // The members that have LEDs, in the group's order.
    let mut placed: Vec<(&Light, u32)> = Vec::with_capacity(members.len());
    for member in members {
        let Some(light) = lights.iter().find(|l| l.id == *member) else {
            continue;
        };
        let count = light_led_count(light, devices);
        if count == 0 {
            continue;
        }
        placed.push((light, count));
    }

    // The group's LEDs: what the slots are shared out by.
    let total: u32 = placed.iter().map(|(_, count)| *count).sum();
    let mut plan = GroupPlan::default();
    let mut before = 0u32;
    for (light, count) in placed {
        plan.lights.push(LightSamples {
            light: light.id.clone(),
            device_id: light.device_id.clone(),
            frame_start: frame_start_of(light, devices),
            leds: build_light_leds(light, count, &slot_for(mode, before, count, total), devices),
        });
        before += count;
    }
    plan
}

/// The LED count a light has now: its slice's count clamped to the zone, or the zone's
/// count.
///
/// A light whose device or zone is gone has no LEDs: the count is zero.
pub fn light_led_count(light: &Light, devices: &[DeviceInfo]) -> u32 {
    let Some(zone) = zone_of(light, devices) else {
        return 0;
    };
    match light.leds {
        None => zone.led_count,
        // A slice that runs past the end of a zone that shrank has only what is left.
        Some(slice) => slice.count.min(zone.led_count.saturating_sub(slice.start)),
    }
}

/// A slot of the group's canvas: where one member reads the picture.
#[derive(Debug, Clone, Copy)]
struct Slot {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Slot {
    /// The slot's centre.
    fn centre(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// The slot of one member: its share of the canvas in flow across, the whole canvas in
/// each its own.
fn slot_for(mode: GroupMode, before: u32, count: u32, total: u32) -> Slot {
    match mode {
        GroupMode::FlowAcross => {
            let width = CANVAS_WIDTH as f32;
            let x0 = width * before as f32 / total as f32;
            let x1 = width * (before + count) as f32 / total as f32;
            Slot { x: x0, y: 0.0, w: x1 - x0, h: CANVAS_HEIGHT as f32 }
        }
        GroupMode::EachItsOwn => {
            Slot { x: 0.0, y: 0.0, w: CANVAS_WIDTH as f32, h: CANVAS_HEIGHT as f32 }
        }
    }
}

/// The samples of one light's LEDs inside its slot.
fn build_light_leds(
    light: &Light,
    count: u32,
    slot: &Slot,
    devices: &[DeviceInfo],
) -> Vec<LedSample> {
    let n = count as usize;
    if n == 0 {
        return Vec::new();
    }
    let mut leds = match light.arrangement {
        Arrangement::Single => vec![single_sample(slot); n],
        Arrangement::Strip => strip_samples(slot, n),
        Arrangement::Ring => ring_samples(slot, n),
        Arrangement::Grid => grid_samples(light, count, slot, devices),
    };
    // `reversed` flips the LED order inside the light: LED 0 takes the last place.
    if light.reversed {
        leds.reverse();
    }
    leds
}

/// A strip: left to right at mid-height, each LED in the middle of its own cell.
fn strip_samples(slot: &Slot, n: usize) -> Vec<LedSample> {
    let cell_w = slot.w / n as f32;
    let y = slot.y + slot.h / 2.0;
    (0..n)
        .map(|j| {
            let x = slot.x + cell_w * (j as f32 + 0.5);
            let bounds =
                clamp_cell(x - cell_w / 2.0, slot.y, x + cell_w / 2.0, slot.y + slot.h);
            LedSample { center_x: x, center_y: y, bounds, sampled: true }
        })
        .collect()
}

/// A ring: the largest circle that fits the slot, from the top, clockwise.
fn ring_samples(slot: &Slot, n: usize) -> Vec<LedSample> {
    let (cx, cy) = slot.centre();
    // The circle keeps a 4px inset from the slot's edges.
    let r = ((slot.w.min(slot.h) - 2.0 * RING_INSET) / 2.0).max(0.0);
    // The LED's cell: a square as wide as the LEDs are apart around the circle.
    let side = (std::f32::consts::TAU * r / n as f32).min(2.0 * r);
    (0..n)
        .map(|j| {
            // -90° is the top of the circle; the angle grows clockwise (y grows downward).
            let theta = (-90.0 + 360.0 * j as f32 / n as f32).to_radians();
            let x = cx + r * theta.cos();
            let y = cy + r * theta.sin();
            let bounds =
                clamp_cell(x - side / 2.0, y - side / 2.0, x + side / 2.0, y + side / 2.0);
            LedSample { center_x: x, center_y: y, bounds, sampled: true }
        })
        .collect()
}

/// A grid: the zone's own matrix map fills the slot. The LEDs the map does not place keep
/// the black marker, and a light on a zone that is not a matrix falls back to a strip: the
/// hardware cannot tell a grid from a line.
fn grid_samples(
    light: &Light,
    count: u32,
    slot: &Slot,
    devices: &[DeviceInfo],
) -> Vec<LedSample> {
    let n = count as usize;
    let Some(zone) = zone_of(light, devices) else {
        return strip_samples(slot, n);
    };
    let ZoneKind::Matrix { width, height, map } = &zone.kind else {
        return strip_samples(slot, n);
    };
    let (cols, rows) = (*width as usize, *height as usize);
    if cols == 0 || rows == 0 {
        return strip_samples(slot, n);
    }

    // The map names the zone's LEDs; the light owns a run of them.
    let start = light.leds.map(|s| s.start as usize).unwrap_or(0);
    let mut leds = vec![LedSample::default(); n];
    for row in 0..rows {
        for col in 0..cols {
            let Some(Some(led)) = map.get(row * cols + col).copied() else {
                continue;
            };
            let Ok(led) = usize::try_from(led) else {
                continue;
            };
            if led < start || led >= start + n {
                continue;
            }
            leds[led - start] = grid_cell_sample(slot, col, row, cols, rows);
        }
    }
    leds
}

/// One cell of the grid, in the slot.
fn grid_cell_sample(
    slot: &Slot,
    col: usize,
    row: usize,
    cols: usize,
    rows: usize,
) -> LedSample {
    let cell_w = slot.w / cols as f32;
    let cell_h = slot.h / rows as f32;
    let x = slot.x + cell_w * (col as f32 + 0.5);
    let y = slot.y + cell_h * (row as f32 + 0.5);
    let bounds = clamp_cell(x - cell_w / 2.0, y - cell_h / 2.0, x + cell_w / 2.0, y + cell_h / 2.0);
    LedSample { center_x: x, center_y: y, bounds, sampled: true }
}

/// A single LED: it reads the slot's centre.
fn single_sample(slot: &Slot) -> LedSample {
    let (x, y) = slot.centre();
    let bounds = clamp_cell(x - 0.5, y - 0.5, x + 0.5, y + 0.5);
    LedSample { center_x: x, center_y: y, bounds, sampled: true }
}

/// Clamps a cell to the canvas and keeps it at least one pixel wide and tall.
fn clamp_cell(x0: f32, y0: f32, x1: f32, y1: f32) -> [f32; 4] {
    let width = CANVAS_WIDTH as f32;
    let height = CANVAS_HEIGHT as f32;
    let mut x0 = x0.clamp(0.0, width);
    let mut y0 = y0.clamp(0.0, height);
    let mut x1 = x1.clamp(0.0, width);
    let mut y1 = y1.clamp(0.0, height);
    if x1 - x0 < 1.0 {
        x1 = (x0 + 1.0).min(width);
        x0 = x1 - 1.0;
    }
    if y1 - y0 < 1.0 {
        y1 = (y0 + 1.0).min(height);
        y0 = y1 - 1.0;
    }
    [x0, y0, x1, y1]
}

/// The zone of a light, on the device that has it.
fn zone_of<'a>(light: &Light, devices: &'a [DeviceInfo]) -> Option<&'a ZoneInfo> {
    let device = devices.iter().find(|d| d.id == light.device_id)?;
    device.zones.iter().find(|z| z.index == light.zone_index)
}

/// Where the light's first LED sits in its device's frame.
///
/// A device's frame holds its zones in zone order, so the light starts after the zones
/// before its own; a slice starts after the LEDs before it in its zone.
fn frame_start_of(light: &Light, devices: &[DeviceInfo]) -> u32 {
    let Some(device) = devices.iter().find(|d| d.id == light.device_id) else {
        return 0;
    };
    let mut base = 0u32;
    for zone in &device.zones {
        if zone.index == light.zone_index {
            break;
        }
        base += zone.led_count;
    }
    base + light.leds.map(|s| s.start).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::DeviceKind;
    use canvas_proto::layout::LedSlice;

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

    fn light(id: &str, device_id: &str, zone_index: u32, arrangement: Arrangement) -> Light {
        Light {
            id: id.to_string(),
            device_id: DeviceId(device_id.to_string()),
            zone_index,
            leds: None,
            name: None,
            arrangement,
            reversed: false,
        }
    }

    /// A strip light on the first zone of its own device.
    fn strip(id: &str, device_id: &str) -> Light {
        light(id, device_id, 0, Arrangement::Strip)
    }

    fn ids(list: &[&str]) -> Vec<LightId> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flow_across_gives_slots_by_led_share() {
        let devices = vec![
            device("d0", vec![zone(0, ZoneKind::Linear, 12)]),
            device("d1", vec![zone(0, ZoneKind::Linear, 36)]),
        ];
        let lights = vec![strip("a", "d0"), strip("b", "d1")];
        let plan = build_group_plan(&ids(&["a", "b"]), GroupMode::FlowAcross, &lights, &devices);
        assert_eq!(plan.lights.len(), 2);
        let (first, second) = (&plan.lights[0], &plan.lights[1]);
        assert_eq!(first.light, "a");
        assert_eq!(second.light, "b");
        // 12 of the group's 48 LEDs: the first slot is a third of the canvas.
        for led in &first.leds {
            assert!(led.center_x < 80.0, "the first light's LED sits at {}", led.center_x);
        }
        for led in &second.leds {
            assert!(led.center_x >= 80.0, "the second light's LED sits at {}", led.center_x);
        }
    }

    #[test]
    fn each_its_own_gives_every_light_the_whole_canvas() {
        let devices = vec![
            device("d0", vec![zone(0, ZoneKind::Linear, 12)]),
            device("d1", vec![zone(0, ZoneKind::Linear, 36)]),
        ];
        let lights = vec![strip("a", "d0"), strip("b", "d1")];
        let plan = build_group_plan(&ids(&["a", "b"]), GroupMode::EachItsOwn, &lights, &devices);
        assert_eq!(plan.lights.len(), 2);
        for light in &plan.lights {
            let min = light.leds.iter().map(|s| s.center_x).fold(f32::INFINITY, f32::min);
            let max = light.leds.iter().map(|s| s.center_x).fold(f32::NEG_INFINITY, f32::max);
            assert!(max - min > 250.0, "{} spans {}", light.light, max - min);
        }
    }

    #[test]
    fn strip_runs_left_to_right_at_mid_height() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 8)])];
        let lights = vec![strip("a", "d0")];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::FlowAcross, &lights, &devices);
        let leds = &plan.lights[0].leds;
        assert_eq!(leds.len(), 8);
        let mut last = f32::NEG_INFINITY;
        for led in leds {
            assert!(led.center_x > last, "the xs ascend: {} then {}", last, led.center_x);
            assert!((led.center_y - 100.0).abs() < 0.01, "the y is mid-height: {}", led.center_y);
            last = led.center_x;
        }
    }

    #[test]
    fn ring_starts_at_the_top_and_goes_clockwise() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 8)])];
        let lights = vec![light("a", "d0", 0, Arrangement::Ring)];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        let leds = &plan.lights[0].leds;
        assert_eq!(leds.len(), 8);
        let (cx, cy) = (160.0f32, 100.0f32);
        assert!((leds[0].center_x - cx).abs() < 0.01, "LED 0 sits at {}", leds[0].center_x);
        assert!(leds[0].center_y < cy, "LED 0 sits at {}", leds[0].center_y);
        // A quarter of the way round the circle is right of the centre.
        assert!(leds[2].center_x > cx, "LED 2 sits at {}", leds[2].center_x);
        assert!((leds[2].center_y - cy).abs() < 0.01, "LED 2 sits at {}", leds[2].center_y);
    }

    #[test]
    fn reversed_flips_led_order() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 6)])];
        let mut reversed = strip("a", "d0");
        reversed.reversed = true;
        let plan = build_group_plan(&ids(&["a"]), GroupMode::FlowAcross, &[reversed], &devices);
        let leds = &plan.lights[0].leds;
        let mut last = f32::INFINITY;
        for led in leds {
            assert!(led.center_x < last, "the xs descend: {} then {}", last, led.center_x);
            last = led.center_x;
        }
    }

    #[test]
    fn single_reads_the_slot_centre() {
        let devices = vec![
            device("d0", vec![zone(0, ZoneKind::Linear, 12)]),
            device("d1", vec![zone(0, ZoneKind::Linear, 36)]),
        ];
        let lights = vec![light("a", "d0", 0, Arrangement::Single), strip("b", "d1")];
        let plan = build_group_plan(&ids(&["a", "b"]), GroupMode::FlowAcross, &lights, &devices);
        let first = &plan.lights[0];
        assert_eq!(first.leds.len(), 12);
        for led in &first.leds {
            // The first slot is 0..80, so its centre is at x=40.
            assert!((led.center_x - 40.0).abs() < 0.01, "the LED sits at {}", led.center_x);
            assert!((led.center_y - 100.0).abs() < 0.01, "the LED sits at {}", led.center_y);
        }
    }

    #[test]
    fn grid_on_a_matrix_fills_the_slot_and_skips_gaps() {
        let map = vec![Some(0), Some(1), None, Some(3)];
        let devices =
            vec![device("d0", vec![zone(0, ZoneKind::Matrix { width: 2, height: 2, map }, 4)])];
        let lights = vec![light("a", "d0", 0, Arrangement::Grid)];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        let leds = &plan.lights[0].leds;
        assert_eq!(leds.len(), 4);
        // The gap LED has no cell: it is black and never sampled.
        assert_eq!(leds[2].region(), None, "the gap LED has no cell");
        // LED 0 is the top-left cell of the canvas, LED 3 the bottom-right.
        assert!((leds[0].center_x - 80.0).abs() < 0.01, "LED 0 sits at {}", leds[0].center_x);
        assert!((leds[0].center_y - 50.0).abs() < 0.01, "LED 0 sits at {}", leds[0].center_y);
        assert!((leds[1].center_x - 240.0).abs() < 0.01, "LED 1 sits at {}", leds[1].center_x);
        assert!((leds[3].center_x - 240.0).abs() < 0.01, "LED 3 sits at {}", leds[3].center_x);
        assert!((leds[3].center_y - 150.0).abs() < 0.01, "LED 3 sits at {}", leds[3].center_y);
    }

    #[test]
    fn grid_on_a_linear_zone_falls_back_to_strip() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 4)])];
        let lights = vec![light("a", "d0", 0, Arrangement::Grid)];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        let leds = &plan.lights[0].leds;
        assert_eq!(leds.len(), 4);
        // A strip: left to right at mid-height.
        assert!((leds[0].center_x - 40.0).abs() < 0.01, "LED 0 sits at {}", leds[0].center_x);
        assert!((leds[0].center_y - 100.0).abs() < 0.01, "LED 0 sits at {}", leds[0].center_y);
        assert!((leds[3].center_x - 280.0).abs() < 0.01, "LED 3 sits at {}", leds[3].center_x);
    }

    #[test]
    fn a_slice_uses_only_its_run_and_its_frame_start() {
        let devices = vec![device(
            "d0",
            vec![zone(0, ZoneKind::Linear, 8), zone(1, ZoneKind::Linear, 40)],
        )];
        let mut sliced = strip("a", "d0");
        sliced.zone_index = 1;
        sliced.leds = Some(LedSlice { start: 12, count: 12 });
        let plan = build_group_plan(&ids(&["a"]), GroupMode::FlowAcross, &[sliced], &devices);
        let light = &plan.lights[0];
        // The zone before it has 8 LEDs, so the slice starts at 8 + 12.
        assert_eq!(light.frame_start, 20);
        assert_eq!(light.leds.len(), 12);
    }

    #[test]
    fn a_member_whose_device_is_gone_is_skipped_without_panic() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 12)])];
        let lights = vec![strip("a", "d0"), strip("ghost", "gone")];
        let plan = build_group_plan(
            &ids(&["a", "ghost", "nothing"]),
            GroupMode::FlowAcross,
            &lights,
            &devices,
        );
        assert_eq!(plan.lights.len(), 1);
        assert_eq!(plan.lights[0].light, "a");
        // The gone light has no LEDs at all.
        assert_eq!(light_led_count(&lights[1], &devices), 0);
    }

    #[test]
    fn a_slice_that_runs_past_its_zone_is_clamped() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 20)])];
        let mut sliced = strip("a", "d0");
        sliced.leds = Some(LedSlice { start: 16, count: 12 });
        // The zone only has 4 LEDs left of the slice.
        assert_eq!(light_led_count(&sliced, &devices), 4);
        let plan = build_group_plan(&ids(&["a"]), GroupMode::FlowAcross, &[sliced], &devices);
        assert_eq!(plan.lights[0].leds.len(), 4);
        // The slice still starts at 16 in the device's frame.
        assert_eq!(plan.lights[0].frame_start, 16);
    }

    #[test]
    fn a_grid_light_maps_only_the_leds_of_its_slice() {
        let map: Vec<Option<u32>> = (0..16).map(Some).collect();
        let devices =
            vec![device("d0", vec![zone(0, ZoneKind::Matrix { width: 4, height: 4, map }, 16)])];
        let mut sliced = light("a", "d0", 0, Arrangement::Grid);
        sliced.leds = Some(LedSlice { start: 12, count: 4 });
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &[sliced], &devices);
        let leds = &plan.lights[0].leds;
        assert_eq!(leds.len(), 4);
        // The slice is the bottom row of the matrix: y is below the centre.
        for led in leds {
            assert!(led.center_y > 100.0, "the LED sits at {}", led.center_y);
        }
        // And the first LED of the slice is the bottom-left cell.
        assert!((leds[0].center_x - 40.0).abs() < 0.01, "LED 0 sits at {}", leds[0].center_x);
        assert!((leds[0].center_y - 175.0).abs() < 0.01, "LED 0 sits at {}", leds[0].center_y);
    }

    #[test]
    fn every_sample_stays_inside_the_canvas() {
        for n in 1..=300u32 {
            let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, n)])];
            for arrangement in
                [Arrangement::Single, Arrangement::Strip, Arrangement::Ring, Arrangement::Grid]
            {
                let mut light = light("a", "d0", 0, arrangement);
                light.reversed = n % 2 == 0;
                for mode in [GroupMode::FlowAcross, GroupMode::EachItsOwn] {
                    let plan = build_group_plan(
                        &ids(&["a"]),
                        mode,
                        std::slice::from_ref(&light),
                        &devices,
                    );
                    for led in &plan.lights[0].leds {
                        let [x0, y0, x1, y1] = led.bounds;
                        let what = format!("n={n} {arrangement:?} {mode:?} {led:?}");
                        assert!(x0 >= 0.0 && x1 <= CANVAS_WIDTH as f32, "{what}");
                        assert!(y0 >= 0.0 && y1 <= CANVAS_HEIGHT as f32, "{what}");
                        assert!(x1 - x0 >= 1.0, "{what}");
                        assert!(y1 - y0 >= 1.0, "{what}");
                        assert!(
                            led.center_x >= 0.0 && led.center_x <= CANVAS_WIDTH as f32,
                            "{what}"
                        );
                        assert!(
                            led.center_y >= 0.0 && led.center_y <= CANVAS_HEIGHT as f32,
                            "{what}"
                        );
                    }
                }
            }
        }
    }

    /// The same property on a real matrix zone, so the grid's own cells are checked too:
    /// on a linear zone a Grid light falls back to a strip and never reaches that code.
    #[test]
    fn every_matrix_sample_stays_inside_the_canvas() {
        for (cols, rows) in [(1u32, 1u32), (2, 2), (3, 5), (10, 10), (17, 17), (25, 12)] {
            let n = cols * rows;
            if n > 300 {
                continue;
            }
            // The zone's own map: cell (row, col) holds the LED of that number.
            let map: Vec<Option<u32>> = (0..n).map(Some).collect();
            let devices = vec![device(
                "d0",
                vec![zone(0, ZoneKind::Matrix { width: cols, height: rows, map }, n)],
            )];
            let mut light = light("a", "d0", 0, Arrangement::Grid);
            light.reversed = n % 2 == 0;
            for mode in [GroupMode::FlowAcross, GroupMode::EachItsOwn] {
                let plan = build_group_plan(
                    &ids(&["a"]),
                    mode,
                    std::slice::from_ref(&light),
                    &devices,
                );
                for led in &plan.lights[0].leds {
                    let [x0, y0, x1, y1] = led.bounds;
                    let what = format!("{cols}x{rows} {mode:?} {led:?}");
                    assert!(x0 >= 0.0 && x1 <= CANVAS_WIDTH as f32, "{what}");
                    assert!(y0 >= 0.0 && y1 <= CANVAS_HEIGHT as f32, "{what}");
                    assert!(x1 - x0 >= 1.0, "{what}");
                    assert!(y1 - y0 >= 1.0, "{what}");
                    assert!(led.center_x >= 0.0 && led.center_x <= CANVAS_WIDTH as f32, "{what}");
                    assert!(led.center_y >= 0.0 && led.center_y <= CANVAS_HEIGHT as f32, "{what}");
                }
            }
        }
    }
}
