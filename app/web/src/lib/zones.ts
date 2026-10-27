// Zone geometry on the 320 x 200 logical canvas: where each LED of a placed zone sits.
// Linear zones run along the rect's long axis, left to right (or top to bottom) before rotation,
// and `reversed` flips them. Matrix zones fill the rect as their grid. Rotation is clockwise
// about the rect's centre, as CSS rotate() draws it.
//
// The desk canvas draws every pointer move without a React render, so the geometry is split in
// two: `zoneLedOffsets` gives the LED positions in the zone's own frame (they only change when
// the zone is resized or reversed) and `zoneTransform` gives the frame's place and angle.

import type { PlacedZone } from "@/proto/PlacedZone";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { DeviceInfo } from "@/proto/DeviceInfo";

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

/**
 * The LED positions of a zone in the zone's own frame: the origin is the centre of the rect, the
 * x axis runs along the zone's long side. `reversed` flips them. The result does not depend on
 * where the zone sits on the canvas, so the desk canvas can cache it.
 */
export function zoneLedOffsets(info: ZoneInfo, reversed: boolean): Point[] {
  const kind = info.kind;
  if (kind.type === "single") {
    return [{ x: 0, y: 0 }];
  }
  if (kind.type === "linear") {
    const n = Math.max(0, info.led_count);
    const out: Point[] = [];
    for (let i = 0; i < n; i++) {
      let t = (i + 0.5) / n;
      if (reversed) t = 1 - t;
      out.push({ x: t - 0.5, y: 0 });
    }
    return out;
  }
  const cols = kind.width;
  const rows = kind.height;
  const out: Point[] = [];
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const idx = kind.map[r * cols + c];
      if (idx === null || idx === undefined) continue;
      let cx = (c + 0.5) / cols;
      if (reversed) cx = 1 - cx;
      out.push({ x: cx - 0.5, y: (r + 0.5) / rows - 0.5 });
    }
  }
  return out;
}

/** The LED positions of a zone on the canvas. */
export function ledPositions(z: PlacedZone, info: ZoneInfo): Point[] {
  const t = zoneTransform(z);
  const horizontal = z.width >= z.height;
  const linear = info.kind.type === "linear";
  const out: Point[] = [];
  for (const p of zoneLedOffsets(info, z.reversed)) {
    const lx = linear ? (horizontal ? p.x * z.width : 0) : p.x * z.width;
    const ly = linear ? (horizontal ? 0 : p.x * z.height) : p.y * z.height;
    out.push({ x: t.cx + lx * t.cos - ly * t.sin, y: t.cy + lx * t.sin + ly * t.cos });
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

/** Logical size of a zone when it is first placed. */
export function defaultRect(info: ZoneInfo): { width: number; height: number } {
  const kind = info.kind;
  if (kind.type === "matrix") {
    const aspect = kind.width / Math.max(1, kind.height);
    const width = 80 * Math.min(1.6, Math.max(0.6, aspect));
    return { width, height: width / Math.max(0.1, aspect) };
  }
  if (kind.type === "linear") {
    const width = Math.min(160, Math.max(24, info.led_count * 0.6));
    return { width, height: 8 };
  }
  return { width: 14, height: 14 };
}
