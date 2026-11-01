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
  type PointerEvent,
  type ReactNode,
} from "react";
import {
  AlignBottom,
  AlignCenterHorizontal,
  AlignCenterVertical,
  AlignLeft,
  AlignRight,
  AlignTop,
  ArrowsOut,
  LockSimple,
  Minus,
  Plus,
} from "@phosphor-icons/react";
import { NumberField } from "@/components/NumberField";
import { Segmented } from "@/components/Segmented";
import { Tooltip } from "@/components/Tooltip";
import { IconButton } from "@/components/Button";
import { EffectPreview, type PreviewValues } from "@/components/EffectPreview";
import { useMediaQuery } from "@/components/MediaQuery";
import { MM_PER_UNIT, caseLandmarkFor } from "@/lib/caseForms";
import { onFrame, PREVIEW_H, PREVIEW_W, usePreviewStatus, type Frame } from "@/lib/preview";
import {
  canvasCell,
  findZoneInfo,
  formatUnits,
  lengthNumber,
  mmFromLength,
  placementName,
  resolveShape,
  unitWord,
  unitsToMm,
  zoneCorners,
  zoneKey,
  zoneLocalLeds,
  zoneTransform,
  type CanvasCell,
  type LengthUnit,
  type LocalLeds,
  type Point,
} from "@/lib/zones";
import { clamp } from "@/lib/format";
import {
  EDGE_HANDLE_MIN_PX,
  HANDLES,
  dragThresholdPx,
  hitTest,
  marqueePick,
  partsInsideCase,
  type HandleDef,
  type HitPart,
  type HitTarget,
  type PointerKind,
} from "@/lib/hitTest";
import { usePrefersReducedMotion } from "@/lib/motion";
import { FIT_VIEW, forgetView, readView, writeView } from "@/lib/viewPrefs";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { CaseForm } from "@/proto/CaseForm";
import type { Landmark } from "@/proto/Landmark";
import type { PartKind } from "@/proto/PartKind";
import type { PlacedZone } from "@/proto/PlacedZone";
import type { SamplingMode } from "@/proto/SamplingMode";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import { CanvasFurniture } from "@/views/editor/Rulers";
import { PART_LABEL, partForDevice, type Sel } from "@/views/editor/parts";
import { PartArt } from "./ZoneArt";
import desk from "./desk.module.css";

const CANVAS_W = 320;
const CANVAS_H = 200;
const ACCENTS = ["--a1", "--a2", "--a3", "--a4", "--a5"] as const;

/**
 * The parts whose own body can carry the light of their LEDs: the compact ones, which are a
 * solid thing with a light in it. A strip, an edge or a box is only its LEDs and their halos:
 * its body is not lit, because the light in it is a line, not a lamp.
 */
const COMPACT_KINDS: ReadonlySet<PartKind> = new Set<PartKind>([
  "fan",
  "pump",
  "ram",
  "gpu",
  "keyboard",
  "mouse",
  "microphone",
]);

/** How close, in screen pixels, a part has to be to a line to snap to it. */
const SNAP_PIXELS = 8;
/** The smallest a part can be, in canvas units. */
const MIN_SIZE = 2;
/** How far an arrow key nudges the picked part, in mm; with Shift, the big step. */
const NUDGE_MM = 1;
const NUDGE_MM_BIG = 10;
/** The halo's margin past the part's own outline, in css pixels. */
const HALO_MARGIN_PX = 7;
/** The dot at the heart of a halo, in css pixels. */
const CORE_PX = 1.9;

/**
 * A picture the LED dots can be sampled from: the daemon's frames, or the editor's own draw of
 * the effect. The pixels are `stride` bytes each, row by row.
 */
interface Picture {
  width: number;
  height: number;
  data: Uint8Array | Uint8ClampedArray;
  stride: number;
}

type RGB = [number, number, number];

/** The colour of the pixel at (x, y), the pixel nearest the point. */
function sampleNearest(pic: Picture, x: number, y: number): RGB {
  const px = clamp(Math.round(x), 0, pic.width - 1);
  const py = clamp(Math.round(y), 0, pic.height - 1);
  const o = (py * pic.width + px) * pic.stride;
  return [pic.data[o] ?? 0, pic.data[o + 1] ?? 0, pic.data[o + 2] ?? 0];
}

/** The colour at (x, y), interpolated from the four neighbours. */
function sampleBilinear(pic: Picture, x: number, y: number): RGB {
  const maxX = pic.width - 1;
  const maxY = pic.height - 1;
  const fx = clamp(x, 0, maxX);
  const fy = clamp(y, 0, maxY);
  const x0 = Math.floor(fx);
  const y0 = Math.floor(fy);
  const x1 = Math.min(x0 + 1, maxX);
  const y1 = Math.min(y0 + 1, maxY);
  const tx = fx - x0;
  const ty = fy - y0;
  const at = (px: number, py: number): RGB => {
    const o = (py * pic.width + px) * pic.stride;
    return [pic.data[o] ?? 0, pic.data[o + 1] ?? 0, pic.data[o + 2] ?? 0];
  };
  const p00 = at(x0, y0);
  const p10 = at(x1, y0);
  const p01 = at(x0, y1);
  const p11 = at(x1, y1);
  const out: RGB = [0, 0, 0];
  for (let c = 0; c < 3; c++) {
    const a0 = p00[c] ?? 0;
    const a1 = p10[c] ?? 0;
    const b0 = p01[c] ?? 0;
    const b1 = p11[c] ?? 0;
    const top = a0 + (a1 - a0) * tx;
    const bottom = b0 + (b1 - b0) * tx;
    out[c] = Math.round(top + (bottom - top) * ty);
  }
  return out;
}

/** The mean of the pixels whose centres fall inside the LED's rectangle. */
function sampleArea(pic: Picture, x: number, y: number, bounds: [number, number, number, number]): RGB {
  const maxX = pic.width - 1;
  const maxY = pic.height - 1;
  // A pixel's centre is at (x + 0.5, y + 0.5).
  const x0 = clamp(Math.ceil(bounds[0] - 0.5), 0, maxX);
  const y0 = clamp(Math.ceil(bounds[1] - 0.5), 0, maxY);
  const x1 = clamp(Math.floor(bounds[2] - 0.5), 0, maxX);
  const y1 = clamp(Math.floor(bounds[3] - 0.5), 0, maxY);
  if (x0 > x1 || y0 > y1) return sampleNearest(pic, x, y);
  let r = 0;
  let g = 0;
  let b = 0;
  let n = 0;
  for (let py = y0; py <= y1; py++) {
    for (let px = x0; px <= x1; px++) {
      const o = (py * pic.width + px) * pic.stride;
      r += pic.data[o] ?? 0;
      g += pic.data[o + 1] ?? 0;
      b += pic.data[o + 2] ?? 0;
      n++;
    }
  }
  if (n === 0) return sampleNearest(pic, x, y);
  return [Math.round(r / n), Math.round(g / n), Math.round(b / n)];
}

/**
 * The colour of one LED, sampled the way the layout says to: nearest, bilinear, or the mean
 * over the LED's own rectangle. The daemon samples the same cells
 * (`crates/canvas-core/src/sampling.rs`), so the dots and the LEDs agree.
 */
function sampleLed(pic: Picture, cell: CanvasCell, mode: SamplingMode): RGB {
  // The picture may be smaller than the canvas: the editor's own draw is 112 x 70.
  const sx = pic.width / CANVAS_W;
  const sy = pic.height / CANVAS_H;
  const x = cell.cx * sx;
  const y = cell.cy * sy;
  if (mode === "bilinear") return sampleBilinear(pic, x, y);
  if (mode === "area_average") {
    return sampleArea(pic, x, y, [
      cell.bounds[0] * sx,
      cell.bounds[1] * sy,
      cell.bounds[2] * sx,
      cell.bounds[3] * sy,
    ]);
  }
  return sampleNearest(pic, x, y);
}

/** The zoom range. 1 is the whole canvas in the stage. */
const MIN_SCALE = 0.25;
const MAX_SCALE = 8;
/** How long the view waits before it is written back to React, in ms. */
const VIEW_COMMIT_MS = 140;
/** How long the view eases to a new zoom level or back to the fit view, in ms. */
const ZOOM_MS = 180;
/** How long the view eases to a new level after a mouse wheel notch, in ms. */
const WHEEL_MS = 120;
/**
 * A wheel delta this big is a mouse notch. A trackpad's deltas are small and continuous, and a
 * ctrl+wheel is the browser's own gesture: both are applied at once.
 */
const NOTCH_DELTA = 40;
/** Where the one-time hint about the two modes is remembered. */
const HINT_KEY = "canvas-rgb.mode-hint";

/** The resize cursor for a handle on a part turned by `rotation` degrees. */
function handleCursor(h: HandleDef, rotation: number): string {
  const angle = (Math.atan2(h.sy, h.sx) * 180) / Math.PI + rotation;
  const slot = ((Math.round(angle / 45) % 4) + 4) % 4;
  return ["ew-resize", "nwse-resize", "ns-resize", "nesw-resize"][slot] ?? "nwse-resize";
}

/** A rectangle on the stage, in css pixels. */
interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

function overlaps(a: Box, b: Box, pad = 0): boolean {
  return a.x < b.x + b.w + pad && b.x < a.x + a.w + pad && a.y < b.y + b.h + pad && b.y < a.y + a.h + pad;
}

type DragKind = "move" | "resize" | "rotate" | "marquee";

/** A part as it was when a drag began. */
interface Orig {
  sel: Sel;
  zone?: PlacedZone;
  landmark?: Landmark;
}

/** A part being dragged: what it was, where the pointer was, and what it is now. */
interface Drag {
  kind: DragKind;
  /** The part the pointer is on. */
  sel: Sel;
  /** Every part the drag moves, with where it started. */
  origs: Orig[];
  start: Point;
  handle: HandleDef | null;
  /** The parts the drag carries with it: a case and everything inside it, zones and landmarks. */
  riders: Orig[];
}

/**
 * A press on a part that has not travelled far enough to be a drag. If it ends here it was a
 * click: it picked the part and changed nothing, so it makes no undo step.
 */
interface Pending {
  sel: Sel;
  pointerId: number;
  /** Where the press landed, in client pixels. */
  x: number;
  y: number;
  /** Where it landed on the desk, in canvas units: the drag starts from here, so nothing jumps. */
  start: Point;
  thresholdPx: number;
  /** A locked part is picked by a press and never moved. */
  locked: boolean;
  /** Pressing a part in a many-part pick keeps the pick for a drag; a plain click narrows to it. */
  collapse: boolean;
}

/** The view: how the 320 x 200 canvas is drawn into the stage. It never touches the layout. */
interface View {
  scale: number;
  panX: number;
  panY: number;
}

/** A rectangle on the canvas, in canvas units. */
interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The lines a part offers to the guides: its two edges and its middle. */
function zoneLines(z: Rect, axis: "x" | "y"): number[] {
  const a = axis === "x" ? z.x : z.y;
  const b = axis === "x" ? z.width : z.height;
  return [a, a + b / 2, a + b];
}

/** The desk's own edges and middle: a part lines up with them as it does with another part. */
const CANVAS_BOX: Rect = { x: 0, y: 0, width: CANVAS_W, height: CANVAS_H };

interface Snap {
  x: number;
  y: number;
  /** The lines the part snapped to, in canvas units. */
  guideX: number | null;
  guideY: number | null;
  /** A second vertical line, for the equal-spacing snap. */
  guideX2: number | null;
  /** How far the part is from the line it snapped to, in canvas units. */
  gapX: number | null;
  gapY: number | null;
}

/** The nearest of `lines` to `value` within `threshold`, or null. */
function nearestLine(value: number, lines: number[], threshold: number): number | null {
  let best: number | null = null;
  for (const line of lines) {
    if (Math.abs(line - value) <= threshold && (best === null || Math.abs(line - value) < Math.abs(best - value))) {
      best = line;
    }
  }
  return best;
}

/**
 * Where a moving part lands. It follows the pointer exactly - no grid, no easing - unless it
 * comes near a line it can sit against: another part's edge or middle, the case's panels, or
 * the desk's own edges. The alignment wins, and the line it snapped to is the guide.
 */
function snapMove(
  moving: Rect,
  others: Rect[],
  panels: { x: number[]; y: number[] },
  threshold: number,
  free: boolean,
): Snap {
  const none: Snap = { x: moving.x, y: moving.y, guideX: null, guideY: null, guideX2: null, gapX: null, gapY: null };
  if (free) return none;
  const allX = [...others.flatMap((o) => zoneLines(o, "x")), ...panels.x];
  const allY = [...others.flatMap((o) => zoneLines(o, "y")), ...panels.y];
  let bestX: { delta: number; line: number } | null = null;
  let bestY: { delta: number; line: number } | null = null;
  for (const line of allX) {
    for (const mine of zoneLines(moving, "x")) {
      const delta = line - mine;
      if (Math.abs(delta) <= threshold && (!bestX || Math.abs(delta) < Math.abs(bestX.delta))) {
        bestX = { delta, line };
      }
    }
  }
  for (const line of allY) {
    for (const mine of zoneLines(moving, "y")) {
      const delta = line - mine;
      if (Math.abs(delta) <= threshold && (!bestY || Math.abs(delta) < Math.abs(bestY.delta))) {
        bestY = { delta, line };
      }
    }
  }
  return {
    x: bestX ? moving.x + bestX.delta : moving.x,
    y: bestY ? moving.y + bestY.delta : moving.y,
    guideX: bestX?.line ?? null,
    guideY: bestY?.line ?? null,
    guideX2: null,
    gapX: bestX ? Math.abs(bestX.delta) : null,
    gapY: bestY ? Math.abs(bestY.delta) : null,
  };
}

/**
 * The equal-spacing snap: when a part sits between two others and the gaps on either side are
 * the same, it stays there. It is the snap a person expects without thinking about it.
 */
function equalSpacing(
  moving: Rect,
  others: Rect[],
  threshold: number,
): { x: number | null; y: number | null; lineA: number | null; lineB: number | null } {
  let best: { x: number | null; y: number | null; lineA: number | null; lineB: number | null } = {
    x: null,
    y: null,
    lineA: null,
    lineB: null,
  };
  const overlapsY = (a: Rect, b: Rect) => a.y < b.y + b.height && b.y < a.y + a.height;
  const overlapsX = (a: Rect, b: Rect) => a.x < b.x + b.width && b.x < a.x + a.width;
  for (const a of others) {
    for (const b of others) {
      if (a === b) continue;
      // The pair on the left and right of the moving part.
      if (a.x + a.width <= moving.x + 0.001 && b.x >= moving.x + moving.width - 0.001) {
        if (!overlapsY(a, moving) || !overlapsY(b, moving)) continue;
        const gapL = moving.x - (a.x + a.width);
        const gapR = b.x - (moving.x + moving.width);
        if (Math.abs(gapL - gapR) <= threshold) {
          const x = (a.x + a.width + b.x - moving.width) / 2;
          if (best.x === null || Math.abs(x - moving.x) < Math.abs(best.x - moving.x)) {
            best = { ...best, x, lineA: a.x + a.width, lineB: b.x };
          }
        }
      }
      // The pair above and below it.
      if (a.y + a.height <= moving.y + 0.001 && b.y >= moving.y + moving.height - 0.001) {
        if (!overlapsX(a, moving) || !overlapsX(b, moving)) continue;
        const gapT = moving.y - (a.y + a.height);
        const gapB = b.y - (moving.y + moving.height);
        if (Math.abs(gapT - gapB) <= threshold) {
          const y = (a.y + a.height + b.y - moving.height) / 2;
          if (best.y === null || Math.abs(y - moving.y) < Math.abs(best.y - moving.y)) {
            best = { ...best, y, lineA: a.y + a.height, lineB: b.y };
          }
        }
      }
    }
  }
  return best;
}

/**
 * Resizes a part by one of its eight handles, in the part's own turned frame, as Figma does:
 *
 *   - the handle opposite the one dragged stays where it is;
 *   - Shift keeps the box's proportions (corners scale both sides, an edge scales the other side
 *     about the box's middle line);
 *   - Alt resizes from the centre.
 *
 * `p` is the pointer on the canvas, already snapped by the caller.
 */
function resizeBy(o: Rect & { rotation_deg: number }, handle: HandleDef, p: Point, shift: boolean, alt: boolean): Rect {
  const rad = (o.rotation_deg * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const cx0 = o.x + o.width / 2;
  const cy0 = o.y + o.height / 2;
  // The pointer in the part's own frame: origin at its centre, axes along its sides.
  const dx = p.x - cx0;
  const dy = p.y - cy0;
  const qx = dx * cos + dy * sin;
  const qy = -dx * sin + dy * cos;
  let w = o.width;
  let h = o.height;
  if (handle.sx !== 0) w = alt ? 2 * handle.sx * qx : handle.sx * qx + o.width / 2;
  if (handle.sy !== 0) h = alt ? 2 * handle.sy * qy : handle.sy * qy + o.height / 2;
  w = clamp(w, MIN_SIZE, CANVAS_W);
  h = clamp(h, MIN_SIZE, CANVAS_H);
  if (shift) {
    const ratio = o.width / o.height;
    if (handle.sx !== 0 && handle.sy !== 0) {
      const s = Math.max(w / o.width, h / o.height);
      w = clamp(o.width * s, MIN_SIZE, CANVAS_W);
      h = clamp(w / ratio, MIN_SIZE, CANVAS_H);
      w = h * ratio;
    } else if (handle.sx !== 0) {
      h = clamp(w / ratio, MIN_SIZE, CANVAS_H);
      w = h * ratio;
    } else {
      w = clamp(h * ratio, MIN_SIZE, CANVAS_W);
      h = w / ratio;
    }
  }
  // The centre moves by half the growth on each axis the handle pulls, so the far side stays put.
  let lx = 0;
  let ly = 0;
  if (!alt) {
    if (handle.sx !== 0) lx = (handle.sx * (w - o.width)) / 2;
    if (handle.sy !== 0) ly = (handle.sy * (h - o.height)) / 2;
  }
  const cx = cx0 + lx * cos - ly * sin;
  const cy = cy0 + lx * sin + ly * cos;
  return { x: cx - w / 2, y: cy - h / 2, width: w, height: h };
}

/**
 * The view the desk opens in: the desk fills the stage's height, so the PC is as big as the
 * page allows. The crop is capped, so the desk's own edges never disappear off the screen, and
 * the fit control shows the whole of it.
 */
function heroView(w: number, h: number, centre: number): View {
  if (w <= 0 || h <= 0) return { scale: 1, panX: 0, panY: 0 };
  const base = w / CANVAS_W;
  const heightFit = h / CANVAS_H / base;
  const scale = Math.max(1, Math.min(heightFit, 1.35));
  // The view looks at the parts, not at the empty desk around them - but the desk's own edges
  // never come into view, so the picture behind the parts is always the whole desk.
  const span = CANVAS_W * (1 / scale - 1);
  const panX = clamp(CANVAS_W / (2 * scale) - centre, span, 0);
  return { scale, panX, panY: 0 };
}

/** The view that shows the whole desk. */
function wholeView(w: number, h: number): View {
  if (w <= 0 || h <= 0) return { scale: 1, panX: 0, panY: 0 };
  const base = w / CANVAS_W;
  const heightFit = h / CANVAS_H / base;
  return { scale: Math.min(1, heightFit), panX: 0, panY: 0 };
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

/**
 * The blue-ish noise that keeps a halo from banding: a high-pass of white noise, so the steps
 * in the gradient are broken into noise instead of showing as rings. One LSB of it is added to
 * every pixel of a halo, which is enough and is not seen.
 */
let noiseTile: HTMLCanvasElement | null = null;
function noisePattern(ctx: CanvasRenderingContext2D): CanvasPattern | null {
  if (!noiseTile) {
    const size = 64;
    const c = document.createElement("canvas");
    c.width = size;
    c.height = size;
    const g = c.getContext("2d");
    if (!g) return null;
    const img = g.createImageData(size, size);
    const raw = new Float32Array(size * size);
    for (let i = 0; i < raw.length; i++) raw[i] = Math.random();
    // The high pass: a pixel minus the mean of its neighbours.
    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        let sum = 0;
        let n = 0;
        for (let dy = -1; dy <= 1; dy++) {
          for (let dx = -1; dx <= 1; dx++) {
            if (dx === 0 && dy === 0) continue;
            sum += raw[((y + dy + size) % size) * size + ((x + dx + size) % size)] ?? 0;
            n++;
          }
        }
        const hp = (raw[y * size + x] ?? 0) - sum / n;
        const v = hp > 0 ? 255 : 0;
        const o = (y * size + x) * 4;
        img.data[o] = v;
        img.data[o + 1] = v;
        img.data[o + 2] = v;
        img.data[o + 3] = 255;
      }
    }
    g.putImageData(img, 0, 0);
    noiseTile = c;
  }
  return ctx.createPattern(noiseTile, "repeat");
}

/**
 * The tone map: 1 - e^(-x). Two halos that meet never reach white, they reach the colour two
 * lights make together, which is what the eye expects and what the hardware does.
 */
function toneMap(value: number): number {
  return 1 - Math.exp(-value);
}

export type StageMode = "effect" | "lights";

export interface StageProps {
  zones: PlacedZone[];
  landmarks: Landmark[];
  devices: DeviceInfo[] | null;
  /** What is picked. One part is the one with the handles; more are outlined. */
  selection: Sel[];
  onSelect: (sel: Sel[]) => void;
  /** A change to one or more zones, as one undo step. */
  onChangeZones: (patches: { index: number; patch: Partial<PlacedZone> }[]) => void;
  onChangeLandmark: (index: number, patch: Partial<Landmark>) => void;
  /** A change to zones and landmarks together, as one undo step: what a drag of a mixed pick or of a case commits. */
  onChangeParts: (
    zones: { index: number; patch: Partial<PlacedZone> }[],
    landmarks: { index: number; patch: Partial<Landmark> }[],
  ) => void;
  onDrop: (deviceId: string, zoneIndex: number, x: number, y: number) => void;
  /** Told when a press or a drag starts and ends, so the page can hold off replacing the layout or scrolling. */
  onDraggingChange?: (dragging: boolean) => void;
  /**
   * Told the live geometry of the zone being dragged, at most once per animation frame, so the
   * inspector can follow the pointer without the drag itself re-rendering the canvas.
   */
  onLiveChange?: (index: number, zone: PlacedZone) => void;
  /** The layout the view belongs to: the zoom and the pan are remembered per layout. */
  layoutId?: string;
  dropPayloadType: string;
  /**
   * The running effect. The editor draws it itself when the daemon's preview is not connected,
   * so the desk is never an empty grid.
   */
  effect?: { id: string; props: PreviewValues } | null;
  /**
   * True when the effect reads the screen or the audio: the editor cannot draw those, so the
   * daemon's frames are the only honest picture of them.
   */
  effectNeedsInput?: boolean;
  /** How each LED reads the picture: nearest, bilinear or the mean over its own rectangle. */
  sampling?: SamplingMode;
  /** The zone being identified: its outline flashes so the eye finds it. */
  identifying?: number | null;
  /** Effect (the picture under the parts) or lights (the parts as they shine). */
  mode: StageMode;
  onMode: (mode: StageMode) => void;
  /** The unit every size is read in. */
  unit: LengthUnit;
  /** Parts the onboarding points at: they glow, so the eye finds them. */
  pulse?: Sel[];
  /**
   * The LED the setup's walker has lit, so the desk can show where it is: the reader matches the
   * light in the room with the cell on the drawing.
   */
  litLed?: { deviceId: string; zoneIndex: number; led: number; count?: number } | null;
  /**
   * The onboarding's spotlight: the parts being asked about. Every other part is dimmed to a quiet
   * outline (no light, no halo, no label). `null` leaves every part as it is.
   */
  spotlight?: Sel[] | null;
  /** Told which part the pointer is over, so the panel can name it. */
  onHoverPart?: (sel: Sel | null) => void;
  /** Reference outlines behind the case (a small, a mid and a full tower), as guides, not a picker. */
  guides?: CaseGuide[];
  /** The locked case shows a small lock that unlocks it (the onboarding). */
  unlockBadge?: boolean;
  /**
   * The case form of an old layout, with no case box of its own yet. The box is drawn from it
   * so the reader still sees their case; the daemon migrates it.
   */
  caseForm?: CaseForm;
  /**
   * The controls that sit in the stage's own toolbar, beside the canvas mode: the page puts
   * there what it wants next to the desk. Nothing is drawn over the desk itself.
   */
  toolbarExtra?: ReactNode;
  /**
   * Whether the line under the desk says what the cells are. A page that has no room for it
   * turns it off: the words are then the canvas mode's own tooltip.
   */
  showLegend?: boolean;
}

/** The hit-test's name for the kind of pointer an event came from. */
function pointerKind(type: string): PointerKind {
  return type === "touch" ? "touch" : type === "pen" ? "pen" : "mouse";
}

/** A reference outline behind the case, in canvas units. */
export interface CaseGuide {
  id: string;
  /** Two quiet lines of text: the name and the size. */
  name: string;
  size: string;
  x: number;
  y: number;
  width: number;
  height: number;
  /** The canvas x the names line up to, left of every outline: three names, one column. */
  labelX: number;
  /** Snaps the case to this outline. */
  onPick: () => void;
}

/** The device accent of a zone, or -1 when the device is not in the list any more. */
function accentOf(devices: DeviceInfo[] | null, deviceId: string): number {
  const i = devices?.findIndex((d) => d.id === deviceId) ?? -1;
  return i < 0 ? -1 : i % ACCENTS.length;
}

/** The axis-aligned box of a zone's corners, in canvas units. */
function boundsOf(z: PlacedZone): Rect {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const c of zoneCorners(z)) {
    if (c.x < minX) minX = c.x;
    if (c.x > maxX) maxX = c.x;
    if (c.y < minY) minY = c.y;
    if (c.y > maxY) maxY = c.y;
  }
  return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}

/** The same for a landmark. */
function landmarkBounds(l: Landmark): Rect {
  const rad = (l.rotation_deg * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const cx = l.x + l.width / 2;
  const cy = l.y + l.height / 2;
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const [dx, dy] of (
    [
      [-l.width / 2, -l.height / 2],
      [l.width / 2, -l.height / 2],
      [l.width / 2, l.height / 2],
      [-l.width / 2, l.height / 2],
    ] as const
  )) {
    const x = cx + dx * cos - dy * sin;
    const y = cy + dx * sin + dy * cos;
    minX = Math.min(minX, x);
    maxX = Math.max(maxX, x);
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
  }
  return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}

/** The part a zone is, and which way it points. */
function partOf(z: PlacedZone, devices: DeviceInfo[] | null, info: ZoneInfo | null): PartKind {
  if (z.part) return z.part.kind;
  if (!info) return "generic";
  return partForDevice(devices?.find((d) => d.id === z.device_id)?.kind ?? "other", info);
}

/**
 * The live preview with every placed part drawn over it. Parts move, resize and rotate by
 * pointer (or by arrow keys, with Shift for ten millimetres).
 *
 * A drag never re-renders React: the pointer move writes the part's geometry straight onto the
 * element and repaints the LED overlay, and the geometry is committed to the layout once, when
 * the pointer goes up. The canvas can be zoomed and panned; that is a view transform only and
 * never reaches the saved layout.
 *
 * Two ways of seeing the desk: the effect under the parts (the cells the engine samples, in
 * their own colours) and the lights themselves (each LED a dot with a halo that stays inside
 * its own part). The second is approximate, and the canvas mode's tooltip says so.
 */
export function PreviewStage({
  zones,
  landmarks,
  devices,
  selection,
  onSelect,
  onChangeZones,
  onChangeLandmark,
  onChangeParts,
  onDrop,
  onDraggingChange,
  onLiveChange,
  layoutId,
  dropPayloadType,
  effect = null,
  effectNeedsInput = false,
  sampling = "nearest",
  identifying = null,
  mode,
  onMode,
  unit,
  caseForm = "none",
  pulse = [],
  litLed = null,
  spotlight = null,
  onHoverPart,
  guides = [],
  unlockBadge = false,
  toolbarExtra = null,
  showLegend = true,
}: StageProps) {
  const stageRef = useRef<HTMLDivElement>(null);
  const previewRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  /** The mark on the LED the walker has lit. */
  const litMarkerRef = useRef<HTMLSpanElement>(null);
  const frameEl = useRef<HTMLDivElement | null>(null);
  const svgRef = useRef<SVGSVGElement | null>(null);
  const gRef = useRef<SVGGElement | null>(null);
  const guideEls = useRef<{ v: HTMLSpanElement | null; h: HTMLSpanElement | null; v2: HTMLSpanElement | null }>({
    v: null,
    h: null,
    v2: null,
  });
  const readoutRef = useRef<HTMLDivElement | null>(null);
  const editorLayerRef = useRef<HTMLDivElement | null>(null);
  const editorPicRef = useRef<Picture | null>(null);
  const frameDataRef = useRef<Frame | null>(null);
  const dragRef = useRef<Drag | null>(null);
  const pendingRef = useRef<Pending | null>(null);
  /** The pointer that owns the press or drag in progress: no other pointer may steer it. */
  const activePointer = useRef<number | null>(null);
  /** The part the pointer is over, so the hover only re-renders when it changes. */
  const hoverRef = useRef<Sel | null>(null);
  const marqueeRef = useRef<Box | null>(null);
  const panRef = useRef<{ start: Point; orig: View } | null>(null);
  const liveRef = useRef<PlacedZone[]>(zones.map((z) => ({ ...z })));
  const liveLmRef = useRef<Landmark[]>(landmarks.map((l) => ({ ...l })));
  const zoneEls = useRef<(HTMLDivElement | null)[]>([]);
  const lmEls = useRef<(HTMLDivElement | null)[]>([]);
  const paintQueued = useRef(false);
  const liveRaf = useRef<number | null>(null);
  const zoomRaf = useRef<number | null>(null);
  const viewCommit = useRef<ReturnType<typeof setTimeout> | null>(null);
  const layoutCache = useRef<Map<string, LedLayout>>(new Map());
  const ledOff = useRef<{ key: string; color: string }>({ key: "", color: "rgba(255,255,255,0.3)" });
  const [hover, setHover] = useState<Sel | null>(null);
  /** The part a finger or a pointer is on: it names itself while it is held. */
  const [pressed, setPressed] = useState<Sel | null>(null);
  /** The one-time hint that says which key shows what. */
  const [hint, setHint] = useState(() => {
    try {
      return window.localStorage.getItem(HINT_KEY) !== "1";
    } catch {
      return true;
    }
  });
  useEffect(() => {
    if (!hint) return;
    const t = setTimeout(() => {
      setHint(false);
      try {
        window.localStorage.setItem(HINT_KEY, "1");
      } catch {
        // Not remembered, still shown this once.
      }
    }, 8000);
    return () => clearTimeout(t);
  }, [hint]);
  /** The part whose size is being typed in. */
  const [sizing, setSizing] = useState<Sel | null>(null);
  const [marquee, setMarquee] = useState<Box | null>(null);
  const [view, setView] = useState<View>({ scale: 1, panX: 0, panY: 0 });
  const viewRef = useRef<View>(view);
  /** True once the reader has zoomed or panned: the view is then theirs, not the desk's. */
  const userView = useRef(false);
  const [viewFor, setViewFor] = useState<string | null>(null);
  const status = usePreviewStatus();
  const reduced = usePrefersReducedMotion();
  const phone = useMediaQuery("(max-width: 720px)");

  /** The daemon's frames are the honest picture of the effect. */
  const daemonLive = status.state === "live";
  const daemonLiveRef = useRef(daemonLive);
  daemonLiveRef.current = daemonLive;
  const samplingRef = useRef<SamplingMode>(sampling);
  samplingRef.current = sampling;
  const modeRef = useRef<StageMode>(mode);
  modeRef.current = mode;
  const selectionRef = useRef<Sel[]>(selection);
  selectionRef.current = selection;
  const zonesRef = useRef(zones);
  zonesRef.current = zones;
  const unitRef = useRef(unit);
  unitRef.current = unit;
  const litRef = useRef(litLed);
  litRef.current = litLed;
  /** Whether a part is dimmed: the spotlight is on and the part is not in it. */
  const spotRef = useRef(spotlight);
  spotRef.current = spotlight;
  const isDim = (sel: Sel): boolean =>
    spotlight !== null && !spotlight.some((s) => s.kind === sel.kind && s.index === sel.index);
  const onHoverPartRef = useRef(onHoverPart);
  onHoverPartRef.current = onHoverPart;
  useEffect(() => {
    onHoverPartRef.current?.(hover);
  }, [hover]);

  /** The picture on the canvas, and the one it is fading out of. */
  const [drawn, setDrawn] = useState<{ id: string; props: PreviewValues } | null>(null);
  const [fading, setFading] = useState<{ id: string; props: PreviewValues } | null>(null);
  const effectId = effect?.id ?? null;
  const [lastEffectId, setLastEffectId] = useState<string | null>(null);
  if (effectId !== lastEffectId) {
    setLastEffectId(effectId);
    setFading(drawn);
    setDrawn(effect ? { id: effect.id, props: effect.props } : null);
  }
  const drawnNow = useMemo(() => {
    if (!drawn) return null;
    if (effect && effect.id === drawn.id) return { id: drawn.id, props: effect.props };
    return drawn;
  }, [drawn, effect]);
  useEffect(() => {
    if (!fading) return;
    const t = setTimeout(() => setFading(null), 320);
    return () => clearTimeout(t);
  }, [fading]);
  const [editorGone, setEditorGone] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setEditorGone(daemonLive), daemonLive ? 400 : 0);
    return () => clearTimeout(t);
  }, [daemonLive]);

  // The view belongs to the layout: it is restored when the editor comes back to it.
  if (layoutId !== viewFor) {
    const remembered = readView(layoutId ?? "");
    const next = remembered ?? FIT_VIEW;
    viewRef.current = next;
    setViewFor(layoutId ?? null);
    setView(next);
  }

  /** The device's description of every zone, found once per layout change. */
  const infos = useMemo(() => {
    const map = new Map<string, ZoneInfo>();
    for (const z of zones) {
      const found = findZoneInfo(devices, z.device_id, z.zone_index);
      if (found) map.set(zoneKey(z), found.zone);
    }
    return map;
  }, [zones, devices]);
  const infosRef = useRef(infos);
  infosRef.current = infos;

  /**
   * The case box of a layout that has a form but no box of its own: it is drawn from the
   * form's measurements, so the reader still sees their case. The daemon migrates it.
   */
  const derivedCase = useMemo(() => {
    if (landmarks.some((l) => l.kind === "case")) return null;
    return caseLandmarkFor(caseForm);
  }, [landmarks, caseForm]);

  /** A zone whose device the daemon does not drive is stale. */
  const missing = useMemo(() => {
    const out = new Set<number>();
    if (!devices) return out;
    zones.forEach((z, i) => {
      const info = findZoneInfo(devices, z.device_id, z.zone_index);
      if (!info || !info.device.connected) out.add(i);
    });
    return out;
  }, [zones, devices]);
  const missingRef = useRef(missing);
  missingRef.current = missing;

  /** Where a zone's LEDs sit, rebuilt only when its size, shape or order changes. */
  const layoutOf = useCallback((z: PlacedZone, info: ZoneInfo): LocalLeds => {
    const sig = `${z.width.toFixed(3)}|${z.height.toFixed(3)}|${JSON.stringify(z.shape)}|${z.reversed}|${info.led_count}|${info.kind.type}|${z.leds?.start ?? -1}|${z.leds?.count ?? 0}`;
    const key = zoneKey(z);
    const hit = layoutCache.current.get(key);
    if (hit && hit.sig === sig) return hit.local;
    const local = zoneLocalLeds(z, info);
    layoutCache.current.set(key, { sig, local });
    return local;
  }, []);

  /** Writes the selection frame onto its element: the part's box and which edge handles fit. */
  const applyFrame = useCallback((z: PlacedZone | Landmark, v: View) => {
    const el = frameEl.current;
    const stage = stageRef.current;
    if (!el || !stage) return;
    const base = stage.clientWidth / CANVAS_W;
    const k = base * v.scale;
    el.style.left = `${(z.x + v.panX) * k}px`;
    el.style.top = `${(z.y + v.panY) * k}px`;
    el.style.width = `${z.width * k}px`;
    el.style.height = `${z.height * k}px`;
    el.style.transform = `rotate(${z.rotation_deg}deg)`;
    el.dataset.narrowX = z.width * k < EDGE_HANDLE_MIN_PX ? "true" : "false";
    el.dataset.narrowY = z.height * k < EDGE_HANDLE_MIN_PX ? "true" : "false";
  }, []);

  /** Writes one part's geometry onto its element. No React involved. */
  const applyGeom = useCallback(
    (sel: Sel, v: View) => {
      const stage = stageRef.current;
      const base = (stage?.clientWidth ?? CANVAS_W) / CANVAS_W;
      const k = base * v.scale;
      const el = sel.kind === "zone" ? zoneEls.current[sel.index] : lmEls.current[sel.index];
      const obj = sel.kind === "zone" ? liveRef.current[sel.index] : liveLmRef.current[sel.index];
      if (!obj) return;
      if (el) {
        el.style.left = `${(obj.x + v.panX) * k}px`;
        el.style.top = `${(obj.y + v.panY) * k}px`;
        el.style.width = `${obj.width * k}px`;
        el.style.height = `${obj.height * k}px`;
        el.style.transform = `rotate(${obj.rotation_deg}deg)`;
      }
      // The frame follows the one part that is picked, a landmark as well as a zone.
      const only = selectionRef.current.length === 1 ? selectionRef.current[0] : undefined;
      if (only && only.kind === sel.kind && only.index === sel.index) applyFrame(obj, v);
    },
    [applyFrame],
  );

  /** The colour of an unlit dot, read from the token so it follows the theme. */
  const unlitColor = useCallback((): string => {
    const stage = stageRef.current;
    if (!stage) return ledOff.current.color;
    const key = `${document.documentElement.getAttribute("data-theme") ?? ""}|${window.matchMedia("(prefers-color-scheme: light)").matches}`;
    if (ledOff.current.key !== key) {
      const v = getComputedStyle(stage).getPropertyValue("--dot-unlit").trim();
      ledOff.current = { key, color: v || "rgba(255,255,255,0.3)" };
    }
    return ledOff.current.color;
  }, []);

  /**
   * Repaints the LED overlay. Two ways of showing the same lights:
   *
   *   effect  every LED's own cell is filled with the colour it reads from the picture. No
   *           glow: this is the engine's geometry, drawn as it is.
   *   lights  every LED is a dot in its colour with a halo that stays inside its own part. The
   *           overlaps are tone-mapped and dithered, so nothing clips to white and nothing
   *           bands.
   */
  const paintOverlay = useCallback(() => {
    const ov = overlayRef.current;
    const stage = stageRef.current;
    if (!ov || !stage) return;
    const octx = ov.getContext("2d");
    if (!octx) return;
    const v = viewRef.current;
    const dpr = ov.width / Math.max(1, stage.clientWidth);
    const k = (ov.width / CANVAS_W) * v.scale;
    const ox = v.panX * k;
    const oy = v.panY * k;
    octx.setTransform(1, 0, 0, 1, 0, 0);
    octx.clearRect(0, 0, ov.width, ov.height);
    const frame = frameDataRef.current;
    const pic = daemonLiveRef.current
      ? frame
        ? { width: frame.width, height: frame.height, data: frame.rgb, stride: 3 }
        : null
      : editorPicRef.current;
    const modeNow = modeRef.current;
    const geom = liveRef.current;
    const unlit = unlitColor();
    const ringDark = "rgba(0, 0, 0, 0.7)";
    const pattern = modeNow === "lights" ? noisePattern(octx) : null;
    const lit = litRef.current;
    const marker = litMarkerRef.current;
    if (marker) {
      marker.hidden = true;
      delete marker.dataset.run;
    }
    const kCss = k / dpr;
    for (let i = 0; i < geom.length; i++) {
      const z = geom[i];
      if (!z || !z.visible) continue;
      const info = infosRef.current.get(zoneKey(z));
      if (!info) continue;
      // The spotlight is on and this part is not in it: a quiet outline, no light drawn.
      const spot = spotRef.current;
      if (spot && !spot.some((x) => x.kind === "zone" && x.index === i)) continue;
      // Which of this placement's LEDs is the one the walker has lit.
      const litAt =
        lit && lit.deviceId === z.device_id && lit.zoneIndex === z.zone_index
          ? z.leds
            ? lit.led - z.leds.start
            : lit.led
          : -1;
      // How many LEDs from there are lit (a whole fan, when the identify asks for it).
      const litTo = litAt >= 0 ? litAt + Math.max(1, lit?.count ?? 1) : -1;
      const local = layoutOf(z, info);
      const t = zoneTransform(z);
      // Whether this part's own body can carry the light of its LEDs. Only the compact parts
      // (a fan, a stick, a card, a keyboard) are lit from within; a strip, an edge or a box is
      // only its LEDs and their halos along the line.
      const bodyLight = COMPACT_KINDS.has(partOf(z, devices, info));
      // A zone whose device is not connected has no LEDs lit: its dots stay dark.
      const off = missingRef.current.has(i);
      const count = local.shape === "point" ? Math.min(1, local.leds.length) : local.leds.length;
      const isPoint = local.shape === "point";
      const minPx = isPoint ? 4 : 1.4;
      const radius = clamp(
        (isPoint ? local.radius * 0.6 : local.radius) * k,
        minPx * dpr,
        (isPoint ? 8 : 9) * dpr * Math.max(1, v.scale * 0.5),
      );
      /** The box round the lit run, when the identify lights more than one LED. */
      let run: [number, number, number, number] | null = null;
      if (modeNow === "effect") {
        // The cells the engine samples, in the colours they read. Each cell is drawn as a
        // crisp outline filled with its colour, so the picture shows through and the engine's
        // own geometry is visible. No glow.
        for (let j = 0; j < count; j++) {
          const led = local.leds[j];
          if (!led) continue;
          const cell = canvasCell(t, led);
          const x = cell.cx * k + ox;
          const y = cell.cy * k + oy;
          if (x + cell.bounds[2] * k < 0 || y + cell.bounds[3] * k < 0 || x > ov.width || y > ov.height) continue;
          const c = pic && !off ? sampleLed(pic, cell, samplingRef.current) : null;
          octx.save();
          octx.translate(x, y);
          octx.rotate((z.rotation_deg * Math.PI) / 180);
          const w = Math.max(1, led.w * k);
          const h = Math.max(1, led.h * k);
          // The cell is the LED's own ground: its colour, solid, with a dark line round it so
          // two cells that touch read as two tiles and not as one soft patch. The cells are
          // drawn in the order of the LEDs, so where they overlap the later one simply covers
          // the earlier one: nothing is blended and nothing goes see-through.
          octx.fillStyle = c ? `rgb(${c[0]} ${c[1]} ${c[2]})` : unlit;
          octx.fillRect(-w / 2, -h / 2, w, h);
          octx.lineWidth = Math.max(1, dpr);
          octx.strokeStyle = "rgba(0, 0, 0, 0.55)";
          octx.strokeRect(-w / 2, -h / 2, w, h);
          if (litAt >= 0 && j >= litAt && j < litTo) {
            // The LED the walker has lit: the mark on the desk, so the reader can match it with
            // the light in the room.
            if (litTo - litAt === 1) markLit(marker, cell, led, v, kCss);
            else run = growRun(run, cell);
          }
          octx.restore();
        }
        if (run) markRun(marker, run, v, kCss);
        continue;
      }
      // Lights mode: the part is lit from within. The light falls on the part's own body, the
      // halo around each LED is a little over one LED's pitch, and the LED itself is its own
      // colour with a hot centre. All of it is clipped to the part plus a small margin, so no
      // light ever leaves the part it belongs to.
      const horizontal = z.width >= z.height;
      const pitch = firstPitch(local, horizontal);
      const haloR = clamp(pitch * k * 1.25, 3 * dpr, 70 * dpr);
      octx.save();
      // The halo is clipped to the part's own outline plus a margin: no light ever leaves the
      // part it belongs to. The margin is at least the halo's own reach, so the glow fades out
      // before the clip and no hard edge is left behind.
      partPath(octx, z, k, ox, oy, Math.max(HALO_MARGIN_PX * dpr, haloR + dpr));
      octx.clip();
      // The colours of this part's LEDs, and their average: the body of the part is tinted with
      // it, so a fan or a stick looks lit from within.
      const colours: RGB[] = [];
      for (let j = 0; j < count; j++) {
        const led = local.leds[j];
        if (!led) continue;
        const cell = canvasCell(t, led);
        if (pic && !off) colours.push(sampleLed(pic, cell, samplingRef.current));
      }
      const avg: RGB = colours.length
        ? [
            Math.round(colours.reduce((a, c) => a + c[0], 0) / colours.length),
            Math.round(colours.reduce((a, c) => a + c[1], 0) / colours.length),
            Math.round(colours.reduce((a, c) => a + c[2], 0) / colours.length),
          ]
        : [0, 0, 0];
      // The part's own drawing takes the colour of its light, so the glyph is lit too.
      if (avg[0] + avg[1] + avg[2] > 0) {
        const el = zoneEls.current[i];
        const lit = `rgb(${avg[0]} ${avg[1]} ${avg[2]})`;
        if (el && el.dataset.lit !== lit) {
          el.dataset.lit = lit;
          el.style.setProperty("--lit", lit);
        }
      }
      for (let j = 0; j < count; j++) {
        const led = local.leds[j];
        if (!led) continue;
        const cell = canvasCell(t, led);
        if (litAt >= 0 && j >= litAt && j < litTo && litTo - litAt > 1) run = growRun(run, cell);
        const x = cell.cx * k + ox;
        const y = cell.cy * k + oy;
        if (x + haloR < 0 || y + haloR < 0 || x - haloR > ov.width || y - haloR > ov.height) continue;
        const c = pic && !off ? sampleLed(pic, cell, samplingRef.current) : null;
        if (!c) {
          octx.beginPath();
          octx.arc(x, y, radius + dpr * 1.25, 0, Math.PI * 2);
          octx.fillStyle = ringDark;
          octx.fill();
          octx.beginPath();
          octx.arc(x, y, radius, 0, Math.PI * 2);
          octx.fillStyle = unlit;
          octx.fill();
          continue;
        }
        const rgb = `${c[0]} ${c[1]} ${c[2]}`;
        const peak = Math.max(c[0], c[1], c[2]) / 255;
        // The halo: the light around the LED, tone-mapped so two lights never make white.
        const alpha = toneMap(0.45 + peak * 1.6);
        const grad = octx.createRadialGradient(x, y, 0, x, y, haloR);
        grad.addColorStop(0, `rgba(${rgb} / ${alpha})`);
        grad.addColorStop(0.35, `rgba(${rgb} / ${alpha * 0.5})`);
        grad.addColorStop(1, `rgba(${rgb} / 0)`);
        octx.beginPath();
        octx.arc(x, y, haloR, 0, Math.PI * 2);
        octx.fillStyle = grad;
        octx.fill();
        if (bodyLight) {
          // The light on the part's own body: the same colour, spread a little past the halo, so
          // the part is lit from where its LED is and fades away from it.
          const reach = haloR * 1.25;
          const body = octx.createRadialGradient(x, y, 0, x, y, reach);
          body.addColorStop(0, `rgba(${rgb} / ${alpha * 0.26})`);
          body.addColorStop(1, `rgba(${rgb} / 0)`);
          octx.beginPath();
          octx.arc(x, y, reach, 0, Math.PI * 2);
          octx.fillStyle = body;
          octx.fill();
        }
        // The LED itself: its own colour, with a hot centre.
        const coreR = Math.max(radius * 1.3, CORE_PX * dpr);
        const hot: RGB = [
          Math.round(c[0] + (255 - c[0]) * 0.6),
          Math.round(c[1] + (255 - c[1]) * 0.6),
          Math.round(c[2] + (255 - c[2]) * 0.6),
        ];
        const core = octx.createRadialGradient(x, y, 0, x, y, coreR);
        core.addColorStop(0, `rgb(${hot[0]} ${hot[1]} ${hot[2]})`);
        core.addColorStop(0.28, `rgb(${c[0]} ${c[1]} ${c[2]})`);
        core.addColorStop(1, `rgba(${rgb} / 0)`);
        octx.beginPath();
        octx.arc(x, y, coreR, 0, Math.PI * 2);
        octx.fillStyle = core;
        octx.fill();
        if (j === litAt && litTo - litAt === 1) {
          // The LED the walker has lit: the mark on the desk, so the reader can match it with
          // the light in the room.
          markLit(marker, cell, led, v, kCss);
        }
      }
      if (run) markRun(marker, run, v, kCss);
      // One least significant bit of noise over the halo, so the gradient does not band.
      if (pattern) {
        octx.globalCompositeOperation = "overlay";
        octx.globalAlpha = 1 / 255;
        octx.fillStyle = pattern;
        octx.fill();
        octx.globalAlpha = 1;
        octx.globalCompositeOperation = "source-over";
      }
      octx.restore();
    }
  }, [layoutOf, unlitColor, devices]);

  const queuePaint = useCallback(() => {
    if (paintQueued.current) return;
    paintQueued.current = true;
    requestAnimationFrame(() => {
      paintQueued.current = false;
      paintOverlay();
    });
  }, [paintOverlay]);

  /** Reads the editor's own draw, so the LED dots can be coloured from it. */
  const readEditorPicture = useCallback(() => {
    const layer = editorLayerRef.current;
    const canvas = layer?.querySelector("canvas") as HTMLCanvasElement | null;
    if (!canvas || !canvas.width || !canvas.height) {
      editorPicRef.current = null;
      return;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) {
      editorPicRef.current = null;
      return;
    }
    try {
      const img = ctx.getImageData(0, 0, canvas.width, canvas.height);
      editorPicRef.current = { width: canvas.width, height: canvas.height, data: img.data, stride: 4 };
    } catch {
      editorPicRef.current = null;
    }
  }, []);

  /** The editor's draw animates on its own clock, so the dots have to follow it. */
  useEffect(() => {
    const editorDraws = !daemonLive && !!effect && !effectNeedsInput;
    if (!editorDraws) return;
    let raf = 0;
    const step = () => {
      readEditorPicture();
      paintOverlay();
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [daemonLive, effect, effectNeedsInput, paintOverlay, readEditorPicture]);

  /** Draws the whole view: the effect, the parts and the LEDs. */
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
        if (geom[i]) applyGeom({ kind: "zone", index: i }, v);
      }
      const lms = liveLmRef.current;
      for (let i = 0; i < lms.length; i++) {
        if (lms[i]) applyGeom({ kind: "landmark", index: i }, v);
      }
      const kk = ((stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W) * v.scale;
      gRef.current?.setAttribute("transform", `scale(${kk}) translate(${v.panX} ${v.panY})`);
      svgRef.current?.style.setProperty("--k", String(kk));
      if (stageRef.current) stageRef.current.dataset.moving = "true";
      queuePaint();
    },
    [applyGeom, queuePaint],
  );

  // Paint each arriving frame: the preview canvas, then the LED dots over it.
  useEffect(() => {
    const off = onFrame((frame) => {
      frameDataRef.current = frame;
      const pv = previewRef.current;
      const ctx = pv?.getContext("2d");
      if (ctx && frame.width === PREVIEW_W && frame.height === PREVIEW_H) {
        const img = ctx.createImageData(PREVIEW_W, PREVIEW_H);
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
        s.w === stage.clientWidth && s.h === stage.clientHeight ? s : { w: stage.clientWidth, h: stage.clientHeight },
      );
      queuePaint();
    };
    size();
    const ro = new ResizeObserver(size);
    ro.observe(stage);
    return () => ro.disconnect();
  }, [queuePaint]);

  // When the layout changes outside a drag, the live geometry and the elements follow it.
  useLayoutEffect(() => {
    const v = viewRef.current;
    if (!dragRef.current) {
      liveRef.current = zones.map((z) => ({ ...z }));
      liveLmRef.current = landmarks.map((l) => ({ ...l }));
      zones.forEach((z, i) => applyGeom({ kind: "zone", index: i }, v));
      landmarks.forEach((_, i) => applyGeom({ kind: "landmark", index: i }, v));
    }
    queuePaint();
  }, [zones, landmarks, applyGeom, queuePaint, stageBox, litLed]);

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

  /** What a drag says in words beside the pointer, in the reader's own unit. */
  const showReadout = useCallback((e: PointerEvent<HTMLDivElement>, text: string) => {
    const el = readoutRef.current;
    const stage = stageRef.current;
    if (!el || !stage) return;
    const box = stage.getBoundingClientRect();
    el.textContent = text;
    el.style.opacity = "1";
    el.style.visibility = "visible";
    el.style.left = `${clamp(e.clientX - box.left + 16, 4, Math.max(4, box.width - 190))}px`;
    el.style.top = `${clamp(e.clientY - box.top + 16, 4, Math.max(4, box.height - 40))}px`;
  }, []);

  /** Shows or hides the alignment guides. */
  const applyGuides = useCallback((snap: { guideX: number | null; guideY: number | null; guideX2: number | null }) => {
    const els = guideEls.current;
    const v = viewRef.current;
    const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
    const k = base * v.scale;
    const set = (el: HTMLSpanElement | null, on: boolean, pos: number | null, axis: "x" | "y") => {
      if (!el) return;
      el.style.opacity = on ? "1" : "0";
      el.style.visibility = on ? "visible" : "hidden";
      if (pos === null) return;
      if (axis === "x") el.style.left = `${(pos + v.panX) * k}px`;
      else el.style.top = `${(pos + v.panY) * k}px`;
    };
    set(els.v, snap.guideX !== null, snap.guideX, "x");
    set(els.v2, snap.guideX2 !== null, snap.guideX2, "x");
    set(els.h, snap.guideY !== null, snap.guideY, "y");
  }, []);

  /** Every part that can be hit, in its live geometry (the drag's, while one is under way). */
  const hitParts = (): HitPart[] => {
    const out: HitPart[] = [];
    liveRef.current.forEach((z, index) => {
      out.push({
        ref: { kind: "zone", index },
        x: z.x,
        y: z.y,
        width: z.width,
        height: z.height,
        rotation_deg: z.rotation_deg,
        isCase: false,
        locked: z.locked === true,
        offline: missingRef.current.has(index),
      });
    });
    liveLmRef.current.forEach((l, index) => {
      if (l.visible === false) return;
      out.push({
        ref: { kind: "landmark", index },
        x: l.x,
        y: l.y,
        width: l.width,
        height: l.height,
        rotation_deg: l.rotation_deg,
        isCase: l.kind === "case",
        locked: l.locked === true,
      });
    });
    return out;
  };

  /** Css pixels per canvas unit, now. */
  const pxPerUnit = (): number =>
    ((stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W) * viewRef.current.scale;

  /** What is under the pointer: the one place a press or a hover is turned into a part. */
  const hitAt = (e: { clientX: number; clientY: number; pointerType: string; altKey: boolean }): HitTarget =>
    hitTest({
      point: toLogical(e),
      parts: hitParts(),
      selection: selectionRef.current,
      k: pxPerUnit(),
      pointer: pointerKind(e.pointerType),
      under: e.altKey,
    });

  /** Starts a move, a resize or a turn. A move starts from `startAt` when the press came earlier. */
  const beginDrag = (
    kind: DragKind,
    sel: Sel,
    e: PointerEvent<HTMLElement>,
    handle: HandleDef | null = null,
    startAt?: Point,
  ) => {
    e.preventDefault();
    const zone = sel.kind === "zone" ? liveRef.current[sel.index] : undefined;
    const landmark = sel.kind === "landmark" ? liveLmRef.current[sel.index] : undefined;
    if (!zone && !landmark) return;
    // A move of a part that is already picked keeps the whole selection: the drag moves them
    // all. A resize or a turn is about one part, so it picks that one.
    const already = selectionRef.current.some((s) => s.kind === sel.kind && s.index === sel.index);
    if (!already || kind !== "move") onSelect([sel]);
    setPressed(sel);
    const members: Sel[] = already && kind === "move" ? selectionRef.current : [sel];
    const origOf = (s: Sel): Orig | null => {
      if (s.kind === "zone") {
        const z = liveRef.current[s.index];
        return z ? { sel: s, zone: { ...z } } : null;
      }
      const l = liveLmRef.current[s.index];
      return l ? { sel: s, landmark: { ...l } } : null;
    };
    const isPrimary = (s: Sel) => s.kind === sel.kind && s.index === sel.index;
    // Every picked part moves together, whatever mix of zones and landmarks the pick holds. A
    // locked part is picked but stays where it is.
    const origs: Orig[] = [];
    for (const m of members) {
      const o = origOf(m);
      if (!o) continue;
      const locked = o.zone?.locked === true || o.landmark?.locked === true;
      if (kind === "move" && locked && !isPrimary(m)) continue;
      origs.push(o);
    }
    if (!origs.some((o) => isPrimary(o.sel))) {
      const o = origOf(sel);
      if (o) origs.push(o);
    }
    // A case carries everything inside it, zones and landmarks, by the middle of each part
    // against the case's own turned box: moving the PC moves the parts in it.
    const riders: Orig[] = [];
    if (kind === "move") {
      const parts = hitParts();
      const taken = (ref: Sel) =>
        origs.some((x) => x.sel.kind === ref.kind && x.sel.index === ref.index) ||
        riders.some((x) => x.sel.kind === ref.kind && x.sel.index === ref.index);
      for (const o of origs) {
        if (o.landmark?.kind !== "case") continue;
        const box = parts.find((p) => p.ref.kind === "landmark" && p.ref.index === o.sel.index);
        if (!box) continue;
        for (const ref of partsInsideCase(box, parts)) {
          const r = origOf(ref);
          if (r && !taken(ref)) riders.push(r);
        }
      }
    }
    dragRef.current = {
      kind,
      sel,
      origs,
      start: startAt ?? toLogical(e),
      handle,
      riders,
    };
    const el = sel.kind === "zone" ? zoneEls.current[sel.index] : lmEls.current[sel.index];
    if (el) el.dataset.dragging = "true";
    if (stageRef.current && kind !== "resize") stageRef.current.style.cursor = "grabbing";
    onDraggingChange?.(true);
    // A move leaves focus on the part pressed (set by the press); a handle drag has no element.
    if (kind !== "move") stageRef.current?.focus({ preventScroll: true });
    if (!stageRef.current?.hasPointerCapture(e.pointerId)) stageRef.current?.setPointerCapture(e.pointerId);
  };

  /** A press on the stage: the hit-test says what it landed on, and what that does follows. */
  const onStagePointerDown = (e: PointerEvent<HTMLDivElement>) => {
    const stage = stageRef.current;
    if (!stage) return;
    // Empty space with the middle button: a pan.
    if (e.button === 1) {
      if (activePointer.current !== null) return;
      activePointer.current = e.pointerId;
      panRef.current = { start: { x: e.clientX, y: e.clientY }, orig: { ...viewRef.current } };
      stage.setPointerCapture(e.pointerId);
      e.preventDefault();
      return;
    }
    if (e.button !== 0) return;
    // A second finger (or any other pointer) never takes over a press or drag in progress. A new
    // primary pointer means the old one is gone (its end event was lost), so it starts afresh.
    if (activePointer.current !== null && activePointer.current !== e.pointerId && !e.isPrimary) return;
    activePointer.current = e.pointerId;
    // Whatever press came before is over: a stale one can never turn this press into a move.
    pendingRef.current = null;
    const target = hitAt(e);
    if (target.kind === "handle") {
      const sel = target.ref as Sel;
      beginDrag(target.handle === "rotate" ? "rotate" : "resize", sel, e, target.handle === "rotate" ? null : target.handle);
      return;
    }
    if (target.kind === "desk") {
      // The empty desk: a marquee picks what it touches.
      onSelect([]);
      setSizing(null);
      const box = stage.getBoundingClientRect();
      marqueeRef.current = { x: e.clientX - box.left, y: e.clientY - box.top, w: 0, h: 0 };
      setMarquee({ ...marqueeRef.current });
      stage.setPointerCapture(e.pointerId);
      return;
    }
    // A part, or the case by its edge or its body.
    const sel = target.ref as Sel;
    e.preventDefault();
    // Focus goes to the part pressed (so Tab continues from it), without scrolling the page.
    const partEl = sel.kind === "zone" ? zoneEls.current[sel.index] : lmEls.current[sel.index];
    (partEl ?? stage).focus({ preventScroll: true });
    const picked = selectionRef.current.some((s) => s.kind === sel.kind && s.index === sel.index);
    if (e.shiftKey && !e.altKey) {
      // Shift-click picks one more part, or lets one go.
      onSelect(
        picked
          ? selectionRef.current.filter((s) => !(s.kind === sel.kind && s.index === sel.index))
          : [...selectionRef.current, sel],
      );
      return;
    }
    if (!picked) onSelect([sel]);
    setPressed(sel);
    pendingRef.current = {
      sel,
      pointerId: e.pointerId,
      x: e.clientX,
      y: e.clientY,
      start: toLogical(e),
      thresholdPx: dragThresholdPx(pointerKind(e.pointerType)),
      locked: target.locked,
      collapse: picked && selectionRef.current.length > 1,
    };
    // The page holds off (on a phone it would scroll the inspector into view, and the part
    // would run away under the finger) until the press is over.
    onDraggingChange?.(true);
    stage.setPointerCapture(e.pointerId);
  };

  /** Lights the handle the pointer is over (data-hover), and none other. */
  const setHandleHover = (id: string | null) => {
    const frame = frameEl.current;
    if (!frame) return;
    for (const h of frame.querySelectorAll<HTMLElement>("[data-handle]")) {
      if (h.dataset.handle === id) h.dataset.hover = "true";
      else delete h.dataset.hover;
    }
  };

  /** Hover and cursor, from the same hit-test a press uses. */
  const onStageHover = (e: PointerEvent<HTMLDivElement>) => {
    const stage = stageRef.current;
    if (!stage || e.pointerType === "touch") return;
    const target = hitAt(e);
    let cursor = "";
    let over: Sel | null = null;
    if (target.kind === "handle") {
      const part = target.ref.kind === "zone" ? liveRef.current[target.ref.index] : liveLmRef.current[target.ref.index];
      cursor = target.handle === "rotate" ? "grab" : handleCursor(target.handle, part?.rotation_deg ?? 0);
    } else if (target.kind !== "desk") {
      over = target.ref as Sel;
      cursor = target.locked ? "not-allowed" : "grab";
    }
    stage.style.cursor = cursor;
    setHandleHover(target.kind === "handle" ? (target.handle === "rotate" ? "rotate" : target.handle.id) : null);
    const was = hoverRef.current;
    if (was?.kind !== over?.kind || was?.index !== over?.index) {
      hoverRef.current = over;
      setHover(over);
    }
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
    // Only the pointer that began the press or drag steers it.
    if (activePointer.current !== null && e.pointerId !== activePointer.current) return;
    const pan = panRef.current;
    if (pan) {
      const box = stageRef.current?.getBoundingClientRect();
      if (!box) return;
      const base = box.width / CANVAS_W;
      const k = base * pan.orig.scale;
      userView.current = true;
      const next: View = {
        ...pan.orig,
        panX: pan.orig.panX + (e.clientX - pan.start.x) / k,
        panY: pan.orig.panY + (e.clientY - pan.start.y) / k,
      };
      applyView(clampView(next));
      return;
    }
    // A press on a part becomes a drag only once the pointer has travelled a few pixels. Until
    // then it is a click that has not finished: nothing moves, and no undo step is made.
    const pend = pendingRef.current;
    if (pend && pend.pointerId === e.pointerId && !dragRef.current) {
      if (Math.hypot(e.clientX - pend.x, e.clientY - pend.y) < pend.thresholdPx) return;
      // A locked part is picked by the press and does not move: the press stays a click.
      if (pend.locked) return;
      pendingRef.current = null;
      beginDrag("move", pend.sel, e, null, pend.start);
    }
    // The marquee: a rubber band on the empty desk that picks everything under it.
    const mq = marqueeRef.current;
    if (mq && !dragRef.current) {
      const box = stageRef.current?.getBoundingClientRect();
      if (!box) return;
      const px = e.clientX - box.left;
      const py = e.clientY - box.top;
      const next = {
        x: Math.min(mq.x, px),
        y: Math.min(mq.y, py),
        w: Math.abs(px - mq.x),
        h: Math.abs(py - mq.y),
      };
      marqueeRef.current = next;
      setMarquee(next);
      return;
    }
    const d = dragRef.current;
    if (!d) {
      if (!pendingRef.current) onStageHover(e);
      return;
    }
    const p = toLogical(e);
    const o = d.origs.find((x) => x.sel.index === d.sel.index && x.sel.kind === d.sel.kind);
    if (!o) return;
    const dx = p.x - d.start.x;
    const dy = p.y - d.start.y;
    const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
    const threshold = SNAP_PIXELS / (base * viewRef.current.scale);
    const u = unitRef.current;
    // The parts that are not moving (nor carried): they are what a part snaps to.
    const moving = new Set(
      [...d.origs, ...d.riders].map((x) => (x.sel.kind === "zone" ? x.sel.index : -1)),
    );
    const others = liveRef.current
      .filter((_, i) => !moving.has(i))
      .map((z) => boundsOf(z));
    // The case's panels: a part can sit against the top, the floor, the front or the back. A
    // case that is itself moving has no panels to snap to: they would move with it.
    const caseMoves = d.origs.some((x) => x.landmark?.kind === "case");
    const caseBox = caseMoves ? null : (liveLmRef.current.find((l) => l.kind === "case") ?? derivedCase);
    const panels = caseBox
      ? {
          x: [caseBox.x, caseBox.x + caseBox.width / 2, caseBox.x + caseBox.width],
          y: [caseBox.y, caseBox.y + caseBox.height / 2, caseBox.y + caseBox.height],
        }
      : { x: [], y: [] };

    if (d.kind === "move") {
      const src = o.zone ?? o.landmark;
      if (!src) return;
      const want = {
        x: clamp(src.x + dx, -src.width / 2, CANVAS_W - src.width / 2),
        y: clamp(src.y + dy, -src.height / 2, CANVAS_H - src.height / 2),
        width: src.width,
        height: src.height,
      };
      const free = e.ctrlKey || e.metaKey || e.altKey;
      const snap = snapMove(want, [CANVAS_BOX, ...others], panels, threshold, free);
      // The equal spacing: the gaps on either side become the same.
      const eq = free ? null : equalSpacing(want, others, threshold);
      const nextX = eq?.x ?? snap.x;
      const nextY = eq?.y ?? snap.y;
      const deltaX = nextX - src.x;
      const deltaY = nextY - src.y;
      // Every picked part moves by the same amount, and a case carries what is inside it. Each
      // goes from where it started, not from where it is, so every frame of the drag is exact.
      for (const orig of [...d.origs, ...d.riders]) {
        if (orig.zone) {
          liveRef.current[orig.sel.index] = { ...orig.zone, x: orig.zone.x + deltaX, y: orig.zone.y + deltaY };
        } else if (orig.landmark) {
          liveLmRef.current[orig.sel.index] = {
            ...orig.landmark,
            x: orig.landmark.x + deltaX,
            y: orig.landmark.y + deltaY,
          };
        }
      }
      const primary = liveRef.current[d.sel.index];
      applyGuides({
        guideX: eq?.lineA ?? snap.guideX,
        guideY: snap.guideY,
        guideX2: eq?.lineB ?? null,
      });
      const where = eq
        ? " · even spacing"
        : snap.gapX !== null && snap.gapX < 0.05
          ? " · on the guide"
          : "";
      const pos = `${formatUnits(nextX, u)} · ${formatUnits(nextY, u)}${where}`;
      showReadout(e, pos);
      for (const orig of [...d.origs, ...d.riders]) applyGeom(orig.sel, viewRef.current);
      if (d.sel.kind === "zone" && primary) notifyLive(d.sel.index);
      queuePaint();
      return;
    }

    if (d.kind === "resize" && d.handle) {
      const src = o.zone ?? o.landmark;
      if (!src) return;
      let q = { x: clamp(p.x, 0, CANVAS_W), y: clamp(p.y, 0, CANVAS_H) };
      let guideX: number | null = null;
      let guideY: number | null = null;
      const square = Math.abs(Math.sin((src.rotation_deg * Math.PI) / 180)) < 1e-6 && Math.cos((src.rotation_deg * Math.PI) / 180) > 0;
      if (square && !(e.ctrlKey || e.metaKey || e.altKey)) {
        if (d.handle.sx !== 0) {
          const lines = [...others.flatMap((z) => zoneLines(z, "x")), ...panels.x];
          const line = nearestLine(q.x, lines, threshold);
          guideX = line;
          q = { ...q, x: line ?? q.x };
        }
        if (d.handle.sy !== 0) {
          const lines = [...others.flatMap((z) => zoneLines(z, "y")), ...panels.y];
          const line = nearestLine(q.y, lines, threshold);
          guideY = line;
          q = { ...q, y: line ?? q.y };
        }
      }
      const next = resizeBy(src, d.handle, q, e.shiftKey, e.altKey);
      if (d.sel.kind === "zone") {
        const cur = liveRef.current[d.sel.index];
        if (!cur) return;
        liveRef.current[d.sel.index] = { ...cur, ...next };
        applyGeom(d.sel, viewRef.current);
        notifyLive(d.sel.index);
      } else {
        const cur = liveLmRef.current[d.sel.index];
        if (!cur) return;
        liveLmRef.current[d.sel.index] = { ...cur, ...next };
        applyGeom(d.sel, viewRef.current);
      }
      applyGuides({ guideX, guideY, guideX2: null });
      showReadout(e, `${formatUnits(next.width, u)} × ${formatUnits(next.height, u)}${e.shiftKey ? " · locked" : ""}`);
      queuePaint();
      return;
    }

    // The rotate handle.
    const src = o.zone ?? o.landmark;
    if (!src) return;
    const cx = src.x + src.width / 2;
    const cy = src.y + src.height / 2;
    let deg = (Math.atan2(p.y - cy, p.x - cx) * 180) / Math.PI + 90;
    deg = ((deg % 360) + 360) % 360;
    if (e.shiftKey) deg = Math.round(deg / 15) * 15;
    const rounded = Math.round((deg % 360) * 10) / 10;
    if (d.sel.kind === "zone") {
      const cur = liveRef.current[d.sel.index];
      if (!cur) return;
      liveRef.current[d.sel.index] = { ...cur, rotation_deg: rounded };
      applyGeom(d.sel, viewRef.current);
      notifyLive(d.sel.index);
    } else {
      const cur = liveLmRef.current[d.sel.index];
      if (!cur) return;
      liveLmRef.current[d.sel.index] = { ...cur, rotation_deg: rounded };
      applyGeom(d.sel, viewRef.current);
    }
    applyGuides({ guideX: null, guideY: null, guideX2: null });
    showReadout(e, `${rounded.toFixed(1)}°${e.shiftKey ? " · 15° steps" : ""}`);
    queuePaint();
  };

  /** The end of a press: pointerup, cancel or lost capture, from the pointer that owns it only. */
  const endDrag = (e: PointerEvent<HTMLDivElement>) => {
    if (activePointer.current !== null && e.pointerId !== activePointer.current) return;
    endPress(e);
    activePointer.current = null;
  };

  const endPress = (e: PointerEvent<HTMLDivElement>) => {
    setPressed(null);
    // A press that never became a drag was a click: it picked the part (a click on one of
    // several picked parts picks just that one) and changed nothing, so no undo step is made.
    const pend = pendingRef.current;
    if (pend && pend.pointerId === e.pointerId && !dragRef.current) {
      pendingRef.current = null;
      if (pend.collapse && e.type === "pointerup") onSelect([pend.sel]);
      if (stageRef.current?.hasPointerCapture(e.pointerId)) {
        stageRef.current.releasePointerCapture(e.pointerId);
      }
      onDraggingChange?.(false);
      return;
    }
    const pan = panRef.current;
    if (pan) {
      panRef.current = null;
      if (stageRef.current?.hasPointerCapture(e.pointerId)) {
        stageRef.current.releasePointerCapture(e.pointerId);
      }
      setView(viewRef.current);
      return;
    }
    // The marquee ends: everything under the band is picked.
    const mq = marqueeRef.current;
    if (mq) {
      marqueeRef.current = null;
      setMarquee(null);
      if (mq.w > 4 || mq.h > 4) {
        // The band in canvas units; it picks zones and landmarks alike.
        const v = viewRef.current;
        const kk = pxPerUnit();
        const band = { x: mq.x / kk - v.panX, y: mq.y / kk - v.panY, width: mq.w / kk, height: mq.h / kk };
        onSelect(marqueePick(band, hitParts()).map((r) => r as Sel));
      }
      if (stageRef.current?.hasPointerCapture(e.pointerId)) {
        stageRef.current.releasePointerCapture(e.pointerId);
      }
      return;
    }
    const d = dragRef.current;
    if (!d) return;
    dragRef.current = null;
    const el = d.sel.kind === "zone" ? zoneEls.current[d.sel.index] : lmEls.current[d.sel.index];
    if (el) delete el.dataset.dragging;
    onDraggingChange?.(false);
    applyGuides({ guideX: null, guideY: null, guideX2: null });
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
    // One drag is one change, so one undo step: every zone and landmark it moved, together.
    const zonePatches: { index: number; patch: Partial<PlacedZone> }[] = [];
    const landmarkPatches: { index: number; patch: Partial<Landmark> }[] = [];
    for (const orig of [...d.origs, ...d.riders]) {
      const src = orig.zone ?? orig.landmark;
      const now = orig.sel.kind === "zone" ? liveRef.current[orig.sel.index] : liveLmRef.current[orig.sel.index];
      if (!src || !now) continue;
      const patch: Partial<PlacedZone> & Partial<Landmark> = {};
      if (Math.abs(now.x - src.x) > 1e-6) patch.x = now.x;
      if (Math.abs(now.y - src.y) > 1e-6) patch.y = now.y;
      if (Math.abs(now.width - src.width) > 1e-6) patch.width = now.width;
      if (Math.abs(now.height - src.height) > 1e-6) patch.height = now.height;
      if (Math.abs(now.rotation_deg - src.rotation_deg) > 1e-6) patch.rotation_deg = now.rotation_deg;
      if (Object.keys(patch).length === 0) continue;
      if (orig.sel.kind === "zone") zonePatches.push({ index: orig.sel.index, patch });
      else landmarkPatches.push({ index: orig.sel.index, patch });
    }
    if (zonePatches.length > 0 || landmarkPatches.length > 0) onChangeParts(zonePatches, landmarkPatches);
    if (stageRef.current) stageRef.current.style.cursor = "";
  };

  // The keys: the arrows nudge, Enter opens the size, L and E change the mode.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const stage = stageRef.current;
      const onCanvas = !!target && !!stage && stage.contains(target);
      const onNothing = !target || target === document.body;
      if (target && target.closest("button, input, select, textarea")) return;
      // The mode keys work everywhere on the page, as long as the reader is not typing.
      const key = e.key.toLowerCase();
      if (key === "l" && !e.ctrlKey && !e.metaKey && !e.altKey) {
        onMode("lights");
        return;
      }
      if (key === "e" && !e.ctrlKey && !e.metaKey && !e.altKey) {
        onMode("effect");
        return;
      }
      if (!onCanvas && !onNothing) return;
      const sel = selectionRef.current[0];
      if (!sel) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.key === "Enter") {
        e.preventDefault();
        setSizing(sel);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setSizing(null);
        onSelect([]);
        return;
      }
      const z = sel.kind === "zone" ? zonesRef.current[sel.index] : undefined;
      const lm = sel.kind === "landmark" ? liveLmRef.current[sel.index] : undefined;
      const step = ((e.shiftKey ? NUDGE_MM_BIG : NUDGE_MM) / MM_PER_UNIT);
      if (z) {
        const patch: Partial<PlacedZone> = {};
        if (e.key === "ArrowLeft") patch.x = clamp(z.x - step, -z.width / 2, CANVAS_W - z.width / 2);
        else if (e.key === "ArrowRight") patch.x = clamp(z.x + step, -z.width / 2, CANVAS_W - z.width / 2);
        else if (e.key === "ArrowUp") patch.y = clamp(z.y - step, -z.height / 2, CANVAS_H - z.height / 2);
        else if (e.key === "ArrowDown") patch.y = clamp(z.y + step, -z.height / 2, CANVAS_H - z.height / 2);
        else if (e.key === "Delete" || e.key === "Backspace") patch.visible = false;
        else return;
        e.preventDefault();
        onChangeZones([{ index: sel.index, patch }]);
        return;
      }
      if (lm) {
        const patch: Partial<Landmark> = {};
        if (e.key === "ArrowLeft") patch.x = lm.x - step;
        else if (e.key === "ArrowRight") patch.x = lm.x + step;
        else if (e.key === "ArrowUp") patch.y = lm.y - step;
        else if (e.key === "ArrowDown") patch.y = lm.y + step;
        else return;
        e.preventDefault();
        onChangeLandmark(sel.index, patch);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onChangeZones, onChangeLandmark, onMode, onSelect]);

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

  // The view is written back to React when the gesture settles, so the page keeps the zoom.
  const commitView = useCallback((v: View) => {
    if (viewCommit.current) clearTimeout(viewCommit.current);
    viewCommit.current = setTimeout(() => {
      viewCommit.current = null;
      setView(v);
    }, VIEW_COMMIT_MS);
  }, []);

  useEffect(() => {
    if (!layoutId) return;
    if (view.scale === 1 && view.panX === 0 && view.panY === 0) forgetView(layoutId);
    else writeView(layoutId, view);
  }, [layoutId, view]);

  /**
   * Eases the view to a new one, with the house curve.
   */
  const animateTo = useCallback(
    (target: View, ms: number = ZOOM_MS) => {
      if (zoomRaf.current !== null) {
        cancelAnimationFrame(zoomRaf.current);
        zoomRaf.current = null;
      }
      const from = { ...viewRef.current };
      const still =
        reduced ||
        (typeof document !== "undefined" && document.documentElement.getAttribute("data-motion") === "reduced");
      if (still) {
        applyView(target);
        setView(target);
        return;
      }
      const t0 = performance.now();
      const step = () => {
        const raw = Math.min(1, (performance.now() - t0) / ms);
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

  // The wheel zooms around the pointer.
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      userView.current = true;
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
      const lx = px / (base * v.scale) - v.panX;
      const ly = py / (base * v.scale) - v.panY;
      const next = clampView({ scale, panX: px / (base * scale) - lx, panY: py / (base * scale) - ly });
      const notch = !e.ctrlKey && (e.deltaMode === 1 || Math.abs(e.deltaY) >= NOTCH_DELTA);
      if (notch) {
        animateTo(next, WHEEL_MS);
      } else {
        applyView(next);
        commitView(next);
      }
    };
    stage.addEventListener("wheel", onWheel, { passive: false });
    return () => stage.removeEventListener("wheel", onWheel);
  }, [applyView, commitView, animateTo]);

  useEffect(
    () => () => {
      if (zoomRaf.current !== null) cancelAnimationFrame(zoomRaf.current);
    },
    [],
  );

  const zoomBy = (factor: number) => {
    userView.current = true;
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
    userView.current = true;
    animateTo(wholeView(stageBox.w, stageBox.h));
  };

  useEffect(() => {
    if (stageRef.current) delete stageRef.current.dataset.moving;
  }, [view]);

  /** The middle of the parts on the desk, in canvas units: the view opens looking at them. */
  const contentCentre = (): number => {
    let minX = Infinity;
    let maxX = -Infinity;
    for (const z of liveRef.current) {
      minX = Math.min(minX, z.x);
      maxX = Math.max(maxX, z.x + z.width);
    }
    for (const l of liveLmRef.current) {
      minX = Math.min(minX, l.x);
      maxX = Math.max(maxX, l.x + l.width);
    }
    if (!Number.isFinite(minX) || !Number.isFinite(maxX)) return CANVAS_W / 2;
    return (minX + maxX) / 2;
  };

  // The stage is measured: until the reader zooms or pans, the desk fills it and the view
  // looks at the parts.
  if (!userView.current && stageBox.w > 0 && stageBox.h > 0) {
    const hero = heroView(stageBox.w, stageBox.h, contentCentre());
    if (Math.abs(hero.scale - view.scale) > 1e-6 || Math.abs(hero.panX - view.panX) > 1e-6) {
      viewRef.current = hero;
      setView(hero);
    }
  }

  const base = (stageBox.w || CANVAS_W) / CANVAS_W;
  const k = base * view.scale;

  /**
   * What the canvas mode's two keys show, for the tooltip on them (R8). It says outright that the
   * lights view is approximate: the key itself reads only "Lights".
   */
  const modeNote =
    "Effect: each cell is one LED, in the colour it reads from the effect. " +
    "Lights is approximate: each LED is a dot in its own colour and its halo stays inside its part, so the picture is close, not exact." +
    (phone ? " Tap anything for its name." : "");

  /** What the cells on the desk are: the line under the desk when there is room for one. */
  const legend =
    (mode === "lights"
      ? "Each LED is a dot in its own colour. The halo stays inside its part, so the picture is close, not exact."
      : "Each cell is one LED, in the colour it reads from the effect.") + (phone ? " Tap anything for its name." : "");

  /** The part that is picked, and the one that is only hovered. */
  const primary = selection[0] ?? null;
  const hovered = pressed ?? hover;
  const selZone = primary?.kind === "zone" ? zones[primary.index] : undefined;
  const selLm = primary?.kind === "landmark" ? landmarks[primary.index] : undefined;

  /** The name of a part: what a person would call it. */
  const nameOf = (sel: Sel): string => {
    if (sel.kind === "landmark") {
      const l = landmarks[sel.index];
      if (!l) return "";
      return l.label?.trim() || PART_LABEL[l.kind];
    }
    const z = zones[sel.index];
    if (!z) return "";
    return placementName(devices, z, zones);
  };

  /** The size of a part, in the reader's unit. */
  const sizeOf = (sel: Sel): string => {
    const o = sel.kind === "zone" ? zones[sel.index] : landmarks[sel.index];
    if (!o) return "";
    return `${formatUnits(o.width, unit)} × ${formatUnits(o.height, unit)}`;
  };

  /** Where a tag sits: just above the part, or below it when there is no room. */
  const tagBox = (sel: Sel): Box => {
    if (sel.kind === "zone") {
      const o = zones[sel.index];
      if (!o) return { x: 0, y: 0, w: 0, h: 0 };
      const b = boundsOf(o);
      return { x: (b.x + view.panX) * k, y: (b.y + view.panY) * k, w: b.width * k, h: b.height * k };
    }
    const o = landmarks[sel.index];
    if (!o) return { x: 0, y: 0, w: 0, h: 0 };
    const b = landmarkBounds(o);
    return { x: (b.x + view.panX) * k, y: (b.y + view.panY) * k, w: b.width * k, h: b.height * k };
  };

  /** The tags: one for the part under the pointer, one for each picked part. They never stack. */
  const tags = useMemo(() => {
    const out: { sel: Sel; text: string; x: number; y: number; w: number; selected: boolean; accent: number }[] = [];
    const taken: Box[] = [];
    const add = (sel: Sel, text: string, selected: boolean) => {
      // A dimmed part carries no label.
      if (spotlight !== null && !spotlight.some((x) => x.kind === sel.kind && x.index === sel.index)) return;
      const b = tagBox(sel);
      const w = Math.ceil(16 + text.length * 7.2);
      const h = 22;
      const x = clamp(b.x, 2, Math.max(2, (stageBox.w || CANVAS_W) - w - 2));
      let y = b.y - h - 6;
      if (y < 2) y = b.y + b.h + 6;
      const box = { x, y, w, h };
      if (taken.some((t) => overlaps(box, t, 2))) return;
      taken.push(box);
      out.push({
        sel,
        text,
        x,
        y,
        w,
        selected,
        accent:
          sel.kind === "zone"
            ? accentOf(devices, zones[sel.index]?.device_id ?? "")
            : -1,
      });
    };
    if (hovered && !selection.some((s) => s.kind === hovered.kind && s.index === hovered.index)) {
      add(hovered, nameOf(hovered), false);
    }
    for (const s of selection) {
      const o = s.kind === "zone" ? zones[s.index] : landmarks[s.index];
      if (!o) continue;
      add(s, sizeOf(s), true);
    }
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hovered, selection, zones, landmarks, devices, unit, view, stageBox.w, spotlight]);

  /** The scale bar: a real length, in the reader's unit. */
  const scaleBar = useMemo(() => {
    if (!stageBox.w) return null;
    const pxPerUnit = base * view.scale;
    const pxPerCm = pxPerUnit * 2.5;
    const stepsCm = [1, 2, 5, 10, 20];
    const stepsIn = [1, 2, 5, 10];
    const steps = unit === "in" ? stepsIn : stepsCm;
    const raw = 120 / pxPerCm;
    let value = steps[0] as number;
    for (const s of steps) if (s <= raw) value = s;
    const px = Math.round(value * (unit === "in" ? pxPerCm / 2.54 : pxPerCm));
    return { value, px, text: unit === "in" ? `${value} in` : `${value} cm` };
  }, [base, view.scale, stageBox.w, unit]);

  const frameObj = selZone ?? selLm;

  /** The picked parts that carry lights: the ones that can be aligned and spread. */
  const pickedZones = useMemo(
    () =>
      selection
        .filter((s) => s.kind === "zone")
        .map((s) => ({ index: s.index, zone: zones[s.index] }))
        .filter((p): p is { index: number; zone: PlacedZone } => !!p.zone),
    [selection, zones],
  );

  /** Lines the picked parts up, or spreads them out, as one change. */
  const arrange = (how: "left" | "hcenter" | "right" | "top" | "vcenter" | "bottom" | "spreadH" | "spreadV") => {
    if (pickedZones.length < 2) return;
    const boxes = pickedZones.map((p) => ({ index: p.index, zone: p.zone }));
    const minX = Math.min(...boxes.map((b) => b.zone.x));
    const maxX = Math.max(...boxes.map((b) => b.zone.x + b.zone.width));
    const minY = Math.min(...boxes.map((b) => b.zone.y));
    const maxY = Math.max(...boxes.map((b) => b.zone.y + b.zone.height));
    const patches = boxes.map((b) => {
      const z = b.zone;
      let x = z.x;
      let y = z.y;
      if (how === "left") x = minX;
      else if (how === "right") x = maxX - z.width;
      else if (how === "hcenter") x = (minX + maxX - z.width) / 2;
      else if (how === "top") y = minY;
      else if (how === "bottom") y = maxY - z.height;
      else if (how === "vcenter") y = (minY + maxY - z.height) / 2;
      else if (how === "spreadH") {
        const sorted = [...boxes].sort((p, q) => p.zone.x - q.zone.x);
        const first = sorted[0];
        const last = sorted[sorted.length - 1];
        if (!first || !last) return { index: b.index, patch: {} as Partial<PlacedZone> };
        const span = last.zone.x - (first.zone.x + first.zone.width);
        const used = sorted.reduce((sum, s) => sum + s.zone.width, 0);
        const gap = (span - used) / Math.max(1, sorted.length - 1);
        let at = first.zone.x + first.zone.width;
        for (const s of sorted) {
          if (s.index === b.index) {
            x = at;
            break;
          }
          at += s.zone.width + gap;
        }
      } else if (how === "spreadV") {
        const sorted = [...boxes].sort((p, q) => p.zone.y - q.zone.y);
        const first = sorted[0];
        const last = sorted[sorted.length - 1];
        if (!first || !last) return { index: b.index, patch: {} as Partial<PlacedZone> };
        const span = last.zone.y - (first.zone.y + first.zone.height);
        const used = sorted.reduce((sum, s) => sum + s.zone.height, 0);
        const gap = (span - used) / Math.max(1, sorted.length - 1);
        let at = first.zone.y + first.zone.height;
        for (const s of sorted) {
          if (s.index === b.index) {
            y = at;
            break;
          }
          at += s.zone.height + gap;
        }
      }
      return { index: b.index, patch: { x, y } as Partial<PlacedZone> };
    });
    onChangeZones(patches.filter((p) => Object.keys(p.patch).length > 0));
  };

  return (
    <>
      <div
        ref={stageRef}
        className={`stage ${desk.desk} ${desk.canvas}`}
        data-editor-draw={!daemonLive && !!effect && !effectNeedsInput ? "true" : "false"}
        data-mode={mode}
        tabIndex={-1}
        onPointerMove={onStageMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onLostPointerCapture={endDrag}
        onPointerLeave={() => {
          // Off the desk: nothing is hovered. (A held press keeps the pointer by capture.)
          if (dragRef.current || pendingRef.current) return;
          if (stageRef.current) stageRef.current.style.cursor = "";
          setHandleHover(null);
          hoverRef.current = null;
          setHover(null);
        }}
        onDragOver={onDragOver}
        onDrop={onDropHere}
        // One handler for the whole desk: what a press lands on is decided from the parts'
        // geometry (lib/hitTest.ts), not from which element the browser says is on top.
        onPointerDown={onStagePointerDown}
      >
        {fading && !editorGone && (
          <div
            className={desk.editorPreview}
            data-editor-fading="true"
            aria-hidden="true"
            style={{
              transform: `translate(${view.panX * base * view.scale}px, ${view.panY * base * view.scale}px) scale(${base * view.scale})`,
            }}
          >
            <EffectPreview id={fading.id} className={desk.editorCanvas} />
          </div>
        )}
        {drawnNow && !editorGone && (
          // The editor's own draw of the running effect. It is drawn in the canvas's own
          // 320 x 200 units, so it lines up with the zones, and the daemon's frames fade in over
          // it when they arrive. In the lights mode it is hidden, but it is still there: the
          // editor reads its pixels to know what colour each LED is.
          <div
            ref={editorLayerRef}
            className={`${desk.editorPreview} ${fading ? desk.editorFadeIn : ""}`}
            data-editor-preview="true"
            data-mode={mode}
            aria-hidden="true"
            style={{
              opacity: !daemonLive && !!effect && !effectNeedsInput ? (mode === "effect" ? 1 : 0) : 0,
              transform: `translate(${view.panX * base * view.scale}px, ${view.panY * base * view.scale}px) scale(${base * view.scale})`,
            }}
          >
            <EffectPreview id={drawnNow.id} className={desk.editorCanvas} />
          </div>
        )}
        <canvas
          ref={previewRef}
          className="stage-canvas"
          width={PREVIEW_W}
          height={PREVIEW_H}
          aria-hidden="true"
          style={{
            transformOrigin: "0 0",
            transform: `translate(${view.panX * base * view.scale}px, ${view.panY * base * view.scale}px) scale(${view.scale})`,
            opacity: daemonLive && mode === "effect" ? 1 : 0,
          }}
        />
        <CanvasFurniture svgRef={svgRef} gRef={gRef} unit={unit} k={k} />
        <canvas ref={overlayRef} className="stage-leds" aria-hidden="true" />
        {/* The mark on the LED the setup's walker has lit. */}
        <span ref={litMarkerRef} className={desk.litMarker} aria-hidden="true" hidden />

        {guides.length > 0 && (
          // Faint outlines of a typical small, mid and full tower behind the case: guides to
          // compare against, not a picker. Their names are buttons that snap the case.
          <div className={desk.guides} data-guides="true">
            {guides.map((g) => (
              <div
                key={g.id}
                className={desk.guide}
                data-guide-id={g.id}
                style={{
                  left: `${(g.x + view.panX) * k}px`,
                  top: `${(g.y + view.panY) * k}px`,
                  width: `${g.width * k}px`,
                  height: `${g.height * k}px`,
                }}
              >
                <button
                  type="button"
                  className={desk.guideTag}
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={g.onPick}
                  aria-label={`Use a ${g.name.toLowerCase()}, ${g.size}`}
                  style={{ "--lead": `${(g.x - g.labelX) * k + 6}px` } as CSSProperties}
                >
                  <span>{g.name}</span>
                  <span className={desk.guideSize}>{g.size}</span>
                </button>
              </div>
            ))}
          </div>
        )}

        {landmarks.map((l, i) => {
          if (l.visible === false) return null;
          const isSel = selection.some((s) => s.kind === "landmark" && s.index === i);
          const isHover = hovered?.kind === "landmark" && hovered.index === i;
          const style = {
            left: `${(l.x + view.panX) * k}px`,
            top: `${(l.y + view.panY) * k}px`,
            width: `${l.width * k}px`,
            height: `${l.height * k}px`,
            transform: `rotate(${l.rotation_deg}deg)`,
          } as CSSProperties;
          return (
            <div
              key={`lm-${i}`}
              ref={(el) => {
                lmEls.current[i] = el;
              }}
              className={`${desk.zone} ${desk.landmark}`}
              role="button"
              tabIndex={0}
              aria-pressed={isSel}
              aria-label={`${l.label?.trim() || PART_LABEL[l.kind]}, a landmark without lights`}
              data-landmark="true"
              data-landmark-index={i}
              data-pulse={pulse.some((s) => s.kind === "landmark" && s.index === i)}
              data-kind={l.kind}
              data-selected={isSel}
              data-hover={isHover}
              data-dim={isDim({ kind: "landmark", index: i })}
              data-locked={l.locked === true}
              data-outside={outsideOf(l.x, l.y, l.width, l.height)}
              style={style}
              onFocus={(e) => {
                // The keyboard picks a part by focusing it; a press is the stage's to decide.
                if (e.currentTarget.matches(":focus-visible")) onSelect([{ kind: "landmark", index: i }]);
              }}
            >
              <PartArt kind={l.kind} facing={l.facing} />
              {unlockBadge && l.locked === true && (
                <button
                  type="button"
                  className={desk.lockBadge}
                  aria-label={`Unlock the ${(l.label?.trim() || PART_LABEL[l.kind]).toLowerCase()}`}
                  title="Locked. Click to unlock"
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    onChangeLandmark(i, { locked: false });
                  }}
                >
                  <LockSimple size={12} weight="fill" aria-hidden="true" />
                </button>
              )}
            </div>
          );
        })}

        {derivedCase && (
          // The case of an old layout, drawn from its form until the daemon migrates it. It is
          // a picture, not a part: it cannot be picked.
          <div
            className={`${desk.zone} ${desk.landmark} ${desk.derived}`}
            aria-hidden="true"
            data-derived="true"
            data-kind="case"
            style={{
              left: `${(derivedCase.x + view.panX) * k}px`,
              top: `${(derivedCase.y + view.panY) * k}px`,
              width: `${derivedCase.width * k}px`,
              height: `${derivedCase.height * k}px`,
            }}
          >
            <PartArt kind="case" facing={derivedCase.facing} />
          </div>
        )}

        {zones.map((z, i) => {
          const found = findZoneInfo(devices, z.device_id, z.zone_index);
          const accent = accentOf(devices, z.device_id);
          const isSel = selection.some((s) => s.kind === "zone" && s.index === i);
          const isMissing = missing.has(i);
          const kind = partOf(z, devices, found?.zone ?? null);
          const facing = z.part?.facing ?? "viewer";
          const key = `zone:${zoneKey(z)}`;
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
              key={key}
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
              data-kind={kind}
              data-locked={z.locked === true}
              data-outside={outsideOf(z.x, z.y, z.width, z.height)}
              data-hover={hovered?.kind === "zone" && hovered.index === i}
              data-dim={isDim({ kind: "zone", index: i })}
              data-identify={identifying === i}
              data-pulse={pulse.some((s) => s.kind === "zone" && s.index === i)}
              data-shape={found ? resolveShape(z.shape, found.zone.kind) : "none"}
              style={style}
              onFocus={(e) => {
                // The keyboard picks a part by focusing it; a press is the stage's to decide.
                if (e.currentTarget.matches(":focus-visible")) onSelect([{ kind: "zone", index: i }]);
              }}
            >
              <PartArt kind={kind} facing={facing} />
              {isMissing && <span className={desk.offline}>offline</span>}
            </div>
          );
        })}

        {frameObj && selection.length === 1 && (
          // The selection: eight resize handles and a rotation handle, in the part's own turned
          // box. They are drawn here and found by the stage's hit-test; none takes a click itself.
          <div
            ref={frameEl}
            className={desk.frame}
            data-frame="true"
            data-locked={frameObj.locked === true}
            data-narrow-x={frameObj.width * k < EDGE_HANDLE_MIN_PX}
            data-narrow-y={frameObj.height * k < EDGE_HANDLE_MIN_PX}
            style={{
              left: `${(frameObj.x + view.panX) * k}px`,
              top: `${(frameObj.y + view.panY) * k}px`,
              width: `${frameObj.width * k}px`,
              height: `${frameObj.height * k}px`,
              transform: `rotate(${frameObj.rotation_deg}deg)`,
            }}
          >
            <span className={desk.stem} aria-hidden="true" />
            <span
              className={desk.rotateHandle}
              data-handle="rotate"
              role="button"
              tabIndex={-1}
              aria-label="Rotate this part"
            />
            {HANDLES.map((h) => (
              <span
                key={h.id}
                className={desk.handle}
                data-handle={h.id}
                role="button"
                tabIndex={-1}
                aria-label={`Resize this part from the ${h.id} handle`}
                style={{
                  left: `${(h.sx + 1) * 50}%`,
                  top: `${(h.sy + 1) * 50}%`,
                }}
              />
            ))}
          </div>
        )}

        {selection.length > 1 && (
          // More than one part is picked: each gets a quiet outline, no handles.
          <div className={desk.multi} aria-hidden="true">
            {selection.map((s) => {
              if (s.kind === "zone") {
                const o = zones[s.index];
                if (!o) return null;
                const b = boundsOf(o);
                return (
                  <span
                    key={`${s.kind}-${s.index}`}
                    className={desk.multiBox}
                    style={{
                      left: `${(b.x + view.panX) * k}px`,
                      top: `${(b.y + view.panY) * k}px`,
                      width: `${b.width * k}px`,
                      height: `${b.height * k}px`,
                      transform: `rotate(${o.rotation_deg}deg)`,
                    }}
                  />
                );
              }
              const o = landmarks[s.index];
              if (!o) return null;
              const b = landmarkBounds(o);
              return (
                <span
                  key={`${s.kind}-${s.index}`}
                  className={desk.multiBox}
                  style={{
                    left: `${(b.x + view.panX) * k}px`,
                    top: `${(b.y + view.panY) * k}px`,
                    width: `${b.width * k}px`,
                    height: `${b.height * k}px`,
                    transform: `rotate(${o.rotation_deg}deg)`,
                  }}
                />
              );
            })}
          </div>
        )}

        {marquee && (
          <div
            className={desk.marquee}
            aria-hidden="true"
            style={{ left: marquee.x, top: marquee.y, width: marquee.w, height: marquee.h }}
          />
        )}

        {hint && (
          <div className={desk.hint} data-hint="true">
            <b>L</b> lights · <b>E</b> effect
          </div>
        )}

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
          data-guide="v"
          aria-hidden="true"
        />
        <span
          ref={(el) => {
            guideEls.current.v2 = el;
          }}
          className={desk.guideV2}
          data-guide="v2"
          aria-hidden="true"
        />
        <span
          ref={(el) => {
            guideEls.current.h = el;
          }}
          className={desk.guideH}
          data-guide="h"
          aria-hidden="true"
        />

        {tags.map((t) => (
          <div
            key={`${t.sel.kind}-${t.sel.index}`}
            className={desk.tag}
            data-tag="true"
            data-selected={t.selected}
            style={
              {
                left: `${t.x}px`,
                top: `${t.y}px`,
                "--zc": t.accent < 0 ? "var(--line-2)" : `var(${ACCENTS[t.accent] ?? "--a1"})`,
              } as CSSProperties
            }
          >
            <span className={t.selected ? desk.tagFig : undefined}>{t.text}</span>
          </div>
        ))}

        {sizing && (
          <SizeEditor
            unit={unit}
            zone={sizing.kind === "zone" ? zones[sizing.index] : undefined}
            landmark={sizing.kind === "landmark" ? landmarks[sizing.index] : undefined}
            onChange={(patch) => {
              if (sizing.kind === "zone") onChangeZones([{ index: sizing.index, patch }]);
              else onChangeLandmark(sizing.index, patch);
            }}
            onClose={() => setSizing(null)}
          />
        )}
      </div>
      <div className={`${desk.toolbar} stage-toolbar`}>
        {scaleBar ? (
          <div className={desk.scaleBar} role="img" aria-label={`Scale: ${scaleBar.text}`}>
            <span className={desk.scaleLine} style={{ width: `${scaleBar.px}px` }} />
            <span className={desk.scaleLabel}>
              <span className="fig">{scaleBar.text}</span>
            </span>
          </div>
        ) : (
          <span />
        )}
        <div className={`${desk.segs} stage-segs`}>
          {/* What the canvas shows, and what the cells mean: the note is the segment's own
              tooltip, not a line under the desk (R8). */}
          <Tooltip label={modeNote} className={`${desk.modeTip} stage-mode-tip`}>
            <Segmented<StageMode>
              label="What the canvas shows"
              value={mode}
              options={[
                { value: "effect", label: "Effect" },
                { value: "lights", label: "Lights" },
              ]}
              onChange={onMode}
            />
          </Tooltip>
          {toolbarExtra}
        </div>
        <div className={`${desk.controls} stage-zoom`}>
          <IconButton label="Zoom out" icon={<Minus size={13} />} size="sm" onClick={() => zoomBy(1 / 1.3)} />
          <span className={desk.zoomValue}>{Math.round(view.scale * 100)}%</span>
          <IconButton label="Zoom in" icon={<Plus size={13} />} size="sm" onClick={() => zoomBy(1.3)} />
          <IconButton label="Fit the canvas" icon={<ArrowsOut size={13} />} size="sm" onClick={fit} />
        </div>
      </div>
      {pickedZones.length > 1 && (
        <div className={desk.toolbarRow}>
          <span className={desk.toolbarLabel}>Line up</span>
          <IconButton label="Line the parts up on the left" icon={<AlignLeft size={13} />} size="sm" onClick={() => arrange("left")} />
          <IconButton label="Line the parts up in the middle" icon={<AlignCenterHorizontal size={13} />} size="sm" onClick={() => arrange("hcenter")} />
          <IconButton label="Line the parts up on the right" icon={<AlignRight size={13} />} size="sm" onClick={() => arrange("right")} />
          <IconButton label="Line the parts up at the top" icon={<AlignTop size={13} />} size="sm" onClick={() => arrange("top")} />
          <IconButton label="Line the parts up in the middle" icon={<AlignCenterVertical size={13} />} size="sm" onClick={() => arrange("vcenter")} />
          <IconButton label="Line the parts up at the bottom" icon={<AlignBottom size={13} />} size="sm" onClick={() => arrange("bottom")} />
          <IconButton label="Spread the parts out across" icon={<AlignCenterHorizontal size={13} />} size="sm" onClick={() => arrange("spreadH")} />
          <IconButton label="Spread the parts out down" icon={<AlignCenterVertical size={13} />} size="sm" onClick={() => arrange("spreadV")} />
        </div>
      )}
      {showLegend ? (
        <p className={desk.legend}>
          {legend}
        </p>
      ) : null}
    </>
  );
}

/** True when a part sits outside the frame of the effect area. */
function outsideOf(x: number, y: number, w: number, h: number): boolean {
  return x < -0.5 || y < -0.5 || x + w > CANVAS_W + 0.5 || y + h > CANVAS_H + 0.5;
}

/**
 * The distance between two neighbouring LEDs of a part, in canvas units: the pitch of the
 * light. The halo is a little over one pitch, so the light of a row of LEDs runs together
 * without becoming one blob.
 */
function firstPitch(local: LocalLeds, horizontal: boolean): number {
  const led = local.leds.find((l) => !!l);
  if (!led) return 4;
  if (local.shape === "strip") return Math.max(0.5, horizontal ? led.w : led.h);
  return Math.max(0.5, Math.min(led.w, led.h));
}

/** The box (canvas units) that holds the lit cells so far. */
function growRun(
  run: [number, number, number, number] | null,
  cell: CanvasCell,
): [number, number, number, number] {
  const [x0, y0, x1, y1] = cell.bounds;
  if (!run) return [x0, y0, x1, y1];
  return [Math.min(run[0], x0), Math.min(run[1], y0), Math.max(run[2], x1), Math.max(run[3], y1)];
}

/** Puts the mark round a whole lit run (a fan): the same ring, as wide as the run. */
function markRun(
  marker: HTMLElement | null,
  run: [number, number, number, number],
  v: View,
  kCss: number,
): void {
  if (!marker) return;
  const pad = 4;
  marker.style.left = `${(run[0] + v.panX) * kCss - pad}px`;
  marker.style.top = `${(run[1] + v.panY) * kCss - pad}px`;
  marker.style.width = `${Math.max(20, (run[2] - run[0]) * kCss) + pad * 2}px`;
  marker.style.height = `${Math.max(20, (run[3] - run[1]) * kCss) + pad * 2}px`;
  marker.dataset.run = "true";
  marker.hidden = false;
}

/**
 * Puts the mark on the LED the setup's walker has lit: a bright ring in the accent colour, with
 * a slow pulse, so the reader can match the light in the room with the cell on the drawing at
 * any zoom.
 */
function markLit(
  marker: HTMLElement | null,
  cell: CanvasCell,
  led: { w: number; h: number },
  v: View,
  kCss: number,
): void {
  if (!marker) return;
  delete marker.dataset.run;
  // A ring round the cell, big enough to be found at a glance, and never smaller than the cell.
  const d = Math.max(20, Math.min(led.w, led.h) * kCss);
  marker.style.left = `${(cell.cx + v.panX) * kCss - d / 2}px`;
  marker.style.top = `${(cell.cy + v.panY) * kCss - d / 2}px`;
  marker.style.width = `${d}px`;
  marker.style.height = `${d}px`;
  marker.hidden = false;
}

/**
 * The outline of a part on the overlay canvas: its own box, turned with the part, and a margin
 * round it. A part's halo is clipped to this, so no light ever leaves the part it belongs to.
 */
function partPath(
  ctx: CanvasRenderingContext2D,
  z: PlacedZone,
  k: number,
  ox: number,
  oy: number,
  margin: number,
): void {
  const t = zoneTransform(z);
  const hw = (z.width / 2) * k + margin;
  const hh = (z.height / 2) * k + margin;
  const cx = t.cx * k + ox;
  const cy = t.cy * k + oy;
  ctx.beginPath();
  for (const [lx, ly] of [
    [-hw, -hh],
    [hw, -hh],
    [hw, hh],
    [-hw, hh],
  ] as const) {
    const x = cx + lx * t.cos - ly * t.sin;
    const y = cy + lx * t.sin + ly * t.cos;
    if (lx === -hw && ly === -hh) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.closePath();
}

/**
 * The typed size of a part: width and height, in the reader's unit, with Tab moving between
 * them and Enter letting go. It is opened with Enter on a picked part.
 */
function SizeEditor({
  unit,
  zone,
  landmark,
  onChange,
  onClose,
}: {
  unit: LengthUnit;
  zone?: PlacedZone;
  landmark?: Landmark;
  onChange: (patch: Partial<PlacedZone> | Partial<Landmark>) => void;
  onClose: () => void;
}) {
  const o = zone ?? landmark;
  if (!o) return null;
  const wMm = unitsToMm(o.width);
  const hMm = unitsToMm(o.height);
  return (
    <div className={desk.sizeEditor} data-size-editor="true" onKeyDown={(e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    }}>
      <label className={desk.sizeField}>
        <span>Width</span>
        <NumberField
          label="Width"
          value={lengthNumber(wMm, unit)}
          min={0}
          max={lengthNumber(CANVAS_W * MM_PER_UNIT, unit)}
          decimals={unit === "in" ? 2 : 1}
          unit={unitWord(unit)}
          onCommit={(v) => onChange({ width: mmFromLength(v, unit) / MM_PER_UNIT })}
        />
      </label>
      <label className={desk.sizeField}>
        <span>Height</span>
        <NumberField
          label="Height"
          value={lengthNumber(hMm, unit)}
          min={0}
          max={lengthNumber(CANVAS_H * MM_PER_UNIT, unit)}
          decimals={unit === "in" ? 2 : 1}
          unit={unitWord(unit)}
          onCommit={(v) => onChange({ height: mmFromLength(v, unit) / MM_PER_UNIT })}
        />
      </label>
    </div>
  );
}

/** A zone's LED layout, kept until the zone's own shape changes. */
interface LedLayout {
  sig: string;
  local: LocalLeds;
}

/**
 * Keeps the canvas in the stage: at or above 1 the canvas covers the stage, so it can be
 * panned but never dragged away; below 1 it is centred.
 */
function clampView(v: View): View {
  const scale = clamp(v.scale, MIN_SCALE, MAX_SCALE);
  const pan = scale >= 1 ? clamp(v.panX, CANVAS_W * (1 / scale - 1), 0) : (CANVAS_W * (1 / scale - 1)) / 2;
  const panY = scale >= 1 ? clamp(v.panY, CANVAS_H * (1 / scale - 1), 0) : (CANVAS_H * (1 / scale - 1)) / 2;
  return { scale, panX: pan, panY };
}
