"use client";

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent,
  type PointerEvent,
} from "react";
import { ArrowsOut, Minus, Plus } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";
import { onFrame, PREVIEW_H, PREVIEW_W, type Frame } from "@/lib/preview";
import { findZoneInfo, zoneCorners, zoneGripPoint, zoneLedOffsets, zoneTransform, type Point } from "@/lib/zones";
import { clamp } from "@/lib/format";
import { usePrefersReducedMotion } from "@/lib/motion";
import { FIT_VIEW, forgetView, readView, writeView } from "@/lib/viewPrefs";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { PlacedZone } from "@/proto/PlacedZone";
import desk from "./desk.module.css";

const CANVAS_W = 320;
const CANVAS_H = 200;
const ACCENTS = ["--a1", "--a2", "--a3", "--a4", "--a5"] as const;

/** The grid a zone snaps to, in canvas units. */
const SNAP_GRID = 5;
/** How close, in screen pixels, a zone has to be to another zone's line to snap to it. */
const SNAP_PIXELS = 8;
/** How far the rotation grip sits above the zone's top edge, in canvas units. */
const GRIP_GAP = 7;
/** How far a chip may sit from its own zone and still count as sitting at it, in css pixels. */
const CHIP_GAP = 10;
/** The zoom range. 1 is the whole canvas in the stage. */
const MIN_SCALE = 0.25;
const MAX_SCALE = 8;
/** How long the view waits before it is written back to React, in ms. */
const VIEW_COMMIT_MS = 140;
/** How long the view eases to a new zoom level or back to the fit view, in ms. */
const ZOOM_MS = 180;
/** The chrome around a chip's text: the padding, the hairline and the accent edge, in css pixels. */
const CHIP_CHROME = 14;
/**
 * On a phone a chip may take at most this share of the canvas. A chip that fits keeps the LED
 * count ("DRAM 8"), a chip that does not keeps the name alone ("JARGB 1").
 */
const NARROW_BUDGET = 0.25;

type DragKind = "move" | "resize" | "rotate";
interface Drag {
  kind: DragKind;
  index: number;
  start: Point;
  orig: PlacedZone;
  /** Where the zone's label chip was when the drag began, as an offset from the zone's corner. */
  label: { dx: number; dy: number } | null;
}

/** The view: how the 320 x 200 canvas is drawn into the stage. It never touches the layout. */
interface View {
  scale: number;
  panX: number;
  panY: number;
}

interface LedCache {
  offsets: Point[];
  linear: boolean;
}

interface LabelBox {
  key: string;
  /** The zone the chip belongs to. */
  index: number;
  text: string;
  x: number;
  y: number;
  w: number;
  missing: boolean;
  /** The device's accent, so the chip carries the same colour as the zone it names. */
  accent: number;
}

/** The lines a zone offers to the guides: its two edges and its middle. */
function zoneLines(z: { x: number; y: number; width: number; height: number }, axis: "x" | "y"): number[] {
  const a = axis === "x" ? z.x : z.y;
  const b = axis === "x" ? z.width : z.height;
  return [a, a + b / 2, a + b];
}

interface Snap {
  x: number;
  y: number;
  /** The line the zone snapped to, in canvas units. */
  guideX: number | null;
  guideY: number | null;
  /** How far the zone is from the line it snapped to, in canvas units. */
  gapX: number | null;
  gapY: number | null;
}

/**
 * Where a moving zone lands: on the grid, or on another zone's edge or middle when it is close
 * enough. The alignment wins over the grid, and the lines it snapped to are the guides.
 */
function snapMove(
  moving: { x: number; y: number; width: number; height: number },
  others: PlacedZone[],
  threshold: number,
  free: boolean,
): Snap {
  if (free) return { x: moving.x, y: moving.y, guideX: null, guideY: null, gapX: null, gapY: null };
  let bestX: { delta: number; line: number } | null = null;
  let bestY: { delta: number; line: number } | null = null;
  for (const o of others) {
    for (const line of zoneLines(o, "x")) {
      for (const mine of zoneLines(moving, "x")) {
        const delta = line - mine;
        if (Math.abs(delta) <= threshold && (!bestX || Math.abs(delta) < Math.abs(bestX.delta))) {
          bestX = { delta, line };
        }
      }
    }
    for (const line of zoneLines(o, "y")) {
      for (const mine of zoneLines(moving, "y")) {
        const delta = line - mine;
        if (Math.abs(delta) <= threshold && (!bestY || Math.abs(delta) < Math.abs(bestY.delta))) {
          bestY = { delta, line };
        }
      }
    }
  }
  const grid = (v: number) => Math.round(v / SNAP_GRID) * SNAP_GRID;
  return {
    x: bestX ? moving.x + bestX.delta : grid(moving.x),
    y: bestY ? moving.y + bestY.delta : grid(moving.y),
    guideX: bestX?.line ?? null,
    guideY: bestY?.line ?? null,
    gapX: bestX ? Math.abs(bestX.delta) : null,
    gapY: bestY ? Math.abs(bestY.delta) : null,
  };
}

/**
 * The label chip's size at a zoom: the text grows with the canvas, between a floor (the 14 px
 * the taste brief sets as the smallest readable text) and a ceiling (a chip that would cover the
 * zone it names).
 */
function labelMetrics(scale: number): { h: number; font: number; char: number } {
  const font = clamp(Math.round(14 * scale), 14, 26);
  return { h: Math.round(font * 1.45), font, char: font * 0.6 };
}

/**
 * The house easing, cubic-bezier(0.2, 0.8, 0.2, 1), as a function of time. The zoom eases with
 * the same curve as every other move in the editor, so one number describes all of them.
 */
function houseEase(t: number): number {
  const cx = 0.2;
  const cy = 0.8;
  const bx = 0.2;
  const by = 1;
  // The curve's x is monotonic, so a few bisections find the parameter for the wanted time.
  let lo = 0;
  let hi = 1;
  let u = t;
  for (let i = 0; i < 18; i++) {
    u = (lo + hi) / 2;
    const x = 3 * (1 - u) * (1 - u) * u * cx + 3 * (1 - u) * u * u * bx + u * u * u;
    if (x < t) lo = u;
    else hi = u;
  }
  return 3 * (1 - u) * (1 - u) * u * cy + 3 * (1 - u) * u * u * by + u * u * u;
}

export interface StageProps {
  zones: PlacedZone[];
  devices: DeviceInfo[] | null;
  selected: number | null;
  onSelect: (index: number | null) => void;
  onChange: (index: number, patch: Partial<PlacedZone>) => void;
  onDrop: (deviceId: string, zoneIndex: number, x: number, y: number) => void;
  /** Told when a drag starts and ends, so the page can hold off replacing the layout. */
  onDraggingChange?: (dragging: boolean) => void;
  /**
   * Told the live geometry of the zone being dragged, at most once per animation frame, so the
   * inspector can follow the pointer without the drag itself re-rendering the canvas.
   */
  onLiveChange?: (index: number, zone: PlacedZone) => void;
  /** The layout the view belongs to: the zoom and the pan are remembered per layout. */
  layoutId?: string;
  dropPayloadType: string;
}

function zoneKey(z: PlacedZone): string {
  return `${z.device_id}#${z.zone_index}`;
}

/** The device accent of a zone, or -1 when the device is not in the list any more. */
function accentOf(devices: DeviceInfo[] | null, deviceId: string): number {
  const i = devices?.findIndex((d) => d.id === deviceId) ?? -1;
  return i < 0 ? -1 : i % ACCENTS.length;
}

/**
 * Places a label above every zone's top-left corner, in css pixels, and pushes labels apart when
 * two of them would land on the same line. The result never leaves the canvas.
 */
/** A rectangle on the canvas, in css pixels. */
interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** True when two boxes overlap. */
function overlaps(a: Box, b: Box): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/** How far two boxes are from each other, in css pixels: 0 when they touch or overlap. */
function boxGap(a: Box, b: Box): number {
  const gapX = Math.max(0, Math.max(a.x - (b.x + b.w), b.x - (a.x + a.w)));
  const gapY = Math.max(0, Math.max(a.y - (b.y + b.h), b.y - (a.y + a.h)));
  return Math.hypot(gapX, gapY);
}

/** True when two boxes are within `gap` pixels of each other. */
function near(a: Box, b: Box, gap: number): boolean {
  return boxGap(a, b) <= gap;
}

/** True when one box holds the other. */
function contains(outer: Box, inner: Box): boolean {
  return (
    outer.x <= inner.x + 1 &&
    outer.y <= inner.y + 1 &&
    outer.x + outer.w >= inner.x + inner.w - 1 &&
    outer.y + outer.h >= inner.y + inner.h - 1
  );
}

/** The distance between two boxes; 0 when they touch or overlap. */
function boxDistance(a: Box, b: Box): number {
  const dx = Math.max(0, Math.max(a.x - (b.x + b.w), b.x - (a.x + a.w)));
  const dy = Math.max(0, Math.max(a.y - (b.y + b.h), b.y - (a.y + a.h)));
  return Math.hypot(dx, dy);
}

/**
 * Places the label chips.
 *
 * A chip must sit against **its own** zone: above it, or inside it when there is no room above.
 * It may never cover another zone, or it reads as that zone's label, and it may never sit under
 * the zoom control or one of its tooltips, or it is a chip nobody can read. The chips are
 * therefore placed one at a time, each one taking the first place where it is inside the canvas,
 * clear of every other zone and clear of the controls, and the ones that cannot be clear of each
 * other are pushed down.
 */
function layoutLabels(
  zones: PlacedZone[],
  devices: DeviceInfo[] | null,
  stageW: number,
  stageH: number,
  view: View,
  obstacles: Box[],
): LabelBox[] {
  // On a phone the chip must not eat the canvas: the name alone, and a short word for a zone
  // whose device is gone. The zone list below spells both out. A chip that still fits in the
  // phone's budget keeps the LED count as well, so "DRAM 8" stays "DRAM 8".
  const narrow = stageW < 480;
  const base = stageW / CANVAS_W;
  const k = base * view.scale;
  const m = labelMetrics(view.scale);
  // The part of the canvas that is on screen: a chip for a zone that is off the canvas is
  // left out, or every chip would pile up on the edge when the canvas is zoomed in.
  const visX0 = Math.max(0, view.panX);
  const visX1 = Math.min(stageW, view.panX + stageW * view.scale);
  const visY0 = Math.max(0, view.panY);
  const visY1 = Math.min(stageH, view.panY + stageH * view.scale);

  // The zones that are on screen, with their boxes in css pixels.
  const items: {
    index: number;
    key: string;
    text: string;
    w: number;
    missing: boolean;
    accent: number;
    box: Box;
  }[] = [];
  zones.forEach((z, index) => {
    if (!z.visible) return;
    const info = findZoneInfo(devices, z.device_id, z.zone_index);
    // A zone is stale when the daemon does not drive its device: the device is not in the list
    // at all, or it is there and not connected.
    const stale = !info || !info.device.connected;
    const named = info ? info.zone.name : narrow ? "not connected" : "device not found";
    const counted = info ? `${info.zone.name} ${info.zone.led_count}` : named;
    const text =
      info && narrow && counted.length * m.char + CHIP_CHROME > stageW * NARROW_BUDGET ? named : counted;
    const corners = zoneCorners(z);
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const c of corners) {
      if (c.x < minX) minX = c.x;
      if (c.x > maxX) maxX = c.x;
      if (c.y < minY) minY = c.y;
      if (c.y > maxY) maxY = c.y;
    }
    const box: Box = {
      x: (minX + view.panX) * k,
      y: (minY + view.panY) * k,
      w: (maxX - minX) * k,
      h: (maxY - minY) * k,
    };
    if (box.x + box.w < visX0 || box.x > visX1 || box.y + box.h < visY0 || box.y > visY1) return;
    items.push({
      index,
      key: zoneKey(z),
      text,
      w: text.length * m.char + CHIP_CHROME,
      missing: stale,
      accent: accentOf(devices, z.device_id),
      box,
    });
  });

  const placed: LabelBox[] = [];
  for (const item of items) {
    const w = item.w;
    // The places a chip can go: above the zone, inside it, below it; each one either aligned
    // with the zone's left edge or with its right edge. When the zone is tall and narrow, the
    // chip can also stand beside it.
    const xs = [item.box.x, item.box.x + item.box.w - w];
    const ys = [item.box.y - m.h - 4, item.box.y + 3, item.box.y + item.box.h + 4];
    const candidates: Box[] = [];
    for (const y of ys) {
      for (const x of xs) {
        candidates.push({ x: clamp(x, 0, Math.max(0, stageW - w)), y, w, h: m.h });
      }
    }
    // Beside the zone: to its left or to its right, at the zone's head or at its foot.
    for (const y of [item.box.y, item.box.y + item.box.h - m.h]) {
      for (const x of [item.box.x - w - 4, item.box.x + item.box.w + 4]) {
        candidates.push({ x: clamp(x, 0, Math.max(0, stageW - w)), y, w, h: m.h });
      }
    }
    const inCanvas = (b: Box) => b.y >= 0 && b.y + m.h <= stageH && b.x >= 0 && b.x + b.w <= stageW;
    // The chip has to sit at its own zone: it must touch it, cover it, or sit right above it,
    // or it names nothing.
    const anchored = (b: Box) => near(b, item.box, CHIP_GAP);
    // The chip may not cover another zone: it would read as that zone's label. A zone that
    // holds this zone (or is the same box) is the exception: a chip on a small zone inside a
    // big one cannot help but be inside the big one too.
    const zoneClash = (b: Box) =>
      items.some(
        (o) => o.index !== item.index && overlaps(b, o.box) && !contains(o.box, item.box),
      );
    // And it may not cover another chip: that chip's label would be cut in half.
    const chipClash = (b: Box) => placed.some((p) => overlaps(b, { x: p.x, y: p.y, w: p.w, h: m.h }));
    // And it may not sit under the zoom control or one of its tooltips: that is where the
    // pointer goes, and a chip under it is a chip nobody can read.
    const controlClash = (b: Box) => obstacles.some((o) => overlaps(b, o));
    // And its own zone must be the nearest one: a chip that sits closer to another zone names
    // that one.
    const nearestIsOwn = (b: Box) => {
      const others = items.filter((o) => o.index !== item.index && !contains(o.box, item.box));
      let best = Infinity;
      for (const o of others) best = Math.min(best, boxDistance(b, o.box));
      return boxDistance(b, item.box) <= best + 0.001;
    };
    /**
     * How good a place is. The rules are weighed in the order they matter: a chip that names
     * its zone beats a chip that does not, a chip that does not cover another zone beats one
     * that does, and a chip that nobody can see under the control is the worst of all.
     */
    const score = (b: Box): number => {
      if (!inCanvas(b)) return -1000;
      let s = 0;
      s += anchored(b) ? 16 : -8;
      // A chip that is printed on its zone reads better than one that floats next to it.
      s += overlaps(b, item.box) ? 2 : 0;
      s += !zoneClash(b) ? 8 : -4;
      s += !chipClash(b) ? 4 : -2;
      s += !controlClash(b) ? 2 : -8;
      s += nearestIsOwn(b) ? 1 : 0;
      return s;
    };
    /**
     * Where a chip can go when another chip is in the way: to its side if there is room, above
     * it or under it if there is not. Two chips side by side read better than two chips
     * stacked, and a chip that moves sideways stays at its own zone.
     */
    const resolve = (start: Box): Box[] => {
      const out: Box[] = [];
      const queue: Box[] = [start];
      const seen = new Set<string>();
      while (queue.length > 0 && out.length < 8) {
        const b = queue.shift() as Box;
        const key = `${Math.round(b.x)},${Math.round(b.y)}`;
        if (seen.has(key)) continue;
        seen.add(key);
        const hit = placed.find((p) => overlaps(b, { x: p.x, y: p.y, w: p.w, h: m.h }));
        if (!hit) {
          out.push(b);
          continue;
        }
        queue.push({ ...b, x: clamp(hit.x + hit.w + 2, 0, Math.max(0, stageW - w)) });
        queue.push({ ...b, x: clamp(hit.x - w - 2, 0, Math.max(0, stageW - w)) });
        queue.push({ ...b, y: hit.y + m.h + 2 });
        queue.push({ ...b, y: Math.max(0, hit.y - m.h - 2) });
      }
      return out.length > 0 ? out : [start];
    };
    /**
     * A chip that would cover a neighbouring zone slides up or down until it does not. A slide
     * that keeps the chip at its own zone is the one that is taken: the zones on a desk sit
     * close together, and a chip that is a few pixels off still names the right one.
     */
    const slide = (b: Box): Box => {
      if (!zoneClash(b)) return b;
      const free = (t: Box) => inCanvas(t) && !zoneClash(t) && !chipClash(t);
      for (let d = 1; d <= 12; d++) {
        for (const y of [b.y - d, b.y + d]) {
          const t = { ...b, y };
          if (free(t) && anchored(t)) return t;
        }
      }
      for (let d = 1; d <= 12; d++) {
        for (const y of [b.y - d, b.y + d]) {
          const t = { ...b, y };
          if (free(t)) return t;
        }
      }
      return b;
    };
    const tries: Box[] = [];
    const free: Box[] = [];
    const moved: Box[] = [];
    for (const c of candidates) {
      const alt = resolve(c);
      // A place that is already free is preferred over a place another chip had to be moved
      // out of: the chip then sits where the zone says it should.
      if (alt.length > 0 && alt[0] === c) free.push(c);
      else moved.push(...alt);
    }
    tries.push(...free, ...moved);
    // The best place wins; the order of the candidates breaks a tie.
    let best: Box | null = null;
    let bestScore = -Infinity;
    for (const b of tries) {
      const pos = slide(b);
      const s = score(pos);
      if (s > bestScore) {
        bestScore = s;
        best = pos;
      }
    }
    const chosen = best ?? { x: item.box.x, y: item.box.y, w, h: m.h };
    placed.push({
      key: item.key,
      index: item.index,
      text: item.text,
      x: chosen.x,
      y: clamp(chosen.y, 0, Math.max(0, stageH - m.h)),
      w,
      missing: item.missing,
      accent: item.accent,
    });
  }
  return placed;
}

/**
 * The live preview with every placed zone drawn over it. Zones move, resize and rotate by
 * pointer (or by arrow keys, with Shift for ten units).
 *
 * A drag never re-renders React: the pointer move writes the zone's geometry straight onto the
 * element and repaints the LED overlay, and the geometry is committed to the layout once, when
 * the pointer goes up. The canvas can be zoomed and panned; that is a view transform only and
 * never reaches the saved layout.
 */
export function PreviewStage({
  zones,
  devices,
  selected,
  onSelect,
  onChange,
  onDrop,
  onDraggingChange,
  onLiveChange,
  layoutId,
  dropPayloadType,
}: StageProps) {
  const stageRef = useRef<HTMLDivElement>(null);
  const previewRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  const frameRef = useRef<Frame | null>(null);
  const imageRef = useRef<ImageData | null>(null);
  const dragRef = useRef<Drag | null>(null);
  const panRef = useRef<{ start: Point; orig: View } | null>(null);
  const liveRef = useRef<PlacedZone[]>(zones.map((z) => ({ ...z })));
  const zoneEls = useRef<(HTMLDivElement | null)[]>([]);
  const labelEls = useRef<(HTMLDivElement | null)[]>([]);
  const gripEls = useRef<{
    rotate: HTMLSpanElement | null;
    resize: HTMLSpanElement | null;
    stem: HTMLSpanElement | null;
  }>({ rotate: null, resize: null, stem: null });
  const guideEls = useRef<{ v: HTMLSpanElement | null; h: HTMLSpanElement | null }>({ v: null, h: null });
  const readoutRef = useRef<HTMLDivElement | null>(null);
  const controlsRef = useRef<HTMLDivElement | null>(null);
  const obstaclesRef = useRef<Box[]>([]);
  const paintQueued = useRef(false);
  const liveRaf = useRef<number | null>(null);
  const zoomRaf = useRef<number | null>(null);
  const viewCommit = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [view, setView] = useState<View>({ scale: 1, panX: 0, panY: 0 });
  const viewRef = useRef<View>(view);
  const [viewFor, setViewFor] = useState<string | null>(null);
  const reduced = usePrefersReducedMotion();

  // The view belongs to the layout: it is restored when the editor comes back to it. This runs
  // during render (React's pattern for state that follows a prop), so the canvas is never shown
  // with another layout's zoom.
  if (layoutId !== viewFor) {
    const remembered = readView(layoutId ?? "");
    const next = remembered ?? FIT_VIEW;
    viewRef.current = next;
    setViewFor(layoutId ?? null);
    setView(next);
  }

  // The LED offsets of every zone, in the zone's own frame. They only change when the zone is
  // resized or reversed, so they are cached instead of rebuilt on every pointer move.
  const ledCache = useMemo(() => {
    const map = new Map<string, LedCache>();
    for (const z of zones) {
      const info = findZoneInfo(devices, z.device_id, z.zone_index);
      if (!info) continue;
      map.set(zoneKey(z), {
        offsets: zoneLedOffsets(info.zone, z.reversed),
        linear: info.zone.kind.type === "linear",
      });
    }
    return map;
  }, [zones, devices]);

  const ledCacheRef = useRef(ledCache);
  ledCacheRef.current = ledCache;

  /** Writes one zone's geometry onto its element. No React involved. */
  const applyGeom = useCallback(
    (index: number, z: PlacedZone, v: View) => {
      const el = zoneEls.current[index];
      if (!el) return;
      const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
      const k = base * v.scale;
      el.style.left = `${(z.x + v.panX) * k}px`;
      el.style.top = `${(z.y + v.panY) * k}px`;
      el.style.width = `${z.width * k}px`;
      el.style.height = `${z.height * k}px`;
      el.style.transform = `rotate(${z.rotation_deg}deg)`;
    },
    [],
  );

  /** Repaints the LED dots from the last frame, at the zones' live geometry and view. */
  const paintOverlay = useCallback(() => {
    const ov = overlayRef.current;
    const stage = stageRef.current;
    if (!ov || !stage) return;
    const octx = ov.getContext("2d");
    if (!octx) return;
    const v = viewRef.current;
    const k = (ov.width / CANVAS_W) * v.scale;
    const ox = v.panX * k;
    const oy = v.panY * k;
    octx.setTransform(1, 0, 0, 1, 0, 0);
    octx.clearRect(0, 0, ov.width, ov.height);
    const r = clamp(Math.min(3.2, Math.max(2, ov.width / 360)) * v.scale, 1, 7);
    const frame = frameRef.current;
    const src = frame?.rgb;
    const geom = liveRef.current;
    for (let i = 0; i < geom.length; i++) {
      const z = geom[i];
      if (!z || !z.visible) continue;
      const cache = ledCacheRef.current.get(zoneKey(z));
      if (!cache) continue;
      const t = zoneTransform(z);
      const horizontal = z.width >= z.height;
      for (const p of cache.offsets) {
        const lx = cache.linear ? (horizontal ? p.x * z.width : 0) : p.x * z.width;
        const ly = cache.linear ? (horizontal ? 0 : p.x * z.height) : p.y * z.height;
        const cx = t.cx + lx * t.cos - ly * t.sin;
        const cy = t.cy + lx * t.sin + ly * t.cos;
        const ix = clamp(Math.floor(cx), 0, CANVAS_W - 1);
        const iy = clamp(Math.floor(cy), 0, CANVAS_H - 1);
        if (src && frame) {
          const o = (iy * frame.width + ix) * 3;
          octx.fillStyle = `rgb(${src[o] ?? 0} ${src[o + 1] ?? 0} ${src[o + 2] ?? 0})`;
        } else {
          // No frame yet: the dots still show where the LEDs sit.
          octx.fillStyle = "rgba(255, 255, 255, 0.34)";
        }
        octx.beginPath();
        octx.arc(cx * k + ox, cy * k + oy, r, 0, Math.PI * 2);
        octx.fill();
      }
    }
  }, []);

  const queuePaint = useCallback(() => {
    if (paintQueued.current) return;
    paintQueued.current = true;
    requestAnimationFrame(() => {
      paintQueued.current = false;
      paintOverlay();
    });
  }, [paintOverlay]);

  /**
   * The boxes a chip may not sit in: the zoom control and every tooltip it can open. The
   * tooltip bubbles are in the DOM even while they are closed, so their boxes can be read
   * without hovering anything.
   */
  const measureObstacles = useCallback((): Box[] => {
    const stage = stageRef.current;
    const controls = controlsRef.current;
    if (!stage || !controls) return [];
    const stageBox = stage.getBoundingClientRect();
    const rel = (r: DOMRect): Box => ({
      x: r.left - stageBox.left,
      y: r.top - stageBox.top,
      w: r.width,
      h: r.height,
    });
    const out: Box[] = [rel(controls.getBoundingClientRect())];
    for (const bubble of Array.from(controls.querySelectorAll(".tip-bubble"))) {
      const r = bubble.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) out.push(rel(r));
    }
    return out;
  }, []);

  /** Writes the label chips where they belong for a view. */
  const writeLabels = useCallback(
    (v: View) => {
      const stage = stageRef.current;
      if (!stage) return;
      const boxes = layoutLabels(
        zones,
        devices,
        stage.clientWidth,
        stage.clientHeight,
        v,
        obstaclesRef.current,
      );
      for (const b of boxes) {
        const el = labelEls.current[b.index];
        if (!el) continue;
        el.style.left = `${b.x}px`;
        el.style.top = `${b.y}px`;
      }
    },
    [zones, devices],
  );

  /** Draws the whole view: the effect, the zones, the labels and the LEDs. */
  const applyView = useCallback(
    (v: View) => {
      viewRef.current = v;
      const pv = previewRef.current;
      if (pv) {
        pv.style.transformOrigin = "0 0";
        const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
        pv.style.transform = `translate(${v.panX * base * v.scale}px, ${v.panY * base * v.scale}px) scale(${v.scale})`;
      }
      const geom = liveRef.current;
      for (let i = 0; i < geom.length; i++) {
        const z = geom[i];
        if (z) applyGeom(i, z, v);
      }
      writeLabels(v);
      applyGripsRef.current(selected);
      queuePaint();
    },
    [applyGeom, writeLabels, queuePaint, selected],
  );

  // Paint each arriving frame: the preview canvas, then the LED dots over it.
  useEffect(() => {
    const off = onFrame((frame) => {
      frameRef.current = frame;
      const pv = previewRef.current;
      const ctx = pv?.getContext("2d");
      if (ctx && frame.width === PREVIEW_W && frame.height === PREVIEW_H) {
        if (!imageRef.current) imageRef.current = ctx.createImageData(PREVIEW_W, PREVIEW_H);
        const img = imageRef.current;
        const px = img.data;
        const src = frame.rgb;
        for (let i = 0, o = 0; i < src.length; i += 3, o += 4) {
          px[o] = src[i] ?? 0;
          px[o + 1] = src[i + 1] ?? 0;
          px[o + 2] = src[i + 2] ?? 0;
          px[o + 3] = 255;
        }
        ctx.putImageData(img, 0, 0);
      }
      queuePaint();
    });
    return off;
  }, [queuePaint]);

  // Keep the overlay's bitmap the same size as its CSS box, so dots stay round.
  const [stageBox, setStageBox] = useState({ w: 0, h: 0 });
  useEffect(() => {
    const stage = stageRef.current;
    const ov = overlayRef.current;
    if (!stage || !ov) return;
    const size = () => {
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      const w = Math.max(1, Math.round(stage.clientWidth * dpr));
      const h = Math.max(1, Math.round(stage.clientHeight * dpr));
      if (ov.width !== w) ov.width = w;
      if (ov.height !== h) ov.height = h;
      setStageBox((s) =>
        s.w === stage.clientWidth && s.h === stage.clientHeight
          ? s
          : { w: stage.clientWidth, h: stage.clientHeight },
      );
      queuePaint();
    };
    size();
    const ro = new ResizeObserver(size);
    ro.observe(stage);
    return () => ro.disconnect();
  }, [queuePaint]);

  // When the layout changes outside a drag (a nudge, a load, a revert), the live geometry and
  // the elements follow it.
  useEffect(() => {
    liveRef.current = zones.map((z) => ({ ...z }));
    const v = viewRef.current;
    zones.forEach((z, i) => applyGeom(i, z, v));
    writeLabels(v);
    queuePaint();
  }, [zones, applyGeom, writeLabels, queuePaint]);

  const toLogical = (e: { clientX: number; clientY: number }): Point => {
    const box = stageRef.current?.getBoundingClientRect();
    if (!box || box.width === 0) return { x: 0, y: 0 };
    const v = viewRef.current;
    const k = (box.width / CANVAS_W) * v.scale;
    return {
      x: (e.clientX - box.left) / k - v.panX,
      y: (e.clientY - box.top) / k - v.panY,
    };
  };

  /** Where the grips of a zone sit, in canvas units. */
  const gripGeometry = useCallback((z: PlacedZone) => {
    const corners = zoneCorners(z);
    const se = corners[2] ?? { x: z.x + z.width, y: z.y + z.height };
    const outside = zoneGripPoint(z, GRIP_GAP);
    const inside = zoneGripPoint(z, -GRIP_GAP);
    const fits = outside.x > 4 && outside.x < CANVAS_W - 4 && outside.y > 4 && outside.y < CANVAS_H - 4;
    return {
      rotate: fits ? outside : inside,
      resize: { x: clamp(se.x, 2, CANVAS_W - 2), y: clamp(se.y, 2, CANVAS_H - 2) },
      stem: zoneGripPoint(z, 0),
      deg: z.rotation_deg,
    };
  }, []);

  /** Writes the grips straight onto the elements, so they follow a drag and a zoom. */
  const applyGrips = useCallback(
    (index: number | null) => {
      const els = gripEls.current;
      if (!els.rotate || !els.resize || !els.stem) return;
      if (index === null) return;
      const z = liveRef.current[index];
      if (!z) return;
      const g = gripGeometry(z);
      const stage = stageRef.current;
      const v = viewRef.current;
      const base = (stage?.clientWidth ?? CANVAS_W) / CANVAS_W;
      const k = base * v.scale;
      els.rotate.style.left = `${(g.rotate.x + v.panX) * k}px`;
      els.rotate.style.top = `${(g.rotate.y + v.panY) * k}px`;
      els.resize.style.left = `${(g.resize.x + v.panX) * k}px`;
      els.resize.style.top = `${(g.resize.y + v.panY) * k}px`;
      els.stem.style.left = `${(g.stem.x + v.panX) * k}px`;
      els.stem.style.top = `${(g.stem.y + v.panY) * k}px`;
      els.stem.style.height = `${Math.max(2, GRIP_GAP * k)}px`;
      els.stem.style.transform = `rotate(${g.deg}deg)`;
    },
    [gripGeometry],
  );

  // applyView needs applyGrips, and applyGrips needs nothing from it: a ref keeps the order.
  const applyGripsRef = useRef(applyGrips);
  applyGripsRef.current = applyGrips;

  /**
   * The readout by the pointer: where the zone is, in the canvas's own units, and how far it is
   * from the line it lines up with.
   */
  const showReadout = useCallback(
    (e: PointerEvent<HTMLDivElement>, x: number, y: number, snap: Snap) => {
      const el = readoutRef.current;
      const stage = stageRef.current;
      if (!el || !stage) return;
      const box = stage.getBoundingClientRect();
      const gap = snap.gapX !== null && snap.gapY !== null ? Math.min(snap.gapX, snap.gapY) : (snap.gapX ?? snap.gapY);
      const where = gap === null ? "" : gap < 0.05 ? " · on the guide" : ` · ${gap.toFixed(1)} u to the guide`;
      el.textContent = `x ${x.toFixed(0)} · y ${y.toFixed(0)}${where}`;
      el.style.opacity = "1";
      el.style.visibility = "visible";
      el.style.left = `${clamp(e.clientX - box.left + 16, 4, Math.max(4, box.width - 190))}px`;
      el.style.top = `${clamp(e.clientY - box.top + 16, 4, Math.max(4, box.height - 40))}px`;
    },
    [],
  );

  /** Shows or hides the alignment guides. */
  const applyGuides = useCallback((snap: { guideX: number | null; guideY: number | null }) => {
    const els = guideEls.current;
    const v = viewRef.current;
    const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
    const k = base * v.scale;
    if (els.v) {
      const gx = snap.guideX;
      const on = gx !== null;
      els.v.style.opacity = on ? "1" : "0";
      els.v.style.visibility = on ? "visible" : "hidden";
      if (gx !== null) els.v.style.left = `${(gx + v.panX) * k}px`;
    }
    if (els.h) {
      const gy = snap.guideY;
      const on = gy !== null;
      els.h.style.opacity = on ? "1" : "0";
      els.h.style.visibility = on ? "visible" : "hidden";
      if (gy !== null) els.h.style.top = `${(gy + v.panY) * k}px`;
    }
  }, []);

  // The grips follow the selected zone whenever it moves.
  useEffect(() => {
    applyGrips(selected);
  }, [selected, zones, view, applyGrips]);

  const beginDrag = (kind: DragKind, index: number, e: PointerEvent<HTMLElement>) => {
    e.stopPropagation();
    e.preventDefault();
    const z = liveRef.current[index];
    if (!z) return;
    onSelect(index);
    // The chip rides along with the zone it names, so the name never points at the place the
    // zone used to be.
    const label = labelEls.current[index];
    let offset: { dx: number; dy: number } | null = null;
    if (label) {
      const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
      const k = base * viewRef.current.scale;
      const zx = (z.x + viewRef.current.panX) * k;
      const zy = (z.y + viewRef.current.panY) * k;
      offset = {
        dx: (parseFloat(label.style.left) || 0) - zx,
        dy: (parseFloat(label.style.top) || 0) - zy,
      };
    }
    dragRef.current = {
      kind,
      index,
      start: toLogical(e),
      orig: { ...z },
      label: offset,
    };
    const el = zoneEls.current[index];
    if (el) el.dataset.dragging = "true";
    onDraggingChange?.(true);
    stageRef.current?.setPointerCapture(e.pointerId);
  };

  /** Tells the page where the dragged zone is, at most once per animation frame. */
  const notifyLive = useCallback(
    (index: number) => {
      if (liveRaf.current !== null) return;
      liveRaf.current = requestAnimationFrame(() => {
        liveRaf.current = null;
        const z = liveRef.current[index];
        if (z) onLiveChange?.(index, { ...z });
      });
    },
    [onLiveChange],
  );

  const onStageMove = (e: PointerEvent<HTMLDivElement>) => {
    const pan = panRef.current;
    if (pan) {
      // Drag-to-pan on empty space.
      const box = stageRef.current?.getBoundingClientRect();
      if (!box) return;
      const base = box.width / CANVAS_W;
      const k = base * pan.orig.scale;
      const next: View = {
        ...pan.orig,
        panX: pan.orig.panX + (e.clientX - pan.start.x) / k,
        panY: pan.orig.panY + (e.clientY - pan.start.y) / k,
      };
      applyView(clampView(next));
      return;
    }
    const d = dragRef.current;
    if (!d) return;
    const p = toLogical(e);
    const o = d.orig;
    const dx = p.x - d.start.x;
    const dy = p.y - d.start.y;
    const cur = liveRef.current[d.index];
    if (!cur) return;
    let next: PlacedZone;
    if (d.kind === "move") {
      // The zone snaps to the grid, or to another zone's line; Alt frees it.
      const others = liveRef.current.filter((_, i) => i !== d.index);
      const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
      const threshold = SNAP_PIXELS / (base * viewRef.current.scale);
      const want = { x: clamp(o.x + dx, 0, CANVAS_W - o.width), y: clamp(o.y + dy, 0, CANVAS_H - o.height), width: o.width, height: o.height };
      const snap = snapMove(want, others, threshold, e.altKey);
      next = {
        ...cur,
        x: clamp(snap.x, 0, CANVAS_W - o.width),
        y: clamp(snap.y, 0, CANVAS_H - o.height),
      };
      applyGuides({ guideX: snap.guideX, guideY: snap.guideY });
      showReadout(e, next.x, next.y, snap);
    } else if (d.kind === "resize") {
      next = {
        ...cur,
        width: clamp(o.width + dx, 2, CANVAS_W - o.x),
        height: clamp(o.height + dy, 2, CANVAS_H - o.y),
      };
      applyGuides({ guideX: null, guideY: null });
    } else {
      const cx = o.x + o.width / 2;
      const cy = o.y + o.height / 2;
      let deg = (Math.atan2(p.y - cy, p.x - cx) * 180) / Math.PI + 90;
      deg = ((deg % 360) + 360) % 360;
      if (e.shiftKey) deg = Math.round(deg / 15) * 15;
      next = { ...cur, rotation_deg: Math.round(deg * 10) / 10 };
      applyGuides({ guideX: null, guideY: null });
    }
    liveRef.current[d.index] = next;
    applyGeom(d.index, next, viewRef.current);
    applyGrips(d.index);
    // The chip rides along with the zone.
    if (d.label) {
      const el = labelEls.current[d.index];
      if (el) {
        const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
        const k = base * viewRef.current.scale;
        el.style.left = `${(next.x + viewRef.current.panX) * k + d.label.dx}px`;
        el.style.top = `${(next.y + viewRef.current.panY) * k + d.label.dy}px`;
      }
    }
    notifyLive(d.index);
    queuePaint();
  };

  const endDrag = (e: PointerEvent<HTMLDivElement>) => {
    const pan = panRef.current;
    if (pan) {
      panRef.current = null;
      if (stageRef.current?.hasPointerCapture(e.pointerId)) {
        stageRef.current.releasePointerCapture(e.pointerId);
      }
      // The view is written back to React when the gesture is over.
      setView(viewRef.current);
      return;
    }
    const d = dragRef.current;
    if (!d) return;
    dragRef.current = null;
    const el = zoneEls.current[d.index];
    if (el) delete el.dataset.dragging;
    onDraggingChange?.(false);
    applyGuides({ guideX: null, guideY: null });
    if (readoutRef.current) {
      readoutRef.current.style.opacity = "0";
      readoutRef.current.style.visibility = "hidden";
    }
    if (liveRaf.current !== null) {
      cancelAnimationFrame(liveRaf.current);
      liveRaf.current = null;
    }
    if (stageRef.current?.hasPointerCapture(e.pointerId)) {
      stageRef.current.releasePointerCapture(e.pointerId);
    }
    const g = liveRef.current[d.index];
    if (!g) return;
    const o = d.orig;
    const patch: Partial<PlacedZone> = {};
    if (Math.abs(g.x - o.x) > 1e-6) patch.x = g.x;
    if (Math.abs(g.y - o.y) > 1e-6) patch.y = g.y;
    if (Math.abs(g.width - o.width) > 1e-6) patch.width = g.width;
    if (Math.abs(g.height - o.height) > 1e-6) patch.height = g.height;
    if (Math.abs(g.rotation_deg - o.rotation_deg) > 1e-6) patch.rotation_deg = g.rotation_deg;
    if (Object.keys(patch).length > 0) onChange(d.index, patch);
  };

  const onRectKey = (index: number, e: KeyboardEvent<HTMLDivElement>) => {
    const z = zones[index];
    if (!z) return;
    const step = e.shiftKey ? 10 : 1;
    let patch: Partial<PlacedZone> | null = null;
    if (e.key === "ArrowLeft") patch = { x: clamp(z.x - step, 0, CANVAS_W - z.width) };
    if (e.key === "ArrowRight") patch = { x: clamp(z.x + step, 0, CANVAS_W - z.width) };
    if (e.key === "ArrowUp") patch = { y: clamp(z.y - step, 0, CANVAS_H - z.height) };
    if (e.key === "ArrowDown") patch = { y: clamp(z.y + step, 0, CANVAS_H - z.height) };
    if (patch) {
      e.preventDefault();
      onChange(index, patch);
    }
  };

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    if (Array.from(e.dataTransfer.types).includes(dropPayloadType)) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
    }
  };

  const onDropHere = (e: DragEvent<HTMLDivElement>) => {
    const raw = e.dataTransfer.getData(dropPayloadType);
    if (!raw) return;
    e.preventDefault();
    try {
      const payload = JSON.parse(raw) as { deviceId: string; zoneIndex: number };
      const p = toLogical(e);
      onDrop(payload.deviceId, payload.zoneIndex, p.x, p.y);
    } catch {
      // A drag from somewhere else carries nothing we understand; ignore it.
    }
  };

  // The label chips: laid out once per layout change, then moved by hand during a drag.
  const [labelBoxes, setLabelBoxes] = useState<LabelBox[]>([]);
  const metrics = labelMetrics(view.scale);
  useLayoutEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const obstacles = measureObstacles();
    obstaclesRef.current = obstacles;
    setLabelBoxes(
      layoutLabels(zones, devices, stage.clientWidth, stage.clientHeight, view, obstacles),
    );
  }, [zones, devices, stageBox, view, measureObstacles]);

  // The view is written back to React when the gesture settles, so the page keeps the zoom.
  const commitView = useCallback(
    (v: View) => {
      if (viewCommit.current) clearTimeout(viewCommit.current);
      viewCommit.current = setTimeout(() => {
        viewCommit.current = null;
        setView(v);
      }, VIEW_COMMIT_MS);
    },
    [],
  );

  // The view belongs to the layout: it is remembered per layout, and restored when the editor
  // comes back to it. A layout that was never zoomed has no entry at all.
  useEffect(() => {
    if (!layoutId) return;
    if (view.scale === 1 && view.panX === 0 && view.panY === 0) forgetView(layoutId);
    else writeView(layoutId, view);
  }, [layoutId, view]);

  // The wheel zooms around the pointer. React's own wheel listener is passive, so the page would
  // scroll as well: a native, non-passive listener does the work.
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      // A wheel gesture wins over an eased zoom that is still running.
      if (zoomRaf.current !== null) {
        cancelAnimationFrame(zoomRaf.current);
        zoomRaf.current = null;
      }
      const box = stage.getBoundingClientRect();
      const v = viewRef.current;
      const px = e.clientX - box.left;
      const py = e.clientY - box.top;
      const base = box.width / CANVAS_W;
      const factor = Math.exp(-e.deltaY * 0.0016);
      const scale = clamp(v.scale * factor, MIN_SCALE, MAX_SCALE);
      // The logical point under the pointer stays under the pointer.
      const lx = px / (base * v.scale) - v.panX;
      const ly = py / (base * v.scale) - v.panY;
      const next = clampView({ scale, panX: px / (base * scale) - lx, panY: py / (base * scale) - ly });
      applyView(next);
      commitView(next);
    };
    stage.addEventListener("wheel", onWheel, { passive: false });
    return () => stage.removeEventListener("wheel", onWheel);
  }, [applyView, commitView]);

  /**
   * Eases the view to a new one, over ZOOM_MS, with the house curve.
   *
   * The wheel does not use this: a wheel gesture is direct manipulation, and easing it would
   * put the canvas behind the pointer. The buttons and the fit control are a single, discrete
   * action, so they move.
   */
  const animateTo = useCallback(
    (target: View) => {
      if (zoomRaf.current !== null) {
        cancelAnimationFrame(zoomRaf.current);
        zoomRaf.current = null;
      }
      const from = { ...viewRef.current };
      // The reader asked for less motion, in the system or in the editor's own switch: the view
      // lands at once.
      const still =
        reduced ||
        (typeof document !== "undefined" &&
          document.documentElement.getAttribute("data-motion") === "reduced");
      if (still) {
        applyView(target);
        setView(target);
        return;
      }
      const t0 = performance.now();
      const step = () => {
        const raw = Math.min(1, (performance.now() - t0) / ZOOM_MS);
        const e = houseEase(raw);
        const next: View = {
          scale: from.scale + (target.scale - from.scale) * e,
          panX: from.panX + (target.panX - from.panX) * e,
          panY: from.panY + (target.panY - from.panY) * e,
        };
        applyView(next);
        if (raw < 1) {
          zoomRaf.current = requestAnimationFrame(step);
        } else {
          zoomRaf.current = null;
          setView(target);
        }
      };
      zoomRaf.current = requestAnimationFrame(step);
    },
    [applyView, reduced],
  );

  // An eased zoom that is still running when the canvas goes away must not keep painting.
  useEffect(
    () => () => {
      if (zoomRaf.current !== null) cancelAnimationFrame(zoomRaf.current);
    },
    [],
  );

  const zoomBy = (factor: number) => {
    const box = stageRef.current?.getBoundingClientRect();
    if (!box) return;
    const v = viewRef.current;
    const scale = clamp(v.scale * factor, MIN_SCALE, MAX_SCALE);
    const cx = box.width / 2;
    const cy = box.height / 2;
    const base = box.width / CANVAS_W;
    const lx = cx / (base * v.scale) - v.panX;
    const ly = cy / (base * v.scale) - v.panY;
    const next = clampView({ scale, panX: cx / (base * scale) - lx, panY: cy / (base * scale) - ly });
    animateTo(next);
  };

  const fit = () => {
    animateTo({ ...FIT_VIEW });
  };

  // A zone whose device the daemon does not drive is stale: it is drawn dimmed and dashed, and
  // the inspector says so.
  const missing = useMemo(() => {
    const out = new Set<number>();
    if (!devices) return out;
    zones.forEach((z, i) => {
      const info = findZoneInfo(devices, z.device_id, z.zone_index);
      if (!info || !info.device.connected) out.add(i);
    });
    return out;
  }, [zones, devices]);

  const selectedZone = selected !== null ? zones[selected] : undefined;
  const grips = selectedZone ? gripGeometry(selectedZone) : null;

  const base = (stageBox.w || CANVAS_W) / CANVAS_W;
  const k = base * view.scale;

  return (
    <div
      ref={stageRef}
      className={`stage ${desk.desk} ${desk.canvas}`}
      onPointerMove={onStageMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onDragOver={onDragOver}
      onDrop={onDropHere}
      onPointerDown={(e) => {
        onSelect(null);
        // Empty space: drag to pan.
        panRef.current = { start: { x: e.clientX, y: e.clientY }, orig: { ...viewRef.current } };
        stageRef.current?.setPointerCapture(e.pointerId);
      }}
    >
      <canvas
        ref={previewRef}
        className="stage-canvas"
        width={PREVIEW_W}
        height={PREVIEW_H}
        aria-hidden="true"
        style={{
          transformOrigin: "0 0",
          transform: `translate(${view.panX * base * view.scale}px, ${view.panY * base * view.scale}px) scale(${view.scale})`,
        }}
      />
      <canvas ref={overlayRef} className="stage-leds" aria-hidden="true" />

      <div
        ref={readoutRef}
        className={desk.readout}
        data-readout="true"
        aria-hidden="true"
        style={{ opacity: 0, visibility: "hidden" }}
      />

      <span
        ref={(el) => {
          guideEls.current.v = el;
        }}
        className={desk.guideV}
        aria-hidden="true"
      />
      <span
        ref={(el) => {
          guideEls.current.h = el;
        }}
        className={desk.guideH}
        aria-hidden="true"
      />

      {zones.map((z, i) => {
        const found = findZoneInfo(devices, z.device_id, z.zone_index);
        const accent = accentOf(devices, z.device_id);
        const isSel = selected === i;
        const isMissing = missing.has(i);
        const style = {
          left: `${(z.x + view.panX) * k}px`,
          top: `${(z.y + view.panY) * k}px`,
          width: `${z.width * k}px`,
          height: `${z.height * k}px`,
          transform: `rotate(${z.rotation_deg}deg)`,
          "--zc": isMissing ? "var(--down)" : `var(${ACCENTS[accent] ?? "--a1"})`,
        } as CSSProperties;
        const name = found ? found.zone.name : "device not found";
        const count = found ? found.zone.led_count : 0;
        return (
          <div
            key={zoneKey(z)}
            ref={(el) => {
              zoneEls.current[i] = el;
            }}
            className={`zrect ${desk.zone}`}
            role="button"
            tabIndex={0}
            aria-pressed={isSel}
            data-zone-index={i}
            aria-label={`${found?.device.name ?? z.device_id} ${name}, ${count} LEDs${z.visible ? "" : ", hidden"}`}
            data-hidden={!z.visible}
            data-selected={isSel}
            data-missing={isMissing}
            data-hover={hover === i}
            style={style}
            onPointerDown={(e) => beginDrag("move", i, e)}
            onPointerEnter={() => setHover(i)}
            onPointerLeave={() => setHover((h) => (h === i ? null : h))}
            onKeyDown={(e) => onRectKey(i, e)}
            onFocus={() => onSelect(i)}
          />
        );
      })}

      {labelBoxes.map((b) => {
        const i = b.index;
        const isSel = selected === i;
        return (
          <div
            key={b.key}
            ref={(el) => {
              labelEls.current[i] = el;
            }}
            className={desk.label}
            data-zone-label="true"
            data-zone-index={i}
            data-selected={isSel}
            data-missing={b.missing}
            style={{
              left: `${b.x}px`,
              top: `${b.y}px`,
              height: `${metrics.h}px`,
              lineHeight: `${metrics.h - 2}px`,
              fontSize: `${metrics.font}px`,
              // The chip carries the colour of the device it names, so the eye can pair it with
              // the zone's outline without reading it.
              "--zc": b.missing
                ? "var(--down)"
                : `var(${ACCENTS[b.accent] ?? "--a1"})`,
            } as CSSProperties}
          >
            {b.text}
          </div>
        );
      })}

      {grips && selected !== null && (
        <>
          <span
            ref={(el) => {
              gripEls.current.stem = el;
            }}
            className={desk.stem}
            aria-hidden="true"
            style={{
              left: `${(grips.stem.x + view.panX) * k}px`,
              top: `${(grips.stem.y + view.panY) * k}px`,
              height: `${Math.max(2, GRIP_GAP * k)}px`,
              transform: `rotate(${grips.deg}deg)`,
            }}
          />
          <span
            ref={(el) => {
              gripEls.current.rotate = el;
            }}
            className={`${desk.grip} ${desk.gripRotate}`}
            data-grip="rotate"
            style={{
              width: 13,
              height: 13,
              left: `${(grips.rotate.x + view.panX) * k}px`,
              top: `${(grips.rotate.y + view.panY) * k}px`,
            }}
            role="button"
            tabIndex={0}
            aria-label="Rotate this zone"
            onPointerDown={(e) => beginDrag("rotate", selected, e)}
          />
          <span
            ref={(el) => {
              gripEls.current.resize = el;
            }}
            className={desk.grip}
            data-grip="resize"
            style={{
              width: 13,
              height: 13,
              left: `${(grips.resize.x + view.panX) * k}px`,
              top: `${(grips.resize.y + view.panY) * k}px`,
            }}
            role="button"
            tabIndex={0}
            aria-label="Resize this zone"
            onPointerDown={(e) => beginDrag("resize", selected, e)}
          />
        </>
      )}

      <div
        ref={controlsRef}
        className={desk.controls}
        onPointerDown={(e) => e.stopPropagation()}
        onWheel={(e) => e.stopPropagation()}
      >
        <IconButton label="Zoom out" icon={<Minus size={13} />} size="sm" onClick={() => zoomBy(1 / 1.3)} />
        <span className={desk.zoomValue}>{Math.round(view.scale * 100)}%</span>
        <IconButton label="Zoom in" icon={<Plus size={13} />} size="sm" onClick={() => zoomBy(1.3)} />
        <IconButton label="Fit the canvas" icon={<ArrowsOut size={13} />} size="sm" onClick={fit} />
      </div>
    </div>
  );
}

/**
 * Keeps the canvas in the stage: at or above 1 the canvas covers the stage, so it can be
 * panned but never dragged away; below 1 it is centred.
 */
function clampView(v: View): View {
  const scale = clamp(v.scale, MIN_SCALE, MAX_SCALE);
  // The pan is in canvas units, so the view is the same on any screen: at or above 1 the canvas
  // covers the stage and can be panned, below 1 it is centred.
  const spanX = CANVAS_W * (1 / scale - 1);
  const spanY = CANVAS_H * (1 / scale - 1);
  return {
    scale,
    panX: scale >= 1 ? clamp(v.panX, spanX, 0) : spanX / 2,
    panY: scale >= 1 ? clamp(v.panY, spanY, 0) : spanY / 2,
  };
}
