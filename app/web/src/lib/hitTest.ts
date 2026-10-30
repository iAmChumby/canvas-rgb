// What is under the pointer on the layout desk, worked out from the parts' own geometry.
//
// The desk used to ask the browser (`document.elementsFromPoint`), which skips anything with
// `pointer-events: none` and answers in DOM order. That is how an offline zone, a part added
// before the case, or a part at the case's edge sent a press to the wrong thing. This file is
// the whole answer, as a pure function of the geometry, so it can be tested without a browser.
//
// The rule, in order:
//   1. a handle of the picked part (one part picked, not locked);
//   2. the smallest part that is not a case (zones and landmarks alike; a tie goes to a zone,
//      then to the later one);
//   3. a case, by its edge band;
//   4. a case, by its body;
//   5. the empty desk.
// Offline, hidden-light and locked parts are all hit targets: the caller decides what a press
// on a locked one may do. With `under` set (Alt-click) the answer is the next one down the
// stack at that point instead of the first.
//
// All lengths are canvas units except those ending in `Px`, which are css pixels; `k` is css
// pixels per canvas unit.

import type { Point } from "@/lib/zones";

/** A part of the desk, by kind and place in its list. Structurally the editor's `Sel`. */
export interface PartRef {
  kind: "zone" | "landmark";
  index: number;
}

/** The geometry the hit-test needs from a zone or a landmark. */
export interface HitPart {
  ref: PartRef;
  x: number;
  y: number;
  width: number;
  height: number;
  rotation_deg: number;
  /** The case is the part everything else sits in: it loses to any part under the pointer. */
  isCase: boolean;
  locked: boolean;
  /** Whether the part's device is gone. It changes nothing here: an offline part is a part. */
  offline?: boolean;
}

export type HandleId = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

/**
 * The eight resize handles, as Figma lays them out: four corners and four edges. `sx` and `sy`
 * say which way the handle pulls the box (-1 the left or top edge, 1 the right or bottom edge, 0
 * the axis it leaves alone).
 */
export interface HandleDef {
  id: HandleId;
  sx: -1 | 0 | 1;
  sy: -1 | 0 | 1;
}

export const HANDLES: HandleDef[] = [
  { id: "nw", sx: -1, sy: -1 },
  { id: "n", sx: 0, sy: -1 },
  { id: "ne", sx: 1, sy: -1 },
  { id: "e", sx: 1, sy: 0 },
  { id: "se", sx: 1, sy: 1 },
  { id: "s", sx: 0, sy: 1 },
  { id: "sw", sx: -1, sy: 1 },
  { id: "w", sx: -1, sy: 0 },
];

/**
 * A part's edge handle is hidden when the edge is shorter than this, in css pixels: on a tiny
 * part the eight handles would sit on top of each other, so only the corners stay.
 */
export const EDGE_HANDLE_MIN_PX = 44;
/** How far above the top edge the rotation grip sits, in css pixels (the frame's stem). */
export const ROTATE_GRIP_PX = 24;
/** The width of the case's edge band, in css pixels. */
export const EDGE_BAND_PX = 8;
/** A part is never a smaller target than this, in css pixels, however thin it is drawn. */
const MIN_TARGET_PX = 12;

export type PointerKind = "mouse" | "pen" | "touch";

/** How far from a handle's centre a press still counts as on it, in css pixels. */
function handleReachPx(pointer: PointerKind): number {
  return pointer === "touch" ? 12 : 8;
}

/** How far the pointer travels, in css pixels, before a press is a drag and not a click. */
export function dragThresholdPx(pointer: PointerKind = "mouse"): number {
  return pointer === "touch" ? 8 : 4;
}

export interface HitInput {
  /** The pointer, in canvas units. */
  point: Point;
  /** Every part that can be hit, in its live geometry. */
  parts: HitPart[];
  /** What is picked. Handles are offered for a selection of exactly one. */
  selection: PartRef[];
  /** Css pixels per canvas unit. */
  k: number;
  pointer?: PointerKind;
  /** Alt-click: take the next part down from the picked one instead of the first. */
  under?: boolean;
}

export interface PartHit {
  kind: "part" | "case-edge" | "case-body";
  ref: PartRef;
  locked: boolean;
}

export type HitTarget =
  | PartHit
  | { kind: "handle"; ref: PartRef; handle: HandleDef | "rotate" }
  | { kind: "desk" };

const same = (a: PartRef, b: PartRef): boolean => a.kind === b.kind && a.index === b.index;

/** The point in the part's own frame: origin at its middle, axes along its sides. */
function toLocal(part: HitPart, p: Point): Point {
  const rad = (part.rotation_deg * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const dx = p.x - (part.x + part.width / 2);
  const dy = p.y - (part.y + part.height / 2);
  return { x: dx * cos + dy * sin, y: -dx * sin + dy * cos };
}

/** The four corners of a part on the desk, turned. */
export function partCorners(part: Pick<HitPart, "x" | "y" | "width" | "height" | "rotation_deg">): Point[] {
  const rad = (part.rotation_deg * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const cx = part.x + part.width / 2;
  const cy = part.y + part.height / 2;
  const hw = part.width / 2;
  const hh = part.height / 2;
  return (
    [
      [-hw, -hh],
      [hw, -hh],
      [hw, hh],
      [-hw, hh],
    ] as const
  ).map(([lx, ly]) => ({ x: cx + lx * cos - ly * sin, y: cy + lx * sin + ly * cos }));
}

/** The handle under the pointer, if one part is picked and free to be reshaped. */
function handleAt(input: HitInput): HitTarget | null {
  if (input.selection.length !== 1) return null;
  const ref = input.selection[0] as PartRef;
  const part = input.parts.find((p) => same(p.ref, ref));
  if (!part || part.locked) return null;
  const q = toLocal(part, input.point);
  const reach = handleReachPx(input.pointer ?? "mouse") / input.k;
  const narrowX = part.width * input.k < EDGE_HANDLE_MIN_PX;
  const narrowY = part.height * input.k < EDGE_HANDLE_MIN_PX;
  let best: { target: HitTarget; d: number } | null = null;
  const offer = (target: HitTarget, lx: number, ly: number) => {
    const ex = Math.abs(q.x - lx);
    const ey = Math.abs(q.y - ly);
    if (ex > reach || ey > reach) return;
    const d = Math.hypot(ex, ey);
    if (!best || d < best.d) best = { target, d };
  };
  for (const h of HANDLES) {
    if (h.sx === 0 && narrowX) continue;
    if (h.sy === 0 && narrowY) continue;
    offer({ kind: "handle", ref, handle: h }, (h.sx * part.width) / 2, (h.sy * part.height) / 2);
  }
  offer({ kind: "handle", ref, handle: "rotate" }, 0, -part.height / 2 - ROTATE_GRIP_PX / input.k);
  return (best as { target: HitTarget; d: number } | null)?.target ?? null;
}

/** True when the point is on the part's body. A thin part is a little wider than it is drawn. */
function onBody(part: HitPart, p: Point, k: number): boolean {
  const q = toLocal(part, p);
  const pad = part.isCase ? 0 : MIN_TARGET_PX / 2 / k;
  const hw = Math.max(part.width / 2, pad);
  const hh = Math.max(part.height / 2, pad);
  return Math.abs(q.x) <= hw && Math.abs(q.y) <= hh;
}

/** True when the point is inside the part and within `bandPx` of its outline. */
function onEdgeBand(part: HitPart, p: Point, k: number, bandPx: number): boolean {
  const q = toLocal(part, p);
  const band = bandPx / k;
  return part.width / 2 - Math.abs(q.x) < band || part.height / 2 - Math.abs(q.y) < band;
}

const area = (p: HitPart): number => p.width * p.height;

/**
 * Everything under the point, top of the stack first, without the handles: the smallest part,
 * the next smallest, ..., then the case that holds the point by its edge band, then the cases
 * that hold it by their body. Each part appears once.
 */
export function hitStack(input: HitInput): PartHit[] {
  const { parts, point, k } = input;
  const hits = parts.filter((p) => onBody(p, point, k));
  const plain = hits
    .filter((p) => !p.isCase)
    .sort(
      (a, b) =>
        area(a) - area(b) ||
        (a.ref.kind === b.ref.kind ? 0 : a.ref.kind === "zone" ? -1 : 1) ||
        b.ref.index - a.ref.index,
    );
  const cases = hits
    .filter((p) => p.isCase)
    .sort((a, b) => area(a) - area(b) || b.ref.index - a.ref.index);
  const edges = cases.filter((c) => onEdgeBand(c, point, k, EDGE_BAND_PX));
  const bodies = cases.filter((c) => !edges.includes(c));
  return [
    ...plain.map((p): PartHit => ({ kind: "part", ref: p.ref, locked: p.locked })),
    ...edges.map((p): PartHit => ({ kind: "case-edge", ref: p.ref, locked: p.locked })),
    ...bodies.map((p): PartHit => ({ kind: "case-body", ref: p.ref, locked: p.locked })),
  ];
}

/** What a press at the point lands on. */
export function hitTest(input: HitInput): HitTarget {
  if (!input.under) {
    const handle = handleAt(input);
    if (handle) return handle;
  }
  const stack = hitStack(input);
  const first = stack[0];
  if (!first) return { kind: "desk" };
  if (!input.under) return first;
  // The next part below the one that is picked and under the pointer; when the picked part is
  // not under the pointer (or nothing is picked), the top one.
  const picked = input.selection[0];
  const at = picked ? stack.findIndex((t) => same(t.ref, picked)) : -1;
  if (at >= 0) return stack[(at + 1) % stack.length] as PartHit;
  return first;
}

/**
 * The parts a case carries when it moves: every unlocked zone and landmark whose middle is inside
 * the case's own (turned) box. The case itself, any other case and locked parts are not carried.
 */
export function partsInsideCase(box: HitPart, parts: HitPart[]): PartRef[] {
  const out: PartRef[] = [];
  for (const p of parts) {
    // A locked part stays where it is, whatever moves around it.
    if (p.isCase || p.locked || same(p.ref, box.ref)) continue;
    const mid = { x: p.x + p.width / 2, y: p.y + p.height / 2 };
    const q = toLocal(box, mid);
    if (Math.abs(q.x) <= box.width / 2 && Math.abs(q.y) <= box.height / 2) out.push(p.ref);
  }
  return out;
}

/** A rectangle on the desk, in canvas units. */
export interface DeskRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The axis-aligned box of a turned part. */
function aabb(part: HitPart): DeskRect {
  const c = partCorners(part);
  const xs = c.map((p) => p.x);
  const ys = c.map((p) => p.y);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y };
}

/**
 * What a rubber band picks: every zone and landmark it touches. A case is only picked when the
 * band holds all of it, so a band drawn across the parts in a case does not take the case too.
 */
export function marqueePick(band: DeskRect, parts: HitPart[]): PartRef[] {
  const out: PartRef[] = [];
  for (const p of parts) {
    const b = aabb(p);
    const touches =
      b.x < band.x + band.width && band.x < b.x + b.width && b.y < band.y + band.height && band.y < b.y + b.height;
    if (!touches) continue;
    if (p.isCase) {
      const holds =
        b.x >= band.x &&
        b.y >= band.y &&
        b.x + b.width <= band.x + band.width &&
        b.y + b.height <= band.y + band.height;
      if (!holds) continue;
    }
    out.push(p.ref);
  }
  return out;
}
