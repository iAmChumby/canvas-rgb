// Zone geometry on the 320 x 200 logical canvas: where each LED of a placed zone sits.
// The zone's `shape` decides it (strip, ring, outline, point, grid; `auto` follows the device's
// zone kind), and `reversed` flips the order. Rotation is clockwise about the rect's centre, as
// CSS rotate() draws it and as the daemon samples it.
//
// The desk canvas draws every pointer move without a React render, so the geometry is split in
// two: `zoneLedOffsets` gives the LED positions in the zone's own frame (they only change when
// the zone is resized or reversed) and `zoneTransform` gives the frame's place and angle.

import type { PlacedZone } from "@/proto/PlacedZone";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DeviceKind } from "@/proto/DeviceKind";
import type { ZoneShape } from "@/proto/ZoneShape";

const CANVAS_W = 320;
const CANVAS_H = 200;

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
 */
export function zoneLocalLeds(z: PlacedZone, info: ZoneInfo): LocalLeds {
  const shape = resolveShape(z.shape, info.kind);
  const n = Math.max(0, info.led_count);
  const w = z.width;
  const h = z.height;
  const leds: (LocalLed | null)[] = [];
  if (n === 0) return { shape, leds, radius: 1 };
  const kind = info.kind;
  if (shape === "matrix" && kind.type === "matrix") {
    const cw = w / kind.width;
    const ch = h / kind.height;
    for (let i = 0; i < n; i++) leds.push(null);
    for (let r = 0; r < kind.height; r++) {
      for (let c = 0; c < kind.width; c++) {
        const idx = kind.map[r * kind.width + c];
        if (idx === null || idx === undefined || idx >= n) continue;
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
  const cols = z.shape.type === "grid" ? Math.min(Math.max(1, z.shape.columns), n) : 1;
  const rows = Math.ceil(n / cols);
  for (let i = 0; i < n; i++) {
    const slot = z.reversed ? n - 1 - i : i;
    switch (shape) {
      case "point":
      case "matrix":
        leds.push({ x: 0, y: 0, w, h });
        break;
      case "strip":
        leds.push(
          horizontal
            ? { x: -w / 2 + ((slot + 0.5) * w) / n, y: 0, w: w / n, h }
            : { x: 0, y: -h / 2 + ((slot + 0.5) * h) / n, w, h: h / n },
        );
        break;
      case "ring": {
        const size = Math.min(perimeterOfEllipse / n, Math.min(w, h));
        const t = (2 * Math.PI * slot) / n;
        leds.push({ x: a * Math.sin(t), y: -b * Math.cos(t), w: size, h: size });
        break;
      }
      case "outline": {
        const size = Math.min(perimeterOfRect / n, Math.min(w, h));
        const d = (perimeterOfRect * slot) / n;
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
      radius = Math.min(((horizontal ? w : h) / n) * 0.22, (horizontal ? h : w) * 0.14);
      break;
    case "ring":
      radius = Math.min((perimeterOfEllipse / n) * 0.22, Math.min(w, h) * 0.1);
      break;
    case "outline":
      radius = Math.min((perimeterOfRect / n) * 0.22, Math.min(w, h) * 0.1);
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
