// Scenes: a group's members as sample points on that group's own canvas.
//
// A group is a list of lights in the owner's order. Where each light's LEDs read the
// group's effect is decided here, once, when the plan is built: the group has one canvas
// (320x200, the same size the layout path uses) and every member gets a slot in it.
//
// Two modes, the group's `mode`:
// - **Flow across**: the canvas is cut into side-by-side slots, one per member in order.
//   A slot is as wide as that member's share of the group's LEDs, so a 12-LED light
//   beside a 36-LED light gets a third of the canvas and the 36-LED light the rest.
// - **Each its own**: every member's slot is the whole canvas, so every member shows the
//   whole picture.
//
// Inside a slot the light's own arrangement decides (the owner's choice on the Devices
// page): a strip runs left to right at mid-height, a ring is the largest circle that fits
// the slot, a grid fills the slot with the zone's own matrix map, and a single LED reads
// the slot's centre and averages over one logical unit there. `reversed` flips the LED
// order inside the light.
//
// This is the TypeScript port of the daemon's planner (`crates/canvas-core/src/arrangement.rs`),
// so the builder stage draws the picture the daemon samples. The Rust planner prints its
// own output for the fixed inputs (arrangement.vectors.json), and the editor's test checks
// this file against it.
//
// The samples are in the group's canvas units (0..320 x 0..200), the same units the layout
// path's samples are in before a render scale is applied.

import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { GroupMode } from "@/proto/GroupMode";
import type { Light } from "@/proto/Light";
import type { ZoneInfo } from "@/proto/ZoneInfo";

/** The canvas a group's effect draws on: the same size the layout path uses. */
export const CANVAS_WIDTH = 320;
export const CANVAS_HEIGHT = 200;

/** How far the ring's circle stays inside its slot, in canvas units. */
export const RING_INSET = 4;

/**
 * The most LEDs one zone may plan for.
 *
 * A zone reports its own LED count, and a damaged or misbehaving device can report a
 * number no real zone has. The cap keeps the planner from holding more than 4096 samples
 * (about 112 KiB, a sample being 28 bytes) for one light.
 */
export const MAX_ZONE_LEDS = 4096;

/** The largest a device's frame index can be: the Rust planner counts in u32. */
const U32_MAX = 4294967295;

/** One LED, as a sample point on the group's canvas. */
export interface LedSample {
  /** Sample point in canvas units (float, may be fractional). */
  center_x: number;
  /** Sample point in canvas units (float, may be fractional). */
  center_y: number;
  /** Area-average region, in canvas units, as `[x0, y0, x1, y1]`. */
  bounds: [number, number, number, number];
  /** False when the LED has no owner: it is black and is never sampled. */
  sampled: boolean;
}

/** One light's LEDs, as sample points on the group's canvas. */
export interface LightSamples {
  /** The light's id. */
  light: string;
  /** The device the light lives on. */
  device_id: string;
  /** Index of this light's first LED in its device's frame (zone order, then the slice start). */
  frame_start: number;
  /**
   * One sample per LED of the light, in LED order. A LED the arrangement has no cell for
   * (a matrix gap) carries the black marker and is never sampled.
   */
  leds: LedSample[];
}

/** A group's members, arranged on the group's canvas. */
export interface GroupPlan {
  /** The members that have LEDs, in the group's order. */
  lights: LightSamples[];
}

/** How many LEDs the plan covers. */
export function ledCount(plan: GroupPlan): number {
  let total = 0;
  for (const light of plan.lights) total += light.leds.length;
  return total;
}

/** The samples of one light, by its id. */
export function planLight(plan: GroupPlan, id: string): LightSamples | undefined {
  return plan.lights.find((l) => l.light === id);
}

/** The LED's sample region, or undefined when it has no owner and is black. */
export function region(
  led: LedSample,
): [number, number, [number, number, number, number]] | undefined {
  if (!led.sampled) return undefined;
  return [led.center_x, led.center_y, led.bounds];
}

/**
 * Builds the plan of a group: where each member's LEDs read the group's canvas.
 *
 * `members` is the group's member list, in the owner's order. A member that names no
 * light, or a light whose device is not in `devices`, is skipped. A light whose device is
 * there but whose zone has no LEDs left gets no slot and no samples. A member named
 * twice is one member: the first occurrence wins, and the light gets one slot.
 */
export function buildGroupPlan(
  members: readonly string[],
  mode: GroupMode,
  lights: readonly Light[],
  devices: readonly DeviceInfo[],
): GroupPlan {
  // The members that have LEDs, in the group's order.
  const placed: { light: Light; count: number }[] = [];
  for (const member of members) {
    // A member named twice is one member: the first occurrence holds the slot.
    if (placed.some((p) => p.light.id === member)) continue;
    const found = lights.find((l) => l.id === member);
    if (!found) continue;
    const count = lightLedCount(found, devices);
    if (count === 0) continue;
    placed.push({ light: found, count });
  }

  // The group's LEDs: what the slots are shared out by.
  let total = 0;
  for (const p of placed) total += p.count;
  const plan: GroupPlan = { lights: [] };
  let before = 0;
  for (const p of placed) {
    plan.lights.push({
      light: p.light.id,
      device_id: p.light.device_id,
      frame_start: frameStartOf(p.light, devices),
      leds: buildLightLeds(p.light, p.count, slotFor(mode, before, p.count, total), devices),
    });
    before += p.count;
  }
  return plan;
}

/**
 * The LED count a light has now: its slice's count clamped to the zone, or the zone's
 * count.
 *
 * A light whose device or zone is gone has no LEDs: the count is zero. A zone that
 * reports more LEDs than `MAX_ZONE_LEDS` is planned as the cap, so a damaged device
 * cannot make the planner plan for billions of LEDs.
 */
export function lightLedCount(light: Light, devices: readonly DeviceInfo[]): number {
  const zone = zoneOf(light, devices);
  if (!zone) return 0;
  const slice = light.leds;
  // A slice that runs past the end of a zone that shrank has only what is left.
  const count = slice
    ? Math.min(slice.count, Math.max(0, zone.led_count - slice.start))
    : zone.led_count;
  return Math.min(count, MAX_ZONE_LEDS);
}

/** A slot of the group's canvas: where one member reads the picture. */
interface Slot {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * The slot of one member: its share of the canvas in flow across, the whole canvas in
 * each its own.
 */
function slotFor(mode: GroupMode, before: number, count: number, total: number): Slot {
  if (mode === "each_its_own") {
    return { x: 0, y: 0, w: CANVAS_WIDTH, h: CANVAS_HEIGHT };
  }
  const x0 = (CANVAS_WIDTH * before) / total;
  const x1 = (CANVAS_WIDTH * (before + count)) / total;
  return { x: x0, y: 0, w: x1 - x0, h: CANVAS_HEIGHT };
}

/** The samples of one light's LEDs inside its slot. */
function buildLightLeds(
  light: Light,
  count: number,
  slot: Slot,
  devices: readonly DeviceInfo[],
): LedSample[] {
  const n = count;
  if (n === 0) return [];
  let leds: LedSample[];
  switch (light.arrangement) {
    case "single":
      leds = Array.from({ length: n }, () => singleSample(slot));
      break;
    case "strip":
      leds = stripSamples(slot, n);
      break;
    case "ring":
      leds = ringSamples(slot, n);
      break;
    case "grid":
      leds = gridSamples(light, count, slot, devices);
      break;
  }
  // `reversed` flips the LED order inside the light: LED 0 takes the last place.
  if (light.reversed) leds.reverse();
  return leds;
}

/** A strip: left to right at mid-height, each LED in the middle of its own cell. */
function stripSamples(slot: Slot, n: number): LedSample[] {
  const cellW = slot.w / n;
  const y = slot.y + slot.h / 2;
  const leds: LedSample[] = [];
  for (let j = 0; j < n; j++) {
    const x = slot.x + cellW * (j + 0.5);
    leds.push({
      center_x: x,
      center_y: y,
      bounds: clampCell(x - cellW / 2, slot.y, x + cellW / 2, slot.y + slot.h),
      sampled: true,
    });
  }
  return leds;
}

/** A ring: the largest circle that fits the slot, from the top, clockwise. */
function ringSamples(slot: Slot, n: number): LedSample[] {
  const cx = slot.x + slot.w / 2;
  const cy = slot.y + slot.h / 2;
  // The circle keeps a 4px inset from the slot's edges.
  const r = Math.max(0, (Math.min(slot.w, slot.h) - 2 * RING_INSET) / 2);
  // The LED's cell: a square as wide as the LEDs are apart around the circle.
  const side = Math.min((Math.PI * 2 * r) / n, 2 * r);
  const leds: LedSample[] = [];
  for (let j = 0; j < n; j++) {
    // -90° is the top of the circle; the angle grows clockwise (y grows downward).
    const theta = ((-90 + (360 * j) / n) * Math.PI) / 180;
    const x = cx + r * Math.cos(theta);
    const y = cy + r * Math.sin(theta);
    leds.push({
      center_x: x,
      center_y: y,
      bounds: clampCell(x - side / 2, y - side / 2, x + side / 2, y + side / 2),
      sampled: true,
    });
  }
  return leds;
}

/**
 * A grid: the zone's own matrix map fills the slot. The LEDs the map does not place keep
 * the black marker, and a light on a zone that is not a matrix falls back to a strip: the
 * hardware cannot tell a grid from a line.
 */
function gridSamples(
  light: Light,
  count: number,
  slot: Slot,
  devices: readonly DeviceInfo[],
): LedSample[] {
  const n = count;
  const zone = zoneOf(light, devices);
  if (!zone) return stripSamples(slot, n);
  if (zone.kind.type !== "matrix") return stripSamples(slot, n);
  const cols = zone.kind.width;
  const rows = zone.kind.height;
  if (cols === 0 || rows === 0) return stripSamples(slot, n);

  // The map names the zone's LEDs; the light owns a run of them.
  const start = light.leds ? light.leds.start : 0;
  const leds: LedSample[] = Array.from({ length: n }, () => blackSample());
  for (let row = 0; row < rows; row++) {
    for (let col = 0; col < cols; col++) {
      const led = zone.kind.map[row * cols + col];
      if (led === undefined || led === null) continue;
      if (led < start || led >= start + n) continue;
      leds[led - start] = gridCellSample(slot, col, row, cols, rows);
    }
  }
  return leds;
}

/** One cell of the grid, in the slot. */
function gridCellSample(slot: Slot, col: number, row: number, cols: number, rows: number): LedSample {
  const cellW = slot.w / cols;
  const cellH = slot.h / rows;
  const x = slot.x + cellW * (col + 0.5);
  const y = slot.y + cellH * (row + 0.5);
  return {
    center_x: x,
    center_y: y,
    bounds: clampCell(x - cellW / 2, y - cellH / 2, x + cellW / 2, y + cellH / 2),
    sampled: true,
  };
}

/**
 * A single LED: it reads the slot's centre, and its cell is one logical unit there.
 *
 * A single light is a point: it has no spread of its own to average over, so the
 * smallest cell the planner allows is the honest one. The daemon's default sampler is
 * area-average, so a whole-slot cell would make one LED report the mean of the entire
 * picture instead of the colour it sits on.
 */
function singleSample(slot: Slot): LedSample {
  const x = slot.x + slot.w / 2;
  const y = slot.y + slot.h / 2;
  return {
    center_x: x,
    center_y: y,
    bounds: clampCell(x - 0.5, y - 0.5, x + 0.5, y + 0.5),
    sampled: true,
  };
}

/** A LED with no owner: black, and never sampled. */
function blackSample(): LedSample {
  return { center_x: 0, center_y: 0, bounds: [0, 0, 0, 0], sampled: false };
}

/** Clamps a cell to the canvas and keeps it at least one logical unit wide and tall. */
function clampCell(x0: number, y0: number, x1: number, y1: number): [number, number, number, number] {
  let ax0 = Math.min(Math.max(x0, 0), CANVAS_WIDTH);
  let ay0 = Math.min(Math.max(y0, 0), CANVAS_HEIGHT);
  let ax1 = Math.min(Math.max(x1, 0), CANVAS_WIDTH);
  let ay1 = Math.min(Math.max(y1, 0), CANVAS_HEIGHT);
  if (ax1 - ax0 < 1) {
    ax1 = Math.min(ax0 + 1, CANVAS_WIDTH);
    ax0 = ax1 - 1;
  }
  if (ay1 - ay0 < 1) {
    ay1 = Math.min(ay0 + 1, CANVAS_HEIGHT);
    ay0 = ay1 - 1;
  }
  return [ax0, ay0, ax1, ay1];
}

/** The zone of a light, on the device that has it. */
function zoneOf(light: Light, devices: readonly DeviceInfo[]): ZoneInfo | undefined {
  const device = devices.find((d) => d.id === light.device_id);
  if (!device) return undefined;
  return device.zones.find((z) => z.index === light.zone_index);
}

/**
 * Where the light's first LED sits in its device's frame.
 *
 * A device's frame holds its zones in zone order, so the light starts after the zones
 * before its own; a slice starts after the LEDs before it in its zone. The frame follows
 * the device's real counts, and it saturates rather than overflowing on a device that
 * reports more LEDs than any frame could hold.
 */
function frameStartOf(light: Light, devices: readonly DeviceInfo[]): number {
  const device = devices.find((d) => d.id === light.device_id);
  if (!device) return 0;
  let base = 0;
  for (const zone of device.zones) {
    if (zone.index === light.zone_index) break;
    base = saturatingAdd(base, zone.led_count);
  }
  return saturatingAdd(base, light.leds ? light.leds.start : 0);
}

/** Adds two frame counts, saturating at the frame's own limit instead of overflowing. */
function saturatingAdd(a: number, b: number): number {
  return Math.min(a + b, U32_MAX);
}
