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
//!
//! # The cells
//!
//! Every sample carries the cell it averages over, clamped to the canvas:
//! - a strip cell is `slot.w / n` wide and the slot's whole height,
//! - a grid cell is the matrix's own cell,
//! - a single light's cell is one logical unit at the slot's centre,
//! - a ring cell is the square of one LED's share of the circle, `2 * pi * r / n`.
//!
//! Three edge cases the cells meet:
//! - **The one-unit minimum is logical.** A cell never falls below one canvas unit, but
//!   the engine scales the plan by the render factor before it samples, so at quarter
//!   scale that one unit is a quarter of a canvas pixel. Sampling falls back to the
//!   nearest pixel, so nothing crashes; the guarantee just does not survive scaling.
//! - **A ring in a slot narrower than twice the inset has no circle.** Its radius is 0
//!   and every one of its LEDs lands on the slot's centre.
//! - **A single light with more than one LED puts them all on one point**, the slot's
//!   centre, each with the same one-unit cell. The zone still has that many LEDs; they
//!   just all read the same pixel.
//!
//! # Where this and the layout path disagree
//!
//! A grid light on a matrix zone with a slice maps only the LEDs of that slice, so the
//! LEDs outside the slice have no cell and stay black. The layout path
//! (`geometry.rs`) ignores a slice on a matrix zone entirely, because the map owns every
//! LED of the zone. The two paths answer "who owns these LEDs" differently; the builder
//! calls this a 0.15 case.
//!
//! # Counts
//!
//! A zone reports its LED count, and a damaged device can report a number no real zone
//! has. Every count is capped at `MAX_ZONE_LEDS`, and the group's total is summed in
//! `u64`, so a broken count cannot overflow the sum or make the planner allocate more
//! than the cap for one light.
//!
//! The `u64` sums are a guard, not a load-bearing fix under the cap: overflowing a `u32`
//! total needs 1,048,576 members at 4096 LEDs each, which no scene has, and no test can
//! build one. They stay because the member list comes from the caller and the cap is a
//! policy that may rise; a wrapped total would quietly divide the canvas by a wrong
//! number, and the wider sum costs nothing once per plan.
//!
//! The precision limit is the `f32` cast of the share, not the sum: `f32` holds `before`
//! and `total` exactly up to 2^24 (16,777,216 LEDs), and a group under the cap holds at
//! most 4096 LEDs a member, so the cast is exact for every scene this repo can build. See
//! `slot_for`.
//!
//! A light is planned for at most `MAX_ZONE_LEDS` LEDs from its own start (LED 0 of the
//! zone, or the slice's start). The LEDs past the cap have no sample, so the engine's
//! device frame leaves them black. The next zone still starts after its predecessor's
//! real count (see `frame_start_of`), not after the cap.

use canvas_proto::device::{DeviceId, DeviceInfo, ZoneInfo, ZoneKind};
use canvas_proto::layout::{CANVAS_HEIGHT, CANVAS_WIDTH};
use canvas_proto::scene::{Arrangement, GroupMode, Light, LightId};

use crate::geometry::LedSample;

/// How far the ring's circle stays inside its slot, in canvas units.
const RING_INSET: f32 = 4.0;

/// The most LEDs one zone may plan for.
///
/// A zone reports its own LED count, and a damaged or misbehaving device can report a
/// number no real zone has. The largest zone this repo knows of is an OpenRGB zone
/// resized to its maximum of 512 LEDs (`crates/canvas-daemon/src/first_run.rs`), and the
/// desk-shaped fake in `crates/canvas-core/src/cpu.rs` has 100 LEDs a zone. The cap is
/// eight times the largest of those, so every real zone plans in full, while a broken
/// count can never make the planner hold more than 4096 samples (about 112 KiB, a
/// `LedSample` being 28 bytes) for one light.
pub const MAX_ZONE_LEDS: u32 = 4096;

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
    /// for (a matrix gap) carries the black marker and is never sampled, and a zone that
    /// reports more than `MAX_ZONE_LEDS` gets samples only for its first cap's worth.
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
        // The sum is in u64 as a guard (see the module doc's "Counts"); the plan's own
        // count saturates at u32.
        let total: u64 = self.lights.iter().map(|l| l.leds.len() as u64).sum();
        total.min(u32::MAX as u64) as u32
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
/// there but whose zone has no LEDs left gets no slot and no samples. A member named
/// twice is one member: the first occurrence wins, and the light gets one slot.
pub fn build_group_plan(
    members: &[LightId],
    mode: GroupMode,
    lights: &[Light],
    devices: &[DeviceInfo],
) -> GroupPlan {
    // The members that have LEDs, in the group's order.
    let mut placed: Vec<(&Light, u32)> = Vec::with_capacity(members.len());
    for member in members {
        // A member named twice is one member: the first occurrence holds the slot.
        if placed.iter().any(|(l, _)| l.id == *member) {
            continue;
        }
        let Some(light) = lights.iter().find(|l| l.id == *member) else {
            continue;
        };
        let count = light_led_count(light, devices);
        if count == 0 {
            continue;
        }
        placed.push((light, count));
    }

    // The group's LEDs: what the slots are shared out by. The u64 is a guard, not a
    // requirement under the cap: see the module doc's "Counts".
    let total: u64 = placed.iter().map(|(_, count)| u64::from(*count)).sum();
    let mut plan = GroupPlan::default();
    let mut before = 0u64;
    for (light, count) in placed {
        plan.lights.push(LightSamples {
            light: light.id.clone(),
            device_id: light.device_id.clone(),
            frame_start: frame_start_of(light, devices),
            leds: build_light_leds(
                light,
                count,
                &slot_for(mode, before, u64::from(count), total),
                devices,
            ),
        });
        before += u64::from(count);
    }
    plan
}

/// The LED count a light has now: its slice's count clamped to the zone, or the zone's
/// count.
///
/// A light whose device or zone is gone has no LEDs: the count is zero. A zone that
/// reports more LEDs than `MAX_ZONE_LEDS` is planned as the cap, so a damaged device
/// cannot make the planner allocate for billions of LEDs.
pub fn light_led_count(light: &Light, devices: &[DeviceInfo]) -> u32 {
    let Some(zone) = zone_of(light, devices) else {
        return 0;
    };
    let count = match light.leds {
        None => zone.led_count,
        // A slice that runs past the end of a zone that shrank has only what is left.
        Some(slice) => slice.count.min(zone.led_count.saturating_sub(slice.start)),
    };
    count.min(MAX_ZONE_LEDS)
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
///
/// The counts are in `u64` so a group of many capped zones cannot overflow the share.
///
/// The share is then divided in `f32`, which is exact for `before` and `total` up to
/// 2^24 (16,777,216 LEDs): a group under the cap holds at most 4096 LEDs a member, so
/// every scene this repo can build divides exactly. The TS port divides in a double,
/// exact to 2^53; the two agree far inside the vectors' 1e-4 tolerance.
fn slot_for(mode: GroupMode, before: u64, count: u64, total: u64) -> Slot {
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
///
/// The cell is `slot.w / n` wide and the slot's whole height.
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
///
/// The cell is the square of one LED's share of the circle, `2 * pi * r / n`, capped at
/// the circle's own diameter.
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

    // The map names the zone's LEDs; the light owns a run of them. The map is the walk,
    // not the matrix: a damaged matrix can report a huge width and height (100000 x
    // 100000 is 10^10 cells) for a map that names one LED, and walking the matrix would
    // freeze the plan.
    let start = light.leds.map(|s| s.start as usize).unwrap_or(0);
    let cells = (cols as u64).saturating_mul(rows as u64);
    let bound = map.len().min(usize::try_from(cells).unwrap_or(usize::MAX));
    let mut leds = vec![LedSample::default(); n];
    for (index, entry) in map.iter().take(bound).enumerate() {
        let Some(led) = *entry else {
            continue;
        };
        let Ok(led) = usize::try_from(led) else {
            continue;
        };
        if led < start || led >= start + n {
            continue;
        }
        leds[led - start] = grid_cell_sample(slot, index % cols, index / cols, cols, rows);
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

/// A single LED: it reads the slot's centre, and its cell is one logical unit there.
///
/// A single light is a point: it has no spread of its own to average over, so the
/// smallest cell the planner allows is the honest one. The caller's default sampler is
/// area-average, so a whole-slot cell would make one LED report the mean of the entire
/// picture instead of the colour it sits on.
fn single_sample(slot: &Slot) -> LedSample {
    let (x, y) = slot.centre();
    let bounds = clamp_cell(x - 0.5, y - 0.5, x + 0.5, y + 0.5);
    LedSample { center_x: x, center_y: y, bounds, sampled: true }
}

/// Clamps a cell to the canvas and keeps it at least one logical unit wide and tall.
///
/// The unit is logical: the engine scales the plan by the render factor before it
/// samples, so at quarter scale the smallest cell is a quarter of a canvas pixel.
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
/// before its own; a slice starts after the LEDs before it in its zone. The frame
/// follows the device's real counts, not the cap, and it saturates rather than
/// overflowing on a device that reports more LEDs than any frame could hold.
fn frame_start_of(light: &Light, devices: &[DeviceInfo]) -> u32 {
    let Some(device) = devices.iter().find(|d| d.id == light.device_id) else {
        return 0;
    };
    let mut base = 0u32;
    for zone in &device.zones {
        if zone.index == light.zone_index {
            break;
        }
        base = base.saturating_add(zone.led_count);
    }
    base.saturating_add(light.leds.map(|s| s.start).unwrap_or(0))
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

    /// Checks that a sample's bounds hold its centre and are as wide and as tall as the
    /// cell around it.
    fn assert_cell(led: &LedSample, cell_w: f32, cell_h: f32, what: &str) {
        let [x0, y0, x1, y1] = led.bounds;
        assert!(
            x0 <= led.center_x && led.center_x <= x1,
            "{what}: the centre x {} is not in {x0}..{x1}",
            led.center_x
        );
        assert!(
            y0 <= led.center_y && led.center_y <= y1,
            "{what}: the centre y {} is not in {y0}..{y1}",
            led.center_y
        );
        assert!((x1 - x0 - cell_w).abs() < 0.05, "{what}: the cell is {} wide, not {cell_w}", x1 - x0);
        assert!((y1 - y0 - cell_h).abs() < 0.05, "{what}: the cell is {} tall, not {cell_h}", y1 - y0);
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
            // The slot is the whole canvas: the first cell starts at 0 and the last ends
            // at 320, so the light covers the whole picture and not a part of it.
            let first = light.leds.first().expect("the light has LEDs");
            let last = light.leds.last().expect("the light has LEDs");
            assert!(first.bounds[0].abs() < 0.05, "{} starts at {}", light.light, first.bounds[0]);
            assert!(
                (last.bounds[2] - CANVAS_WIDTH as f32).abs() < 0.05,
                "{} ends at {}",
                light.light,
                last.bounds[2]
            );
            assert!(first.bounds[1].abs() < 0.05, "{} starts at {}", light.light, first.bounds[1]);
            assert!(
                (last.bounds[3] - CANVAS_HEIGHT as f32).abs() < 0.05,
                "{} ends at {}",
                light.light,
                last.bounds[3]
            );
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

    /// The ring's circle keeps its inset from the slot's edges: 4 units, so the top LED
    /// of a ring on the whole canvas sits at y = 4 and the bottom one at y = 196.
    #[test]
    fn the_ring_keeps_its_inset_from_the_slot() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 8)])];
        let lights = vec![light("a", "d0", 0, Arrangement::Ring)];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        let leds = &plan.lights[0].leds;
        assert_eq!(leds.len(), 8);
        // LED 0 is at the top of the circle, 4 units from the top of the slot.
        assert!((leds[0].center_x - 160.0).abs() < 0.01, "LED 0 sits at {}", leds[0].center_x);
        assert!((leds[0].center_y - 4.0).abs() < 0.01, "LED 0 sits at {}", leds[0].center_y);
        // And the bottom of the circle keeps the same inset.
        assert!((leds[4].center_y - 196.0).abs() < 0.01, "LED 4 sits at {}", leds[4].center_y);
    }

    /// A ring in a slot narrower than twice the inset has no circle to draw: its radius
    /// is 0 and every LED lands on the slot's centre.
    #[test]
    fn a_ring_in_a_narrow_slot_collapses_to_its_centre() {
        let devices = vec![
            device("d0", vec![zone(0, ZoneKind::Linear, 1)]),
            device("d1", vec![zone(0, ZoneKind::Linear, 99)]),
        ];
        let lights = vec![light("a", "d0", 0, Arrangement::Ring), strip("b", "d1")];
        let plan = build_group_plan(&ids(&["a", "b"]), GroupMode::FlowAcross, &lights, &devices);
        // The ring's slot is a 3.2 unit slice of the canvas: too narrow for a circle.
        let led = plan.lights[0].leds[0];
        assert!((led.center_x - 1.6).abs() < 0.01, "the LED sits at {}", led.center_x);
        assert!((led.center_y - 100.0).abs() < 0.01, "the LED sits at {}", led.center_y);
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

    /// A single light with more than one LED puts all of them on one point: the slot's
    /// centre.
    #[test]
    fn a_single_light_with_many_leds_puts_them_on_one_point() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 12)])];
        let lights = vec![light("a", "d0", 0, Arrangement::Single)];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        let leds = &plan.lights[0].leds;
        assert_eq!(leds.len(), 12);
        for led in leds {
            assert!((led.center_x - 160.0).abs() < 0.01, "the LED sits at {}", led.center_x);
            assert!((led.center_y - 100.0).abs() < 0.01, "the LED sits at {}", led.center_y);
        }
    }

    /// The bounds of a sample hold its centre and are the cell around it: a strip cell is
    /// `slot.w / n` wide, a grid cell is the grid's own cell, a single light's cell is one
    /// logical unit at the slot's centre, and a ring cell is the square of one LED's share
    /// of the circle.
    #[test]
    fn every_bounds_holds_its_centre_and_its_cell() {
        let width = CANVAS_WIDTH as f32;
        let height = CANVAS_HEIGHT as f32;

        // A strip alone: the slot is the whole canvas, so each of its 8 cells is a
        // 320/8 = 40 wide column of the whole height.
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 8)])];
        let plan =
            build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &[strip("a", "d0")], &devices);
        for led in &plan.lights[0].leds {
            assert_cell(led, width / 8.0, height, "a strip cell");
        }

        // A grid alone on a 4x4 matrix: each cell is 320/4 by 200/4.
        let map: Vec<Option<u32>> = (0..16).map(Some).collect();
        let devices =
            vec![device("d0", vec![zone(0, ZoneKind::Matrix { width: 4, height: 4, map }, 16)])];
        let lights = vec![light("a", "d0", 0, Arrangement::Grid)];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        for led in &plan.lights[0].leds {
            assert_cell(led, width / 4.0, height / 4.0, "a grid cell");
        }

        // A single light alone: it reads the slot's centre, and its cell is one logical
        // unit there, not the whole slot. A single point has no spread to average over,
        // so the smallest cell the planner allows is the honest one.
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 1)])];
        let lights = vec![light("a", "d0", 0, Arrangement::Single)];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        let led = plan.lights[0].leds[0];
        assert_cell(&led, 1.0, 1.0, "a single light's cell");
        assert!((led.center_x - width / 2.0).abs() < 0.01, "the centre x is {}", led.center_x);
        assert!((led.center_y - height / 2.0).abs() < 0.01, "the centre y is {}", led.center_y);
        let [x0, y0, x1, y1] = led.bounds;
        assert!((x0 - (width / 2.0 - 0.5)).abs() < 0.01, "the cell starts at x = {x0}");
        assert!((x1 - (width / 2.0 + 0.5)).abs() < 0.01, "the cell ends at x = {x1}");
        assert!((y0 - (height / 2.0 - 0.5)).abs() < 0.01, "the cell starts at y = {y0}");
        assert!((y1 - (height / 2.0 + 0.5)).abs() < 0.01, "the cell ends at y = {y1}");

        // A ring alone: the cell of each LED is the square of its share of the circle.
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 8)])];
        let lights = vec![light("a", "d0", 0, Arrangement::Ring)];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        let radius = (height.min(width) - 2.0 * RING_INSET) / 2.0;
        let side = std::f32::consts::TAU * radius / 8.0;
        for led in &plan.lights[0].leds {
            let [x0, y0, x1, y1] = led.bounds;
            assert!(
                x0 <= led.center_x && led.center_x <= x1,
                "a ring cell: the centre x {} is not in {x0}..{x1}",
                led.center_x
            );
            assert!(
                y0 <= led.center_y && led.center_y <= y1,
                "a ring cell: the centre y {} is not in {y0}..{y1}",
                led.center_y
            );
            assert!((x1 - x0 - side).abs() < 0.05, "a ring cell: the cell is {} wide, not {side}", x1 - x0);
            // The circle's top and bottom LEDs have their cell cut off by the canvas.
            assert!(
                y1 - y0 <= side + 0.05 && y1 - y0 >= 1.0,
                "a ring cell: the cell is {} tall",
                y1 - y0
            );
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

    /// A zone that is there but has no LEDs gets no slot and no samples.
    #[test]
    fn a_zone_with_no_leds_gets_no_slot() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 0)])];
        let lights = vec![strip("a", "d0")];
        assert_eq!(light_led_count(&lights[0], &devices), 0);
        let plan = build_group_plan(&ids(&["a"]), GroupMode::FlowAcross, &lights, &devices);
        assert!(plan.lights.is_empty(), "a light with no LEDs gets no slot");
        assert_eq!(plan.led_count(), 0);
    }

    /// A member named twice is one member: it gets one slot and its LEDs are counted
    /// once.
    #[test]
    fn a_member_listed_twice_gets_one_slot() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 12)])];
        let lights = vec![strip("a", "d0")];
        let plan = build_group_plan(&ids(&["a", "a"]), GroupMode::FlowAcross, &lights, &devices);
        assert_eq!(plan.lights.len(), 1, "the same light twice gets one slot");
        assert_eq!(plan.lights[0].light, "a");
        assert_eq!(plan.lights[0].leds.len(), 12);
        assert_eq!(plan.led_count(), 12, "the light's LEDs are counted once");
    }

    /// A zone that reports more LEDs than the cap is planned as the cap: the planner
    /// never plans more than `MAX_ZONE_LEDS` samples for one light.
    #[test]
    fn a_zone_that_reports_too_many_leds_is_capped() {
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 100_000)])];
        let lights = vec![strip("a", "d0")];
        assert_eq!(light_led_count(&lights[0], &devices), MAX_ZONE_LEDS);
        let plan = build_group_plan(&ids(&["a"]), GroupMode::FlowAcross, &lights, &devices);
        assert_eq!(plan.lights[0].leds.len(), MAX_ZONE_LEDS as usize);
        assert_eq!(plan.led_count(), MAX_ZONE_LEDS);
    }

    /// Two zones that report billions of LEDs: every count is capped, so the group's
    /// total never overflows and the plan never holds billions of samples.
    #[test]
    fn two_huge_zones_do_not_overflow_the_sum() {
        let devices = vec![
            device("d0", vec![zone(0, ZoneKind::Linear, 3_000_000_000)]),
            device("d1", vec![zone(0, ZoneKind::Linear, 3_000_000_000)]),
        ];
        let lights = vec![strip("a", "d0"), strip("b", "d1")];
        let plan = build_group_plan(&ids(&["a", "b"]), GroupMode::FlowAcross, &lights, &devices);
        assert_eq!(plan.led_count(), 2 * MAX_ZONE_LEDS);
        // The two slots share the canvas: the first holds the left half.
        let first = plan.lights[0].leds[0].center_x;
        let second = plan.lights[1].leds[0].center_x;
        assert!(first < 160.0, "the first light sits at {first}");
        assert!(second > 160.0, "the second light sits at {second}");
    }

    /// One device, two zones, the first over the cap: the device's frame is built from the
    /// real zone counts, so the second zone starts where the first one really ends, not
    /// after the cap. The first zone's LEDs past the cap get no sample at all.
    #[test]
    fn a_second_zone_starts_after_the_first_zone_s_real_leds() {
        let first_zone: u32 = 100_000;
        let devices = vec![device(
            "d0",
            vec![zone(0, ZoneKind::Linear, first_zone), zone(1, ZoneKind::Linear, 12)],
        )];
        let mut second = strip("b", "d0");
        second.zone_index = 1;
        let lights = vec![strip("a", "d0"), second];
        let plan =
            build_group_plan(&ids(&["a", "b"]), GroupMode::EachItsOwn, &lights, &devices);

        // The frame the engine builds for this device holds every real LED of both zones.
        assert_eq!(devices[0].led_count(), first_zone + 12);
        // The second zone starts after the first zone's real 100_000 LEDs, not after the
        // 4096 the planner samples.
        let second = plan.light("b").expect("the second light is planned");
        assert_eq!(second.frame_start, first_zone);
        assert_eq!(second.leds.len(), 12);

        // The first zone is planned as the cap's worth of samples...
        let first = plan.light("a").expect("the first light is planned");
        assert_eq!(first.frame_start, 0);
        assert_eq!(first.leds.len(), MAX_ZONE_LEDS as usize);
        // ...and the LEDs past the cap have no sample: nothing in the plan covers frame
        // index 4096, nor any index between the cap and the second zone's start.
        for led_index in [MAX_ZONE_LEDS, first_zone - 1] {
            let covered = plan.lights.iter().any(|light| {
                let end = light.frame_start as u64 + light.leds.len() as u64;
                (light.frame_start as u64) <= u64::from(led_index) && u64::from(led_index) < end
            });
            assert!(!covered, "LED {led_index} of the device frame has a sample");
        }
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

    /// A damaged matrix can report a huge width and height with a map that names one LED.
    /// The planner walks the map, not the matrix: 100000 x 100000 cells is 10^10 of them,
    /// and the map holds one.
    #[test]
    fn a_damaged_matrix_plans_from_its_map_not_its_dimensions() {
        let devices = vec![device(
            "d0",
            vec![zone(
                0,
                ZoneKind::Matrix { width: 100_000, height: 100_000, map: vec![Some(0)] },
                1,
            )],
        )];
        let lights = vec![light("a", "d0", 0, Arrangement::Grid)];
        let started = std::time::Instant::now();
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        let elapsed = started.elapsed();
        let leds = &plan.lights[0].leds;
        assert_eq!(leds.len(), 1);
        assert!(leds[0].region().is_some(), "the map's one LED has a cell");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "the plan took {elapsed:?}"
        );
    }

    /// A map shorter than the matrix names fewer LEDs than the matrix has cells: the
    /// planner plans the LEDs the map names, in the cells it names them, and the rest of
    /// the matrix is empty.
    #[test]
    fn a_map_shorter_than_the_matrix_plans_the_leds_it_names() {
        // A 3x3 matrix whose map holds only its top row.
        let map = vec![Some(0), Some(1), Some(2)];
        let devices = vec![device(
            "d0",
            vec![zone(0, ZoneKind::Matrix { width: 3, height: 3, map }, 3)],
        )];
        let lights = vec![light("a", "d0", 0, Arrangement::Grid)];
        let plan = build_group_plan(&ids(&["a"]), GroupMode::EachItsOwn, &lights, &devices);
        let leds = &plan.lights[0].leds;
        assert_eq!(leds.len(), 3);
        let mut last = f32::NEG_INFINITY;
        for led in leds {
            assert!(led.region().is_some(), "a named LED has a cell");
            // The three LEDs are the matrix's top row: the cells of 3x3.
            assert!((led.center_y - 200.0 / 6.0).abs() < 0.01, "the LED sits at {}", led.center_y);
            assert!(led.center_x > last, "the xs ascend: {} then {}", last, led.center_x);
            last = led.center_x;
        }
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
