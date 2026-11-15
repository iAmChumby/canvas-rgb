// The dots of the pattern editor: where each LED of a device is drawn, which dot a press lands
// on, and which dots a drag crossed. Pure, so every rule is settled without a canvas.
//
// The shapes follow the canvas path's own rules (`lib/arrangement.ts`): a ring runs clockwise from
// the top, a strip runs left to right, a grid fills its cells and a single LED is one dot. The
// dots are in CSS pixels of the surface's box, because they are what the pointer is measured
// against; the drawing uses the same numbers, so the LED under the pointer is the LED painted.
//
// Large devices: a strip longer than the box holds wraps into rows, a grid keeps its cell count
// and a ring keeps its circle. The radius shrinks with the count to a 3 px floor, and past that
// floor (a strip of 4096 LEDs in a phone's box) it keeps shrinking rather than dropping a dot off
// the device: a dot the owner cannot see or reach is one they cannot paint.

import type { BuiltOn } from "@/proto/BuiltOn";
import type { GridSize } from "@/proto/GridSize";
// The canvas path's own inset, reused so the dot ring and the sampled ring sit alike.
import { RING_INSET } from "@/lib/arrangement";

export interface Dot {
  /** The LED's place in the device's frame, in position order. */
  index: number;
  /** Centre, in CSS pixels from the surface's top-left. */
  x: number;
  y: number;
  /** Radius, in CSS pixels. */
  r: number;
}

/** The smallest dot: the plan's 3 CSS px floor. */
export const MIN_R = 3;
/** The largest dot: an LED of a small device, drawn large. */
export const MAX_R = 20;
/**
 * How far past a dot's edge a press still counts, in CSS pixels.
 *
 * It is a press's tolerance and not a layout value: dots packed closer than the slop make a
 * stroke take their neighbours too, which is what the finger did.
 */
export const SLOP = 4;

/** The gap between two dots, in CSS pixels. */
const GAP = 2;

/**
 * Every LED of a device, placed in a box `width` × `height` CSS pixels: clockwise from the top
 * for a ring, left to right for a strip (which wraps into rows when the box cannot hold it in
 * one), the grid's own cells for a grid, and one centred dot for a single LED.
 *
 * A box with no room in it (a surface that has not been measured yet) has no dots.
 */
export function layoutDots(builtOn: BuiltOn, width: number, height: number): Dot[] {
  const w = Math.max(0, width);
  const h = Math.max(0, height);
  if (w <= 0 || h <= 0) return [];
  const n = Math.max(1, Math.floor(builtOn.led_count));
  switch (builtOn.arrangement) {
    case "ring":
      return ringDots(n, w, h);
    case "grid":
      // A grid the daemon reported without a cell map, or with fewer cells than LEDs, reads by
      // LED order: the hardware cannot tell a grid from a line there either.
      return gridDots(builtOn.grid, n, w, h) ?? stripDots(n, w, h);
    case "single":
      return singleDots(w, h);
    default:
      return stripDots(n, w, h);
  }
}

/**
 * The dot under a point, or null when the point is in a gap between dots. The nearest dot wins,
 * so overlapping dots of a dense device still resolve to exactly one.
 */
export function dotAt(dots: Dot[], x: number, y: number): number | null {
  let best: number | null = null;
  let bestDistance = Infinity;
  for (const dot of dots) {
    const distance = Math.hypot(dot.x - x, dot.y - y);
    if (distance <= dot.r + SLOP && distance < bestDistance) {
      bestDistance = distance;
      best = dot.index;
    }
  }
  return best;
}

/**
 * The dots a drag crossed from `a` to `b`, in LED order.
 *
 * Every dot is tested against the segment, rather than the segment being sampled at a step: a
 * sampled step can still jump over a dot smaller than the step, and a fast drag on a long strip
 * is exactly where that would skip one. The pointer's own trail is not sampled, so a stroke that
 * arrives with two events far apart still takes every LED between them.
 */
export function dotsBetween(
  dots: Dot[],
  a: { x: number; y: number },
  b: { x: number; y: number },
): number[] {
  const hit: number[] = [];
  for (const dot of dots) {
    if (distanceToSegment(dot, a, b) <= dot.r + SLOP) hit.push(dot.index);
  }
  return hit;
}

/** A ring: clockwise from the top, as `ringSamples` in `lib/arrangement.ts` draws it. */
function ringDots(n: number, w: number, h: number): Dot[] {
  const half = Math.min(w, h) / 2;
  // The circle is first sized for the biggest dot; the dot is then sized from the spacing along
  // that circle, so a ring of 12 gets LEDs as chunky as the box allows and a ring of 512 gets
  // small ones.
  const nominal = Math.max(0, half - MAX_R - RING_INSET);
  const spacing = (2 * Math.PI * nominal) / n;
  const r = clamp(spacing * 0.38, MIN_R, MAX_R);
  const radius = Math.max(0, half - r - RING_INSET);
  const cx = w / 2;
  const cy = h / 2;
  const dots: Dot[] = [];
  for (let j = 0; j < n; j++) {
    // -90 degrees is the top of the circle; the angle grows clockwise (y grows downward).
    const theta = ((-90 + (360 * j) / n) * Math.PI) / 180;
    dots.push({ index: j, x: cx + radius * Math.cos(theta), y: cy + radius * Math.sin(theta), r });
  }
  return dots;
}

/** A strip: one row across the middle, or rows of a wrapped strip. */
function stripDots(n: number, w: number, h: number): Dot[] {
  const step = w / n;
  if (step >= 2 * MIN_R + GAP) {
    const r = Math.min(
      Math.min(MAX_R, Math.max(MIN_R, step * 0.4)),
      Math.max(0.5, h / 2 - 1),
    );
    const y = h / 2;
    return Array.from({ length: n }, (_, j) => ({ index: j, x: (j + 0.5) * step, y, r }));
  }
  // Wrapped: as many LEDs per row as the box holds at the smallest dot, then rows of that.
  const perRow = Math.max(1, Math.floor((w + GAP) / (2 * MIN_R + GAP)));
  const rows = Math.ceil(n / perRow);
  const cellW = w / perRow;
  const cellH = h / rows;
  const r = Math.min(
    Math.min(MAX_R, Math.max(MIN_R, (cellW - GAP) / 2)),
    Math.max(0.5, (cellH - GAP) / 2),
  );
  const rowGap = Math.min(cellH, 2 * r + GAP);
  const top = (h - rows * rowGap) / 2 + rowGap / 2;
  return Array.from({ length: n }, (_, j) => ({
    index: j,
    x: ((j % perRow) + 0.5) * cellW,
    y: top + Math.floor(j / perRow) * rowGap,
    r,
  }));
}

/** A grid: the pattern's own cells, filled row by row. */
function gridDots(grid: GridSize | undefined, n: number, w: number, h: number): Dot[] | null {
  if (!grid) return null;
  const cols = Math.floor(grid.width);
  const rows = Math.floor(grid.height);
  if (cols <= 0 || rows <= 0 || cols * rows < n) return null;
  const cellW = w / cols;
  const cellH = h / rows;
  const r = Math.max(
    0.5,
    Math.min(MAX_R, Math.max(MIN_R, Math.min(cellW, cellH) * 0.32), Math.min(cellW, cellH) / 2 - 0.5),
  );
  return Array.from({ length: n }, (_, j) => ({
    index: j,
    x: ((j % cols) + 0.5) * cellW,
    y: (Math.floor(j / cols) + 0.5) * cellH,
    r,
  }));
}

/** One LED: one dot in the middle of the box. */
function singleDots(w: number, h: number): Dot[] {
  const r = Math.min(56, Math.max(MIN_R, Math.min(w, h) * 0.16));
  return [{ index: 0, x: w / 2, y: h / 2, r }];
}

/** How far a point is from a segment, in CSS pixels. */
function distanceToSegment(
  point: { x: number; y: number },
  a: { x: number; y: number },
  b: { x: number; y: number },
): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const length = dx * dx + dy * dy;
  if (length === 0) return Math.hypot(point.x - a.x, point.y - a.y);
  const t = Math.max(0, Math.min(1, ((point.x - a.x) * dx + (point.y - a.y) * dy) / length));
  return Math.hypot(point.x - (a.x + t * dx), point.y - (a.y + t * dy));
}

/** Keeps `value` between `min` and `max`. */
function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
