// Zone geometry on the 320 x 200 logical canvas: where each LED of a placed zone sits.
// The zone's `shape` decides it (strip, ring, outline, point, grid; `auto` follows the device's
// zone kind), and `reversed` flips the order. Rotation is clockwise about the rect's centre, as
// CSS rotate() draws it and as the daemon samples it.
//
// The desk canvas draws every pointer move without a React render, so the geometry is split in
// two: `zoneLedOffsets` gives the LED positions in the zone's own frame (they only change when
// the zone is resized or reversed) and `zoneTransform` gives the frame's place and angle.
//
// One zone can drive several parts (three fans daisy-chained on one ARGB header are three
// placements of the same zone). A placement with `leds` owns a run of the zone's LEDs and draws
// only those, sized from its own run, the way the engine samples them.

import { MM_PER_UNIT } from "@/lib/caseForms";
import type { PlacedZone } from "@/proto/PlacedZone";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DeviceKind } from "@/proto/DeviceKind";
import type { ZoneShape } from "@/proto/ZoneShape";

const CANVAS_W = 320;
const CANVAS_H = 200;

/** The units a person reads: centimetres or inches. */
export type LengthUnit = "cm" | "in";

/** Where the owner's choice of unit is remembered on this machine. */
export const UNIT_KEY = "canvas-rgb.units";

/** The regions that measure in inches: the United States, Liberia and Myanmar. */
const IMPERIAL_REGIONS = new Set(["US", "LR", "MM"]);

/**
 * The unit a region reads in. The system's language tag carries the region ("en-US"): the first
 * tag that names one decides, and a tag with no region ("en") says nothing. Everywhere else is cm.
 */
export function unitForLocale(tags: readonly string[]): LengthUnit {
  for (const tag of tags) {
    let region: string | undefined;
    try {
      const loc = new Intl.Locale(tag);
      region = loc.region?.toUpperCase();
    } catch {
      region = /[-_]([A-Za-z]{2})/.exec(tag)?.[1]?.toUpperCase();
    }
    if (region) return IMPERIAL_REGIONS.has(region) ? "in" : "cm";
  }
  return "cm";
}

/** The unit this machine's region uses, until the owner picks one. */
export function systemUnit(): LengthUnit {
  if (typeof navigator === "undefined") return "cm";
  const tags = navigator.languages?.length ? navigator.languages : [navigator.language];
  return unitForLocale(tags.filter(Boolean));
}

/** The owner's unit: what they picked last, else what their region uses. */
export function initialUnit(): LengthUnit {
  try {
    const u = window.localStorage.getItem(UNIT_KEY);
    if (u === "cm" || u === "in") return u;
  } catch {
    // Not remembered: the region decides.
  }
  return systemUnit();
}

/** Remembers the owner's pick. */
export function rememberUnit(unit: LengthUnit): void {
  try {
    window.localStorage.setItem(UNIT_KEY, unit);
  } catch {
    // Still used for this session.
  }
}

/** How many canvas units one centimetre is: 4 mm to the unit. */
export const UNITS_PER_CM = 10 / MM_PER_UNIT;

/** A length in millimetres, from a length in canvas units. */
export function unitsToMm(units: number): number {
  return units * MM_PER_UNIT;
}

/** A length in canvas units, from a length in millimetres. */
export function mmToUnits(mm: number): number {
  return mm / MM_PER_UNIT;
}

/**
 * A length in the reader's own unit, in words: "12.0 cm" or "4.72 in". The editor never shows
 * a canvas unit: they mean nothing outside this file.
 */
export function formatLength(mm: number, unit: LengthUnit): string {
  if (unit === "in") return `${(mm / 25.4).toFixed(2)} in`;
  return `${(mm / 10).toFixed(1)} cm`;
}

/** The same, from a length in canvas units. */
export function formatUnits(units: number, unit: LengthUnit): string {
  return formatLength(unitsToMm(units), unit);
}

/** A length in the reader's unit, from millimetres: the number without its unit. */
export function lengthValue(mm: number, unit: LengthUnit): string {
  return unit === "in" ? (mm / 25.4).toFixed(2) : (mm / 10).toFixed(1);
}

/** A length in the reader's unit, from millimetres: the number itself. */
export function lengthNumber(mm: number, unit: LengthUnit): number {
  return unit === "in" ? mm / 25.4 : mm / 10;
}

/** A length in millimetres, from a number in the reader's unit. */
export function mmFromLength(value: number, unit: LengthUnit): number {
  return unit === "in" ? value * 25.4 : value * 10;
}

/** The unit word a readout carries. */
export function unitWord(unit: LengthUnit): string {
  return unit === "in" ? "in" : "cm";
}

export interface Point {
  x: number;
  y: number;
}

/** The zone's centre on the canvas and its rotation, as cos/sin. */
export interface ZoneTransform {
  cx: number;
  cy: number;
  cos: number;
  sin: number;
}

export function findZoneInfo(
  devices: DeviceInfo[] | null,
  deviceId: string,
  zoneIndex: number,
): { device: DeviceInfo; zone: ZoneInfo } | null {
  const device = devices?.find((d) => d.id === deviceId);
  const zone = device?.zones.find((z) => z.index === zoneIndex);
  if (!device || !zone) return null;
  return { device, zone };
}

/**
 * The key of one placement: the zone it belongs to and, when the zone is split, where its run
 * of LEDs starts. Two placements of the same zone are two different parts, so they are two
 * different keys.
 */
export function zoneKey(z: PlacedZone): string {
  const base = `${z.device_id}#${z.zone_index}`;
  return z.leds ? `${base}@${z.leds.start}` : base;
}

/** The key of the zone itself, without the run: one zone is one zone however it is split. */
export function zoneOwnerKey(z: PlacedZone): string {
  return `${z.device_id}#${z.zone_index}`;
}

export function zoneTransform(z: PlacedZone): ZoneTransform {
  const rad = (z.rotation_deg * Math.PI) / 180;
  return {
    cx: z.x + z.width / 2,
    cy: z.y + z.height / 2,
    cos: Math.cos(rad),
    sin: Math.sin(rad),
  };
}

/** The shape a zone's LEDs really follow, once `auto` has been resolved against the zone's kind. */
export type ResolvedShape = "strip" | "ring" | "outline" | "point" | "grid" | "matrix";

/** The names the shape picker and the labels use. */
export const SHAPE_LABEL: Record<ZoneShape["type"], string> = {
  auto: "Auto",
  strip: "Strip",
  ring: "Ring",
  outline: "Outline",
  point: "Point",
  grid: "Grid",
};

/** The shape types in the order the picker lists them. */
export const SHAPE_ORDER: ZoneShape["type"][] = ["auto", "strip", "ring", "outline", "point", "grid"];

/**
 * What `auto` means for a zone: a single zone is a point, a linear one a strip, a matrix its own
 * map. The daemon resolves it the same way (`crates/canvas-core/src/geometry.rs`).
 */
export function resolveShape(shape: ZoneShape, kind: ZoneInfo["kind"]): ResolvedShape {
  if (shape.type === "auto") {
    if (kind.type === "single") return "point";
    if (kind.type === "linear") return "strip";
    return "matrix";
  }
  return shape.type;
}

/** One LED in the zone's own frame (origin at the rect's centre, before rotation). */
export interface LocalLed {
  /** The cell's centre, in canvas units. */
  x: number;
  y: number;
  /** The cell's size: the region the LED samples. */
  w: number;
  h: number;
}

export interface LocalLeds {
  shape: ResolvedShape;
  /** One entry per LED, in zone order; null for an LED with no cell (a gap in a matrix map). */
  leds: (LocalLed | null)[];
  /** How big a dot for one LED may be, in canvas units: it never touches its neighbour. */
  radius: number;
}

/**
 * Where each LED of a zone sits in the zone's own frame, for the zone's shape. This is the
 * daemon's own geometry (`build_zone_leds` in `crates/canvas-core/src/geometry.rs`), so a dot on
 * the desk and the LED it stands for are the same place:
 *
 *   strip    cells along the long axis
 *   ring     on the inscribed ellipse, clockwise from the top
 *   outline  on the rect's edge, clockwise from the top-left corner
 *   point    every LED samples the whole rect
 *   grid     row by row, top-left first
 *   matrix   the device's own map (`auto` on a matrix zone)
 *
 * `reversed` flips the order of the LEDs along the shape; a matrix keeps its map.
 *
 * A placement that owns a run of the zone's LEDs (`leds`) lays out only that run, and sizes its
 * cells from the run's own count: three fans daisy-chained on one header are three rings of
 * their own LEDs, not three arcs of one big ring. The engine samples a slice the same way.
 */
export function zoneLocalLeds(z: PlacedZone, info: ZoneInfo): LocalLeds {
  const shape = resolveShape(z.shape, info.kind);
  const total = Math.max(0, info.led_count);
  const slice = z.leds;
  // A matrix zone is never split: its map owns every LED, and a slice on it is ignored (the
  // engine says so too, `crates/canvas-core/src/geometry.rs`).
  const split = slice && shape !== "matrix";
  // The shape is sized by the slice: a run of twelve LEDs gets a ring of twelve, not a ring of
  // the whole zone. `count` is the run's own count, `take` is how many of them the zone still
  // has: a zone that shrank under the run keeps the run's shape and is flagged, never clipped.
  const count = split ? slice.count : total;
  const take = split ? Math.max(0, Math.min(slice.count, total - slice.start)) : total;
  const w = z.width;
  const h = z.height;
  const leds: (LocalLed | null)[] = [];
  if (take === 0) return { shape, leds, radius: 1 };
  const kind = info.kind;
  if (shape === "matrix" && kind.type === "matrix") {
    const cw = w / kind.width;
    const ch = h / kind.height;
    for (let i = 0; i < total; i++) leds.push(null);
    for (let r = 0; r < kind.height; r++) {
      for (let c = 0; c < kind.width; c++) {
        const idx = kind.map[r * kind.width + c];
        if (idx === null || idx === undefined || idx >= total) continue;
        leds[idx] = { x: -w / 2 + (c + 0.5) * cw, y: -h / 2 + (r + 0.5) * ch, w: cw, h: ch };
      }
    }
    return { shape, leds, radius: Math.min(cw, ch) * 0.22 };
  }
  const horizontal = w >= h;
  const perimeterOfRect = 2 * (w + h);
  const a = w / 2;
  const b = h / 2;
  // Ramanujan's approximation of the ellipse perimeter, as the daemon uses.
  const perimeterOfEllipse = Math.PI * (3 * (a + b) - Math.sqrt((3 * a + b) * (a + 3 * b)));
  const cols = z.shape.type === "grid" ? Math.min(Math.max(1, z.shape.columns), count) : 1;
  const rows = Math.ceil(count / cols);
  for (let i = 0; i < take; i++) {
    const slot = z.reversed ? count - 1 - i : i;
    switch (shape) {
      case "point":
      case "matrix":
        leds.push({ x: 0, y: 0, w, h });
        break;
      case "strip":
        leds.push(
          horizontal
            ? { x: -w / 2 + ((slot + 0.5) * w) / count, y: 0, w: w / count, h }
            : { x: 0, y: -h / 2 + ((slot + 0.5) * h) / count, w, h: h / count },
        );
        break;
      case "ring": {
        const size = Math.min(perimeterOfEllipse / count, Math.min(w, h));
        const t = (2 * Math.PI * slot) / count;
        leds.push({ x: a * Math.sin(t), y: -b * Math.cos(t), w: size, h: size });
        break;
      }
      case "outline": {
        const size = Math.min(perimeterOfRect / count, Math.min(w, h));
        const d = (perimeterOfRect * slot) / count;
        let x: number;
        let y: number;
        if (d < w) {
          x = -a + d;
          y = -b;
        } else if (d < w + h) {
          x = a;
          y = -b + (d - w);
        } else if (d < 2 * w + h) {
          x = a - (d - w - h);
          y = b;
        } else {
          x = -a;
          y = b - (d - 2 * w - h);
        }
        leds.push({ x, y, w: size, h: size });
        break;
      }
      case "grid": {
        const cw = w / cols;
        const ch = h / rows;
        leds.push({ x: -a + ((slot % cols) + 0.5) * cw, y: -b + (Math.floor(slot / cols) + 0.5) * ch, w: cw, h: ch });
        break;
      }
    }
  }
  // The dot's size: about a fifth of the gap between neighbours, and no more than a seventh of
  // the zone's cross-section, so a strip reads as a line of pips and never as a stack of lamps.
  let radius: number;
  switch (shape) {
    case "strip":
      radius = Math.min(((horizontal ? w : h) / count) * 0.22, (horizontal ? h : w) * 0.14);
      break;
    case "ring":
      radius = Math.min((perimeterOfEllipse / count) * 0.22, Math.min(w, h) * 0.1);
      break;
    case "outline":
      radius = Math.min((perimeterOfRect / count) * 0.22, Math.min(w, h) * 0.1);
      break;
    case "grid":
      radius = Math.min(w / cols, h / rows) * 0.22;
      break;
    default:
      radius = Math.min(w, h) * 0.22;
  }
  return { shape, leds, radius: Math.max(0.05, radius) };
}

/** A cell on the canvas: where an LED samples. */
export interface CanvasCell {
  cx: number;
  cy: number;
  /** x0, y0, x1, y1 of the cell's rotated corners. */
  bounds: [number, number, number, number];
}

/** An LED's cell on the canvas: the zone's transform applied to a local cell. */
export function canvasCell(t: ZoneTransform, led: LocalLed): CanvasCell {
  const hw = led.w / 2;
  const hh = led.h / 2;
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const [dx, dy] of [
    [-hw, -hh],
    [hw, -hh],
    [hw, hh],
    [-hw, hh],
  ] as const) {
    const lx = led.x + dx;
    const ly = led.y + dy;
    const x = t.cx + lx * t.cos - ly * t.sin;
    const y = t.cy + lx * t.sin + ly * t.cos;
    if (x < x0) x0 = x;
    if (x > x1) x1 = x;
    if (y < y0) y0 = y;
    if (y > y1) y1 = y;
  }
  return {
    cx: t.cx + led.x * t.cos - led.y * t.sin,
    cy: t.cy + led.x * t.sin + led.y * t.cos,
    bounds: [x0, y0, x1, y1],
  };
}

/** The LED positions of a zone on the canvas (a gap in a matrix map is left out). */
export function ledPositions(z: PlacedZone, info: ZoneInfo): Point[] {
  const t = zoneTransform(z);
  const out: Point[] = [];
  for (const led of zoneLocalLeds(z, info).leds) {
    if (!led) continue;
    const c = canvasCell(t, led);
    out.push({ x: c.cx, y: c.cy });
  }
  return out;
}

/**
 * The zone's corners on the canvas, clockwise from the top left: top-left, top-right,
 * bottom-right, bottom-left. The desk canvas puts the resize grip on the bottom-right one.
 */
export function zoneCorners(z: PlacedZone): Point[] {
  const t = zoneTransform(z);
  const hw = z.width / 2;
  const hh = z.height / 2;
  const local: Point[] = [
    { x: -hw, y: -hh },
    { x: hw, y: -hh },
    { x: hw, y: hh },
    { x: -hw, y: hh },
  ];
  return local.map((p) => ({ x: t.cx + p.x * t.cos - p.y * t.sin, y: t.cy + p.x * t.sin + p.y * t.cos }));
}

/**
 * The point above the zone's middle of its top edge, in the zone's own frame turned by the
 * rotation: the place the rotation grip sits. `gap` moves it further out along the same line.
 */
export function zoneGripPoint(z: PlacedZone, gap: number): Point {
  const t = zoneTransform(z);
  const lx = 0;
  const ly = -z.height / 2 - gap;
  return { x: t.cx + lx * t.cos - ly * t.sin, y: t.cy + lx * t.sin + ly * t.cos };
}

/**
 * The shape a zone gets when it is first placed: fans are rings, RAM and ARGB headers are strips,
 * a single-LED peripheral is a point and a matrix keeps its map. The first run uses the same
 * rules (`default_shape` in `crates/canvas-daemon/src/first_run.rs`).
 */
export function defaultShape(deviceKind: DeviceKind, info: ZoneInfo): ZoneShape {
  if (info.kind.type === "matrix") return { type: "auto" };
  if (info.name.toLowerCase().includes("fan") && info.led_count > 1) return { type: "ring" };
  if (info.kind.type === "single" || info.led_count === 1) return { type: "point" };
  if (deviceKind === "keyboard") return { type: "grid", columns: Math.max(1, Math.ceil(info.led_count / 6)) };
  return { type: "strip" };
}

/** Logical size of a zone when it is first placed. */
export function defaultRect(
  info: ZoneInfo,
  deviceKind: DeviceKind = "other",
  shape?: ZoneShape,
): { width: number; height: number } {
  const s = shape ?? defaultShape(deviceKind, info);
  if (s.type === "ring") return { width: 40, height: 40 };
  if (deviceKind === "dram") return { width: 16, height: 120 };
  if (info.kind.type === "matrix") {
    const aspect = info.kind.width / Math.max(1, info.kind.height);
    const width = 80 * Math.min(1.6, Math.max(0.6, aspect));
    return { width, height: width / Math.max(0.1, aspect) };
  }
  if (deviceKind === "keyboard") return s.type === "grid" ? { width: 120, height: 38 } : { width: 110, height: 30 };
  if (s.type === "point") {
    if (deviceKind === "mouse") return { width: 22, height: 32 };
    if (deviceKind === "microphone") return { width: 16, height: 28 };
    if (deviceKind === "headset") return { width: 30, height: 30 };
    return { width: 20, height: 20 };
  }
  return { width: 120, height: 10 };
}

/**
 * The box a zone takes when its shape changes, so the new shape starts out looking right: a ring
 * wants a square, a strip wants a long thin box. A box that already suits the shape is kept, and
 * the centre never moves.
 */
export function reshapeBox(z: PlacedZone, shape: ZoneShape): Partial<PlacedZone> {
  const { width: w, height: h } = z;
  const ratio = Math.max(w, h) / Math.max(0.01, Math.min(w, h));
  let nw = w;
  let nh = h;
  if (shape.type === "ring" && ratio > 1.5) {
    nw = nh = Math.min(48, Math.max(24, Math.max(w, h) * 0.6));
  } else if (shape.type === "strip" && ratio < 3) {
    nw = Math.min(140, Math.max(60, Math.max(w, h) * 2.5));
    nh = 10;
  }
  if (nw === w && nh === h) return { shape };
  const cx = z.x + w / 2;
  const cy = z.y + h / 2;
  return {
    shape,
    width: nw,
    height: nh,
    x: Math.min(Math.max(0, cx - nw / 2), CANVAS_W - nw),
    y: Math.min(Math.max(0, cy - nh / 2), CANVAS_H - nh),
  };
}

/**
 * The name of a placement: the owner's own name, or the plain name of the zone. When one zone
 * is several parts (three fans daisy-chained on one header), the parts are numbered, so the
 * reader can tell them apart: "Front fan 1", "Front fan 2", "Front fan 3".
 */
export function placementName(
  devices: DeviceInfo[] | null,
  zone: PlacedZone,
  all: PlacedZone[],
): string {
  const own = zone.label?.trim();
  if (own) return own;
  const plain = plainName(devices, zone.device_id, zone.zone_index).name;
  const siblings = all
    .filter((z) => z.device_id === zone.device_id && z.zone_index === zone.zone_index)
    .sort((a, b) => (a.leds?.start ?? 0) - (b.leds?.start ?? 0));
  if (siblings.length < 2) return plain;
  const at = siblings.findIndex(
    (z) => z === zone || (z.leds?.start === zone.leds?.start && z.leds?.count === zone.leds?.count),
  );
  return `${plain} ${at + 1}`;
}

/** What a zone is called to a person: the plain name and, muted beside it, the hardware's own. */
export interface PlainName {
  name: string;
  hardware: string | null;
  kind: DeviceKind | null;
  /** The zone is a fan (a ring of LEDs round a hub). */
  fan: boolean;
}

/**
 * The plain name of a zone, the one a person would use: "RAM 2", "ARGB header 1", "Keyboard",
 * "Front fan". The hardware's own name ("ENE DRAM", "JARGB 1") goes second and muted. Nothing is
 * invented: a zone the table does not know keeps its own name.
 */
export function plainName(devices: DeviceInfo[] | null, deviceId: string, zoneIndex: number): PlainName {
  const found = findZoneInfo(devices, deviceId, zoneIndex);
  if (!found) return { name: "Not connected", hardware: null, kind: null, fan: false };
  const { device, zone } = found;
  const zn = zone.name.trim();
  const fan = zn.toLowerCase().includes("fan") && zone.led_count > 1;
  const header = /^(JARGB|JRGB|JRAINBOW|JAF|JCORSAIR|JPUMP)\s*(\d*)$/i.exec(zn);
  const hw = (s: string): string | null => (s && s !== zn ? s : null);
  switch (device.kind) {
    case "dram": {
      const sticks = (devices ?? []).filter((d) => d.kind === "dram");
      const n = sticks.findIndex((d) => d.id === device.id) + 1;
      return { name: `RAM ${n}`, hardware: hw(device.name), kind: "dram", fan: false };
    }
    case "keyboard":
      return { name: "Keyboard", hardware: hw(device.name), kind: "keyboard", fan: false };
    case "mouse":
      return { name: "Mouse", hardware: hw(device.name), kind: "mouse", fan: false };
    case "microphone":
      return { name: "Microphone", hardware: hw(device.name), kind: "microphone", fan: false };
    case "headset":
      return { name: "Headset", hardware: hw(device.name), kind: "headset", fan: false };
    case "gpu":
      return { name: "Graphics card", hardware: hw(zn || device.name), kind: "gpu", fan: false };
    default:
      break;
  }
  if (fan) return { name: zn, hardware: hw(device.name), kind: device.kind, fan: true };
  if (header) {
    const word = { jargb: "ARGB header", jrgb: "RGB header", jrainbow: "Rainbow header", jaf: "Fan header", jcorsair: "Corsair header", jpump: "Pump header" }[
      header[1]!.toLowerCase()
    ] as string;
    return { name: header[2] ? `${word} ${header[2]}` : word, hardware: zn, kind: device.kind, fan: false };
  }
  return { name: zn || device.name, hardware: hw(device.name), kind: device.kind, fan: false };
}
