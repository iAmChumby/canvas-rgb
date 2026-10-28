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
} from "react";
import { ArrowsOut, Minus, Plus, X } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { EffectPreview, type PreviewValues } from "@/components/EffectPreview";
import { useMediaQuery } from "@/components/MediaQuery";
import { onFrame, PREVIEW_H, PREVIEW_W, usePreviewStatus, type Frame } from "@/lib/preview";
import {
  canvasCell,
  findZoneInfo,
  resolveShape,
  zoneCorners,
  zoneLocalLeds,
  zoneTransform,
  type CanvasCell,
  type LocalLeds,
  type Point,
} from "@/lib/zones";
import { clamp } from "@/lib/format";
import { usePrefersReducedMotion } from "@/lib/motion";
import { FIT_VIEW, forgetView, readView, writeView } from "@/lib/viewPrefs";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { PlacedZone } from "@/proto/PlacedZone";
import type { SamplingMode } from "@/proto/SamplingMode";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import desk from "./desk.module.css";

const CANVAS_W = 320;
const CANVAS_H = 200;
const ACCENTS = ["--a1", "--a2", "--a3", "--a4", "--a5"] as const;

/** The grid a zone snaps to, in canvas units. */
const SNAP_GRID = 5;
/** How close, in screen pixels, a zone has to be to another zone's line to snap to it. */
const SNAP_PIXELS = 8;
/** The smallest a zone can be, in canvas units. */
const MIN_SIZE = 2;
/**
 * A zone's edge handle is hidden when the edge is shorter than this, in css pixels: on a tiny
 * zone the eight handles would sit on top of each other, so only the corners stay.
 */
const EDGE_HANDLE_MIN_PX = 44;
/** How far the rotation handle stands off the zone, in css pixels. */
const ROTATE_STANDOFF = 24;

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
/** The smallest and the biggest a dot is drawn, in css pixels across the radius. */
const DOT_MIN_PX = 1.2;
const DOT_MAX_PX = 9;

/**
 * The eight resize handles, as Figma lays them out: four corners and four edges. `sx` and `sy`
 * say which way the handle pulls the box (-1 the left or top edge, 1 the right or bottom edge, 0
 * the axis it leaves alone).
 */
type HandleId = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
interface HandleDef {
  id: HandleId;
  sx: -1 | 0 | 1;
  sy: -1 | 0 | 1;
}
const HANDLES: HandleDef[] = [
  { id: "nw", sx: -1, sy: -1 },
  { id: "n", sx: 0, sy: -1 },
  { id: "ne", sx: 1, sy: -1 },
  { id: "e", sx: 1, sy: 0 },
  { id: "se", sx: 1, sy: 1 },
  { id: "s", sx: 0, sy: 1 },
  { id: "sw", sx: -1, sy: 1 },
  { id: "w", sx: -1, sy: 0 },
];

/** The resize cursor for a handle on a zone turned by `rotation` degrees. */
function handleCursor(h: HandleDef, rotation: number): string {
  const angle = (Math.atan2(h.sy, h.sx) * 180) / Math.PI + rotation;
  const slot = ((Math.round(angle / 45) % 4) + 4) % 4;
  return ["ew-resize", "nwse-resize", "ns-resize", "nesw-resize"][slot] ?? "nwse-resize";
}

type DragKind = "move" | "resize" | "rotate";
interface Drag {
  kind: DragKind;
  index: number;
  start: Point;
  orig: PlacedZone;
  handle: HandleDef | null;
}

/** The view: how the 320 x 200 canvas is drawn into the stage. It never touches the layout. */
interface View {
  scale: number;
  panX: number;
  panY: number;
}

/** The lines a zone offers to the guides: its two edges and its middle. */
function zoneLines(z: { x: number; y: number; width: number; height: number }, axis: "x" | "y"): number[] {
  const a = axis === "x" ? z.x : z.y;
  const b = axis === "x" ? z.width : z.height;
  return [a, a + b / 2, a + b];
}

/** The desk's own edges and middle: a zone lines up with them as it does with another zone. */
const CANVAS_BOX = { x: 0, y: 0, width: CANVAS_W, height: CANVAS_H };

const gridOf = (v: number) => Math.round(v / SNAP_GRID) * SNAP_GRID;

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
 * Where a moving zone lands: on the grid, or on another zone's edge or middle when it is close
 * enough. The alignment wins over the grid, and the lines it snapped to are the guides. Figma
 * snaps the same way: to the pixel grid and to the edges and centres of nearby layers
 * (https://help.figma.com/hc/en-us/articles/360041064174-Snap-to-objects-and-guides).
 */
function snapMove(
  moving: { x: number; y: number; width: number; height: number },
  others: { x: number; y: number; width: number; height: number }[],
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
  return {
    x: bestX ? moving.x + bestX.delta : gridOf(moving.x),
    y: bestY ? moving.y + bestY.delta : gridOf(moving.y),
    guideX: bestX?.line ?? null,
    guideY: bestY?.line ?? null,
    gapX: bestX ? Math.abs(bestX.delta) : null,
    gapY: bestY ? Math.abs(bestY.delta) : null,
  };
}

/**
 * Resizes a zone by one of its eight handles, in the zone's own turned frame, as Figma does:
 *
 *   - the handle opposite the one dragged stays where it is;
 *   - Shift keeps the box's proportions (corners scale both sides, an edge scales the other side
 *     about the box's middle line);
 *   - Alt resizes from the centre.
 *
 * `p` is the pointer on the canvas, already snapped by the caller.
 */
function resizeBy(o: PlacedZone, handle: HandleDef, p: Point, shift: boolean, alt: boolean): PlacedZone {
  const rad = (o.rotation_deg * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const cx0 = o.x + o.width / 2;
  const cy0 = o.y + o.height / 2;
  // The pointer in the zone's own frame: origin at its centre, axes along its sides.
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
  return { ...o, x: cx - w / 2, y: cy - h / 2, width: w, height: h };
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
  /**
   * Takes a zone off the layout. The label of a stale zone offers this once the zone is picked:
   * a ghost cannot be clicked on the canvas, so the label is the way to it.
   */
  onRemoveZone?: (index: number) => void;
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
}

function zoneKey(z: PlacedZone): string {
  return `${z.device_id}#${z.zone_index}`;
}

/** The device accent of a zone, or -1 when the device is not in the list any more. */
function accentOf(devices: DeviceInfo[] | null, deviceId: string): number {
  const i = devices?.findIndex((d) => d.id === deviceId) ?? -1;
  return i < 0 ? -1 : i % ACCENTS.length;
}

/** The axis-aligned box of a zone's corners, in canvas units. */
function boundsOf(z: PlacedZone): { x: number; y: number; w: number; h: number } {
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
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
}

/** The luminance of a colour, 0 to 255: whether a dot is bright or dark against its ring. */
function luminance(c: RGB): number {
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
}

/** A zone's LED layout, kept until the zone's own shape changes. */
interface LedLayout {
  sig: string;
  local: LocalLeds;
}

/**
 * The live preview with every placed zone drawn over it. Zones move, resize and rotate by
 * pointer (or by arrow keys, with Shift for ten units).
 *
 * A drag never re-renders React: the pointer move writes the zone's geometry straight onto the
 * element and repaints the LED overlay, and the geometry is committed to the layout once, when
 * the pointer goes up. The canvas can be zoomed and panned; that is a view transform only and
 * never reaches the saved layout.
 *
 * Each zone is drawn as a thin outline of its shape with its LEDs as small dots at the places
 * the daemon samples. A selected zone gets Figma's frame: eight resize handles and a rotation
 * handle (https://help.figma.com/hc/en-us/articles/360039956914).
 */
export function PreviewStage({
  zones,
  devices,
  selected,
  onSelect,
  onChange,
  onDrop,
  onRemoveZone,
  onDraggingChange,
  onLiveChange,
  layoutId,
  dropPayloadType,
  effect = null,
  effectNeedsInput = false,
  sampling = "nearest",
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
  const frameEl = useRef<HTMLDivElement | null>(null);
  const labelEl = useRef<HTMLDivElement | null>(null);
  const guideEls = useRef<{ v: HTMLSpanElement | null; h: HTMLSpanElement | null }>({ v: null, h: null });
  const readoutRef = useRef<HTMLDivElement | null>(null);
  const editorLayerRef = useRef<HTMLDivElement | null>(null);
  const editorPicRef = useRef<Picture | null>(null);
  const daemonLiveRef = useRef(false);
  const samplingRef = useRef<SamplingMode>("nearest");
  const missingRef = useRef<Set<number>>(new Set());
  const selectedRef = useRef<number | null>(selected);
  selectedRef.current = selected;
  const zonesRef = useRef(zones);
  zonesRef.current = zones;
  const paintQueued = useRef(false);
  const liveRaf = useRef<number | null>(null);
  const zoomRaf = useRef<number | null>(null);
  const viewCommit = useRef<ReturnType<typeof setTimeout> | null>(null);
  const layoutCache = useRef<Map<string, LedLayout>>(new Map());
  const ledOff = useRef<{ key: string; color: string }>({ key: "", color: "rgba(255,255,255,0.3)" });
  const [hover, setHover] = useState<number | null>(null);
  /** The zone a finger or a pointer is on: its label shows while it is held. */
  const [pressed, setPressed] = useState<number | null>(null);
  const [view, setView] = useState<View>({ scale: 1, panX: 0, panY: 0 });
  const viewRef = useRef<View>(view);
  const [viewFor, setViewFor] = useState<string | null>(null);
  const status = usePreviewStatus();
  const reduced = usePrefersReducedMotion();
  /**
   * A phone's canvas is 358 px wide and carries a dozen zones: a label on each is a wall of text
   * over the lights. On a phone only the picked zone is named, and the zone under a finger
   * names itself while it is held.
   */
  const phone = useMediaQuery("(max-width: 720px)");
  /** The zone whose name shows: the one being dragged, then the one under the pointer, then the picked one. */
  const labelIndex = phone ? (pressed ?? selected) : (pressed ?? hover ?? selected);
  const labelIndexRef = useRef<number | null>(labelIndex);
  labelIndexRef.current = labelIndex;
  /**
   * The daemon's frames are the honest picture of the effect. When they are not coming, the
   * editor draws the running effect itself, behind the zones, and says so.
   */
  const daemonLive = status.state === "live";
  daemonLiveRef.current = daemonLive;
  samplingRef.current = sampling;
  const editorDraws = !daemonLive && !!effect && !effectNeedsInput;
  /**
   * The picture on the canvas, and the one it is fading out of. When the running effect
   * changes, the new picture fades in over the old one instead of snapping.
   */
  const [drawn, setDrawn] = useState<{ id: string; props: PreviewValues } | null>(null);
  const [fading, setFading] = useState<{ id: string; props: PreviewValues } | null>(null);
  const effectId = effect?.id ?? null;
  const [lastEffectId, setLastEffectId] = useState<string | null>(null);
  if (effectId !== lastEffectId) {
    // The running effect changed: the picture that was showing fades out under the new one.
    setLastEffectId(effectId);
    setFading(drawn);
    setDrawn(effect ? { id: effect.id, props: effect.props } : null);
  }
  /**
   * The effect's values can change while it runs - the Layout page's "Now playing" panel edits
   * them - and the editor's own draw follows them, so the canvas shows the change at once.
   */
  const drawnNow = useMemo(() => {
    if (!drawn) return null;
    if (effect && effect.id === drawn.id) return { id: drawn.id, props: effect.props };
    return drawn;
  }, [drawn, effect]);
  useEffect(() => {
    // The picture that is fading out is taken down once the fade is over.
    if (!fading) return;
    const t = setTimeout(() => setFading(null), 320);
    return () => clearTimeout(t);
  }, [fading]);
  /**
   * The editor's draw is taken down once the daemon's frames have taken over: it would
   * otherwise keep repainting a canvas nobody can see.
   */
  const [editorGone, setEditorGone] = useState(false);
  useEffect(() => {
    // The editor's draw comes back at once when the daemon's frames stop, and is taken down a
    // moment after they take over.
    const t = setTimeout(() => setEditorGone(daemonLive), daemonLive ? 400 : 0);
    return () => clearTimeout(t);
  }, [daemonLive]);

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

  // The device's description of every zone, found once per layout change.
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
  missingRef.current = missing;

  /**
   * A stale zone cannot be picked on the canvas: it is a ghost over another zone and it never
   * takes a click. Its label is the way to it, and the label carries the way to take it off the
   * layout once the zone is picked.
   */
  const removable = useCallback(
    (index: number) => selected === index && missing.has(index) && !!onRemoveZone,
    [selected, missing, onRemoveZone],
  );

  /** Where a zone's LEDs sit, rebuilt only when its size, shape or order changes. */
  const layoutOf = useCallback((z: PlacedZone, info: ZoneInfo): LocalLeds => {
    const sig = `${z.width.toFixed(3)}|${z.height.toFixed(3)}|${JSON.stringify(z.shape)}|${z.reversed}|${info.led_count}|${info.kind.type}`;
    const key = zoneKey(z);
    const hit = layoutCache.current.get(key);
    if (hit && hit.sig === sig) return hit.local;
    const local = zoneLocalLeds(z, info);
    layoutCache.current.set(key, { sig, local });
    return local;
  }, []);

  /** Places the zone's name beside it: below a picked zone (the rotation handle is above), above a hovered one. */
  const placeLabel = useCallback((index: number | null, v: View) => {
    const el = labelEl.current;
    const stage = stageRef.current;
    if (!el || !stage || index === null) return;
    const z = liveRef.current[index];
    if (!z) return;
    const k = (stage.clientWidth / CANVAS_W) * v.scale;
    const b = boundsOf(z);
    const box = { x: (b.x + v.panX) * k, y: (b.y + v.panY) * k, w: b.w * k, h: b.h * k };
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const above = box.y - h - 6;
    const below = box.y + box.h + 8;
    const picked = selectedRef.current === index;
    let y = picked ? below : above;
    if (picked && below + h > stage.clientHeight - 2) y = above;
    else if (!picked && above < 2) y = below;
    el.style.left = `${clamp(box.x, 2, Math.max(2, stage.clientWidth - w - 2))}px`;
    el.style.top = `${clamp(y, 2, Math.max(2, stage.clientHeight - h - 2))}px`;
  }, []);

  /** Writes the selection frame onto its element: the zone's box, its handles' cursors, which handles fit. */
  const applyFrame = useCallback((z: PlacedZone, v: View) => {
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
    // The rotation handle stands above the zone; when that is off the canvas it stands below.
    const t = zoneTransform(z);
    const px = ROTATE_STANDOFF / k;
    const topOut = {
      x: t.cx + 0 * t.cos - (-z.height / 2 - px) * t.sin,
      y: t.cy + 0 * t.sin + (-z.height / 2 - px) * t.cos,
    };
    const fits = topOut.x > 2 && topOut.x < CANVAS_W - 2 && topOut.y > 2 && topOut.y < CANVAS_H - 2;
    el.dataset.flip = fits ? "false" : "true";
    for (const handle of HANDLES) {
      const h = el.querySelector<HTMLElement>(`[data-handle="${handle.id}"]`);
      if (h) h.style.cursor = handleCursor(handle, z.rotation_deg);
    }
  }, []);

  /** Writes one zone's geometry onto its element. No React involved. */
  const applyGeom = useCallback(
    (index: number, z: PlacedZone, v: View) => {
      const el = zoneEls.current[index];
      if (el) {
        const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
        const k = base * v.scale;
        el.style.left = `${(z.x + v.panX) * k}px`;
        el.style.top = `${(z.y + v.panY) * k}px`;
        el.style.width = `${z.width * k}px`;
        el.style.height = `${z.height * k}px`;
        el.style.transform = `rotate(${z.rotation_deg}deg)`;
      }
      if (selectedRef.current === index) applyFrame(z, v);
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

  /** Repaints the LED dots from the last frame, at the zones' live geometry and view. */
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
    const frame = frameRef.current;
    // The dots take their colour from the daemon's frames when they are coming, and from the
    // editor's own draw otherwise.
    const pic = daemonLiveRef.current
      ? frame
        ? { width: frame.width, height: frame.height, data: frame.rgb, stride: 3 }
        : null
      : editorPicRef.current;
    const mode = samplingRef.current;
    const geom = liveRef.current;
    const unlit = unlitColor();
    const ringDark = "rgba(0, 0, 0, 0.62)";
    const ringLight = "rgba(255, 255, 255, 0.5)";
    for (let i = 0; i < geom.length; i++) {
      const z = geom[i];
      if (!z || !z.visible) continue;
      const info = infosRef.current.get(zoneKey(z));
      if (!info) continue;
      const local = layoutOf(z, info);
      const t = zoneTransform(z);
      // A zone whose device is not connected has no LEDs lit: its dots stay dark.
      const off = missingRef.current.has(i);
      // A point is one blob of light: its LEDs all sample the same rect, so one dot stands for them.
      const count = local.shape === "point" ? Math.min(1, local.leds.length) : local.leds.length;
      const minPx = local.shape === "point" ? 3 : DOT_MIN_PX;
      const radius = clamp(local.radius * k, minPx * dpr, DOT_MAX_PX * dpr * Math.max(1, v.scale * 0.5));
      for (let j = 0; j < count; j++) {
        const led = local.leds[j];
        if (!led) continue;
        const cell = canvasCell(t, led);
        const x = cell.cx * k + ox;
        const y = cell.cy * k + oy;
        // A dot that is off the canvas is not drawn: at a high zoom most of the desk is not on
        // screen, and a frame that draws dots nobody can see is a frame wasted.
        if (x + radius < 0 || y + radius < 0 || x - radius > ov.width || y - radius > ov.height) continue;
        let fill: string;
        let ring = ringDark;
        if (pic && !off) {
          const c = sampleLed(pic, cell, mode);
          fill = `rgb(${c[0]} ${c[1]} ${c[2]})`;
          if (luminance(c) < 70) ring = ringLight;
        } else {
          // No picture, or no device: the dots still show where the LEDs sit.
          fill = unlit;
          ring = ringDark;
        }
        octx.beginPath();
        octx.arc(x, y, radius + dpr, 0, Math.PI * 2);
        octx.fillStyle = ring;
        octx.fill();
        octx.beginPath();
        octx.arc(x, y, radius, 0, Math.PI * 2);
        octx.fillStyle = fill;
        octx.fill();
      }
    }
  }, [layoutOf, unlitColor]);

  const queuePaint = useCallback(() => {
    if (paintQueued.current) return;
    paintQueued.current = true;
    requestAnimationFrame(() => {
      paintQueued.current = false;
      paintOverlay();
    });
  }, [paintOverlay]);

  /**
   * Reads the editor's own draw, so the LED dots can be coloured from it: the picture the
   * reader sees and the dots on it are the same picture.
   */
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
      editorPicRef.current = {
        width: canvas.width,
        height: canvas.height,
        data: img.data,
        stride: 4,
      };
    } catch {
      editorPicRef.current = null;
    }
  }, []);

  /**
   * The editor's draw animates on its own clock, so the dots have to follow it: while it is
   * showing, the canvas is repainted every frame.
   */
  useEffect(() => {
    if (!editorDraws) return;
    let raf = 0;
    const step = () => {
      readEditorPicture();
      paintOverlay();
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [editorDraws, paintOverlay, readEditorPicture]);

  /** Draws the whole view: the effect, the zones, the label and the LEDs. */
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
      placeLabel(labelIndexRef.current, v);
      queuePaint();
    },
    [applyGeom, placeLabel, queuePaint],
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
  // the elements follow it. A drag owns the geometry while it lasts: the page re-renders on
  // every frame of it, and the draft it is dragging is still the one it started from.
  useLayoutEffect(() => {
    const v = viewRef.current;
    if (!dragRef.current) {
      liveRef.current = zones.map((z) => ({ ...z }));
      zones.forEach((z, i) => applyGeom(i, z, v));
    }
    // The label comes and goes with the pointer, so it is placed whenever it changes, drag or not.
    placeLabel(labelIndexRef.current, v);
    queuePaint();
  }, [zones, applyGeom, placeLabel, queuePaint, selected, labelIndex, stageBox]);

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

  /** What a drag says in words beside the pointer: where the zone is, how big, how far it is turned. */
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

  const beginDrag = (kind: DragKind, index: number, e: PointerEvent<HTMLElement>, handle: HandleDef | null = null) => {
    e.stopPropagation();
    e.preventDefault();
    const z = liveRef.current[index];
    if (!z) return;
    onSelect(index);
    // The zone under the finger names itself, even before it is picked.
    setPressed(index);
    dragRef.current = {
      kind,
      index,
      start: toLogical(e),
      orig: { ...z },
      handle,
    };
    const el = zoneEls.current[index];
    if (el) el.dataset.dragging = "true";
    onDraggingChange?.(true);
    // The arrow keys work on the picked zone once the stage has the focus.
    stageRef.current?.focus({ preventScroll: true });
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
    const base = (stageRef.current?.clientWidth ?? CANVAS_W) / CANVAS_W;
    const threshold = SNAP_PIXELS / (base * viewRef.current.scale);
    const others = [CANVAS_BOX, ...liveRef.current.filter((_, i) => i !== d.index)];
    let next: PlacedZone;
    let readout: string;
    if (d.kind === "move") {
      // The zone snaps to the grid, or to another zone's line; Alt frees it.
      const want = {
        x: clamp(o.x + dx, 0, CANVAS_W - o.width),
        y: clamp(o.y + dy, 0, CANVAS_H - o.height),
        width: o.width,
        height: o.height,
      };
      const snap = snapMove(want, others, threshold, e.altKey);
      next = {
        ...cur,
        x: clamp(snap.x, 0, CANVAS_W - o.width),
        y: clamp(snap.y, 0, CANVAS_H - o.height),
      };
      applyGuides({ guideX: snap.guideX, guideY: snap.guideY });
      const gap = snap.gapX !== null && snap.gapY !== null ? Math.min(snap.gapX, snap.gapY) : (snap.gapX ?? snap.gapY);
      const where = gap === null ? "" : gap < 0.05 ? " · on the guide" : ` · ${gap.toFixed(1)} u to the guide`;
      readout = `x ${next.x.toFixed(0)} · y ${next.y.toFixed(0)}${where}`;
    } else if (d.kind === "resize" && d.handle) {
      // The dragged edge snaps to the grid and to other zones' lines while the zone is square to
      // the canvas; a turned zone follows the pointer freely. Alt frees it, as it does for a move.
      let q = { x: clamp(p.x, 0, CANVAS_W), y: clamp(p.y, 0, CANVAS_H) };
      let guideX: number | null = null;
      let guideY: number | null = null;
      const square = Math.abs(Math.sin((o.rotation_deg * Math.PI) / 180)) < 1e-6 && Math.cos((o.rotation_deg * Math.PI) / 180) > 0;
      if (square && !e.altKey) {
        if (d.handle.sx !== 0) {
          const line = nearestLine(q.x, others.flatMap((z) => zoneLines(z, "x")), threshold);
          guideX = line;
          q = { ...q, x: line ?? gridOf(q.x) };
        }
        if (d.handle.sy !== 0) {
          const line = nearestLine(q.y, others.flatMap((z) => zoneLines(z, "y")), threshold);
          guideY = line;
          q = { ...q, y: line ?? gridOf(q.y) };
        }
      }
      next = resizeBy(o, d.handle, q, e.shiftKey, e.altKey);
      // Shift rescales the box after the snap, so the guide would no longer be where the edge is.
      if (e.shiftKey) {
        guideX = null;
        guideY = null;
      }
      applyGuides({ guideX, guideY });
      readout = `w ${next.width.toFixed(0)} · h ${next.height.toFixed(0)}${e.shiftKey ? " · locked" : ""}`;
    } else {
      const cx = o.x + o.width / 2;
      const cy = o.y + o.height / 2;
      let deg = (Math.atan2(p.y - cy, p.x - cx) * 180) / Math.PI + 90;
      deg = ((deg % 360) + 360) % 360;
      if (e.shiftKey) deg = Math.round(deg / 15) * 15;
      next = { ...cur, rotation_deg: Math.round((deg % 360) * 10) / 10 };
      applyGuides({ guideX: null, guideY: null });
      readout = `${next.rotation_deg.toFixed(1)}°${e.shiftKey ? " · 15° steps" : ""}`;
    }
    showReadout(e, readout);
    liveRef.current[d.index] = next;
    applyGeom(d.index, next, viewRef.current);
    placeLabel(labelIndexRef.current, viewRef.current);
    notifyLive(d.index);
    queuePaint();
  };

  const endDrag = (e: PointerEvent<HTMLDivElement>) => {
    // The finger is off the zone: its label goes with it.
    setPressed(null);
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

  // The keys work on the picked zone, as in Figma: the arrows nudge it by 1 (Shift: 10) and
  // Delete or Backspace takes it off the desk (it hides; the zone list brings it back), and Escape
  // lets go of it. They answer only while the focus is on the canvas or on nothing, so the arrow
  // keys still move a slider or a segment in the panel beside it.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      const index = selectedRef.current;
      if (index === null) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      const stage = stageRef.current;
      const onCanvas = !!target && !!stage && stage.contains(target);
      const onNothing = !target || target === document.body;
      if (!onCanvas && !onNothing) return;
      // The zone's own buttons (the label's remove button, a handle) keep their own keys.
      if (target && target.closest("button, input, select, textarea")) return;
      const z = zonesRef.current[index];
      if (!z) return;
      const step = e.shiftKey ? 10 : 1;
      let patch: Partial<PlacedZone> | null = null;
      if (e.key === "ArrowLeft") patch = { x: clamp(z.x - step, 0, CANVAS_W - z.width) };
      else if (e.key === "ArrowRight") patch = { x: clamp(z.x + step, 0, CANVAS_W - z.width) };
      else if (e.key === "ArrowUp") patch = { y: clamp(z.y - step, 0, CANVAS_H - z.height) };
      else if (e.key === "ArrowDown") patch = { y: clamp(z.y + step, 0, CANVAS_H - z.height) };
      else if ((e.key === "Delete" || e.key === "Backspace") && z.visible) patch = { visible: false };
      else if (e.key === "Escape") {
        e.preventDefault();
        onSelect(null);
        return;
      }
      if (!patch) return;
      e.preventDefault();
      onChange(index, patch);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onChange, onSelect]);

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

  // The view belongs to the layout: it is remembered per layout, and restored when the editor
  // comes back to it. A layout that was never zoomed has no entry at all.
  useEffect(() => {
    if (!layoutId) return;
    if (view.scale === 1 && view.panX === 0 && view.panY === 0) forgetView(layoutId);
    else writeView(layoutId, view);
  }, [layoutId, view]);

  /**
   * Eases the view to a new one, with the house curve.
   *
   * The wheel does not always use this: a mouse notch does, a trackpad's continuous deltas do
   * not. The buttons and the fit control are a single, discrete action, so they move.
   */
  const animateTo = useCallback(
    (target: View, ms: number = ZOOM_MS) => {
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
      // A mouse notch is a step the reader can see: the view eases to it over WHEEL_MS, and the
      // next notch retargets from where the canvas is now, so nothing queues up. A trackpad's
      // small, continuous deltas and a ctrl+wheel are applied at once.
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

  const selectedZone = selected !== null ? zones[selected] : undefined;

  const base = (stageBox.w || CANVAS_W) / CANVAS_W;
  const k = base * view.scale;

  /**
   * The scale bar: a round number of desk units that is about as wide as the bar itself, so
   * the reader can measure a zone with it. It says what the bar is in words - a share of the
   * desk - and keeps the units beside it as the precise value.
   */
  const scaleBar = useMemo(() => {
    if (!stageBox.w) return null;
    const pxPerUnit = base * view.scale;
    const steps = [1, 2, 5, 10, 20, 40, 80, 160, 320];
    const raw = 120 / pxPerUnit;
    let units = steps[0] as number;
    for (const s of steps) if (s <= raw) units = s;
    // The bar's share of the desk's width, when that share is a clean one.
    const denom = CANVAS_W / units;
    const share =
      Number.isInteger(denom) && denom >= 2 && denom <= 16 ? `1/${denom} of the desk` : null;
    return { units, px: Math.round(units * pxPerUnit), share };
  }, [base, view.scale, stageBox.w]);

  // The label's text: the zone's name and its LEDs, as the zone list says it.
  const labelZone = labelIndex !== null ? zones[labelIndex] : undefined;
  const labelFound = labelZone ? findZoneInfo(devices, labelZone.device_id, labelZone.zone_index) : null;
  const labelMissing = labelIndex !== null && missing.has(labelIndex);
  const labelText = labelZone
    ? labelFound
      ? `${labelFound.zone.name} ${labelFound.zone.led_count}`
      : phone
        ? "not connected"
        : "device not found"
    : "";
  const labelAccent = labelZone ? accentOf(devices, labelZone.device_id) : -1;

  return (
    <div
      ref={stageRef}
      className={`stage ${desk.desk} ${desk.canvas}`}
      data-editor-draw={editorDraws ? "true" : "false"}
      tabIndex={-1}
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
      {fading && !editorGone && (
        // The picture that is fading out. It sits under the new one, which fades in over it.
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
        // it when they arrive.
        <div
          ref={editorLayerRef}
          className={`${desk.editorPreview} ${fading ? desk.editorFadeIn : ""}`}
          data-editor-preview="true"
          aria-hidden="true"
          style={{
            opacity: editorDraws ? 1 : 0,
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
          // The daemon's frames take over from the editor's own draw with a crossfade.
          opacity: daemonLive ? 1 : 0,
        }}
      />
      {/* The picture is the backdrop, not the subject: a quiet veil keeps the zones' outlines
          and dots readable over any effect. */}
      <div className={desk.veil} aria-hidden="true" />
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
        data-guide="v"
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
            data-shape={found ? resolveShape(z.shape, found.zone.kind) : "none"}
            style={style}
            onPointerDown={(e) => beginDrag("move", i, e)}
            onPointerEnter={() => setHover(i)}
            onPointerLeave={() => setHover((h) => (h === i ? null : h))}
            onFocus={() => onSelect(i)}
          />
        );
      })}

      {selectedZone && selected !== null && !missing.has(selected) && (
        // Figma's selection frame: eight resize handles and a rotation handle, drawn in the
        // zone's own turned box. The frame never takes a click itself; its handles do.
        <div
          ref={frameEl}
          className={desk.frame}
          data-frame="true"
          data-narrow-x={selectedZone.width * k < EDGE_HANDLE_MIN_PX}
          data-narrow-y={selectedZone.height * k < EDGE_HANDLE_MIN_PX}
          style={{
            left: `${(selectedZone.x + view.panX) * k}px`,
            top: `${(selectedZone.y + view.panY) * k}px`,
            width: `${selectedZone.width * k}px`,
            height: `${selectedZone.height * k}px`,
            transform: `rotate(${selectedZone.rotation_deg}deg)`,
          }}
        >
          <span className={desk.stem} aria-hidden="true" />
          <span
            className={desk.rotateHandle}
            data-handle="rotate"
            role="button"
            tabIndex={-1}
            aria-label="Rotate this zone"
            onPointerDown={(e) => beginDrag("rotate", selected, e)}
          />
          {HANDLES.map((h) => (
            <span
              key={h.id}
              className={desk.handle}
              data-handle={h.id}
              role="button"
              tabIndex={-1}
              aria-label={`Resize this zone from the ${h.id} handle`}
              style={{
                left: `${(h.sx + 1) * 50}%`,
                top: `${(h.sy + 1) * 50}%`,
                cursor: handleCursor(h, selectedZone.rotation_deg),
              }}
              onPointerDown={(e) => beginDrag("resize", selected, e, h)}
            />
          ))}
        </div>
      )}

      {labelIndex !== null && labelZone && (
        // The zone's name, beside it while it is hovered, held or picked. Nothing else on the
        // canvas is text.
        <div
          key={labelIndex}
          ref={labelEl}
          className={desk.label}
          data-zone-label="true"
          data-zone-index={labelIndex}
          data-selected={selected === labelIndex}
          data-missing={labelMissing}
          style={
            {
              "--zc": labelMissing ? "var(--down)" : `var(${ACCENTS[labelAccent] ?? "--a1"})`,
            } as CSSProperties
          }
        >
          <span className={desk.labelText}>{labelText}</span>
          {removable(labelIndex) && (
            // A ghost cannot be picked on the canvas, so its label is the way to it: once the
            // zone is picked the label carries the way to take it off the layout. The editor's
            // own small button is used, not a raw one.
            <span
              className={desk.labelRemove}
              onPointerDown={(e) => {
                e.stopPropagation();
                e.preventDefault();
              }}
            >
              <Chip
                icon={<X size={10} weight="bold" />}
                label={`Remove ${labelText} from the layout`}
                onClick={() => onRemoveZone?.(labelIndex)}
              >
                {null}
              </Chip>
            </span>
          )}
        </div>
      )}

      <div
        className={desk.controls}
        onPointerDown={(e) => e.stopPropagation()}
        onWheel={(e) => e.stopPropagation()}
      >
        <IconButton label="Zoom out" icon={<Minus size={13} />} size="sm" onClick={() => zoomBy(1 / 1.3)} />
        <span className={desk.zoomValue}>{Math.round(view.scale * 100)}%</span>
        <IconButton label="Zoom in" icon={<Plus size={13} />} size="sm" onClick={() => zoomBy(1.3)} />
        <IconButton label="Fit the canvas" icon={<ArrowsOut size={13} />} size="sm" onClick={fit} />
      </div>

      {scaleBar && (
        // The scale: how long a line on the desk is, in the daemon's own units.
        <div
          className={desk.scaleBar}
          role="img"
          aria-label={`Scale: ${scaleBar.units} of the desk's ${CANVAS_W} units${
            scaleBar.share ? `, ${scaleBar.share}` : ""
          }`}
        >
          <span className={desk.scaleLine} style={{ width: `${scaleBar.px}px` }} />
          <span className={desk.scaleLabel}>
            {scaleBar.share && <span className={desk.scaleShare}>{scaleBar.share}</span>}
            <span className="fig">{scaleBar.units} units</span>
          </span>
        </div>
      )}
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
