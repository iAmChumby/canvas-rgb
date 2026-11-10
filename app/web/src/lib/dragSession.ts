// The builder's drag session (#257 phase 2): one place that lifts a thing, follows a pointer or a
// keyboard, reads the drop zones off the page, and says what happened in the live region.
//
// Native pointer events only, as the spec decides: the repo already owns pointer capture, press
// cancelling and a touch reorder, and the targets that matter (a canvas with area semantics, a
// list, a row) are custom. No dependency.
//
// A rule of its own: a whole drop is one undo step. The session never reorders while a pointer
// moves; it names the insertion point and the builder commits it once, on the drop.

import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";
import type { StackLayer } from "@/proto/StackLayer";
import { CANVAS_H, CANVAS_W, moveLayer } from "@/views/effects/stackDraft";
import { dropTargetWords, type DragPayloadKind, type DropBoxes, type DropRow, type DropTarget } from "@/lib/dropTargets";

/** A press that travels this far is a drag; below it, it is still a tap or a click. */
export const DRAG_SLOP_PX = 4;
/** A finger lifts a tile after this long; before it, the gesture can still scroll the grid. */
export const TOUCH_HOLD_MS = 300;

/** What was lifted: a library effect, a mask from the Masks tab, or a layer already in the stack. */
export type DragSource =
  | { kind: "effect"; effect: string; name: string }
  | { kind: "mask"; effect: string; name: string }
  | { kind: "layer"; id: string; name: string };

export type DragMode = "pointer" | "keyboard" | "touch";

/** What the page draws while a drag runs. */
export interface DragState {
  source: DragSource | null;
  mode: DragMode | null;
  /** Where a drop would land now: the ghost's target, the outline, the insertion line. */
  target: DropTarget | null;
  /** The pointer, in client pixels: the ghost follows it 1:1 (no easing, spec §4). */
  point: { x: number; y: number } | null;
  /** Why the drop would refuse here (a mask on empty picture), said beside the pointer. */
  hint: string | null;
}

export const NO_DRAG: DragState = { source: null, mode: null, target: null, point: null, hint: null };

export function lifted(source: DragSource, mode: DragMode, target: DropTarget | null = null): DragState {
  return { source, mode, target, point: null, hint: null };
}

/** The names a target is said by. */
export interface TargetNames {
  layout: Layout | null;
  devices: DeviceInfo[] | null;
  layers: Pick<StackLayer, "id" | "name">[];
}

/** The live region's words for each step. The keyboard test proves these verbatim. */
export function liftAnnouncement(source: DragSource): string {
  if (source.kind === "effect") {
    return `Lifted ${source.name}. Left and Right pick where it goes, Up and Down pick a place in the list; Enter drops it, Escape cancels.`;
  }
  if (source.kind === "mask") {
    return `Lifted ${source.name} as a mask. Up and Down pick a layer; Enter drops it, Escape cancels.`;
  }
  return `Lifted ${source.name}. Up and Down pick a place in the list; Enter drops it, Escape cancels.`;
}

/** Why a mask refused to land on the picture: nothing is there to wear it. */
export const MASK_EMPTY_HINT = "No layer there. A mask needs a layer.";

export function targetAnnouncement(target: DropTarget, names: TargetNames): string {
  return `${dropTargetWords(target, names)}.`;
}

export function dropAnnouncement(source: DragSource, target: DropTarget, names: TargetNames): string {
  if (source.kind === "mask") {
    const layer =
      target.kind === "maskLayer" || target.kind === "row"
        ? names.layers.find((entry) => entry.id === target.id)
        : undefined;
    return layer ? `${source.name} is now the mask on ${layer.name}.` : `${source.name} added.`;
  }
  const verb = source.kind === "effect" ? "added" : "moved";
  if (target.kind === "stackSlot") {
    const words = dropTargetWords(target, names);
    const place = words.startsWith("Above ") ? `above ${words.slice("Above ".length)}` : `to the ${words.toLowerCase()}`;
    return `${source.name} ${verb} ${place}.`;
  }
  return `${source.name} ${verb} to ${dropTargetWords(target, names)}.`;
}

export const CANCEL_ANNOUNCEMENT = "Cancelled; nothing changed.";

/** One press, before it is a drag. */
export interface PressPoint {
  x: number;
  y: number;
}

/** A pointer that travelled past the slop. */
export function pressMoved(from: PressPoint, to: PressPoint): boolean {
  return Math.abs(to.x - from.x) > DRAG_SLOP_PX || Math.abs(to.y - from.y) > DRAG_SLOP_PX;
}

/** A mouse or a pen lifts on the move; a finger lifts on the hold. */
export function liftsOnMove(pointerType: string): boolean {
  return pointerType !== "touch";
}

export function liftsOnHold(pointerType: string): boolean {
  return pointerType === "touch";
}

/**
 * The one drop-target registry: every zone the page has, read in one pass. A box with no size on
 * the page is left out, so a hidden list or an unmeasured stage is never a target.
 */
export function readDropBoxes(root: ParentNode | null = typeof document === "undefined" ? null : document): DropBoxes {
  const boxes: DropBoxes = { stage: null, strip: null, rows: [] };
  if (!root) return boxes;
  const read = (selector: string) => {
    const el = root.querySelector<HTMLElement>(selector);
    if (!el) return null;
    const box = el.getBoundingClientRect();
    return box.width > 0 && box.height > 0
      ? { left: box.left, top: box.top, width: box.width, height: box.height }
      : null;
  };
  boxes.stage = read(".rd-canvas");
  boxes.strip = read(".rd-drop-strip");
  const rows = [...root.querySelectorAll<HTMLElement>(".rd-layers .layer-row")];
  boxes.rows = rows.flatMap((row, place): DropRow[] => {
    const id = row.dataset.layerId;
    if (!id) return [];
    const box = row.getBoundingClientRect();
    if (box.height <= 0) return [];
    const own = row.dataset.layerIndex === undefined ? Number.NaN : Number(row.dataset.layerIndex);
    return [
      {
        id,
        // The rows are drawn top first; the stack's own order is bottom first.
        index: Number.isInteger(own) ? own : rows.length - 1 - place,
        top: box.top,
        bottom: box.bottom,
      },
    ];
  });
  return boxes;
}

/** The visible devices on the desk, in the layout's own order, each once. */
export function deskDevices(layout: Layout | null): string[] {
  const seen: string[] = [];
  for (const zone of layout?.zones ?? []) {
    if (!zone.visible || zone.width <= 0 || zone.height <= 0) continue;
    if (!seen.includes(zone.device_id)) seen.push(zone.device_id);
  }
  return seen;
}

/**
 * The keyboard's stage targets, in the cycle order the spec gives: the whole canvas, each device
 * on the desk, then a new rectangle. A keyboard drop on a device takes the whole device.
 */
export function stageTargets(layout: Layout | null): DropTarget[] {
  return [
    { kind: "canvas" },
    ...deskDevices(layout).map((device): DropTarget => ({ kind: "device", device })),
    { kind: "rect", x: Math.round((CANVAS_W - 120) / 2), y: Math.round((CANVAS_H - 80) / 2), width: 120, height: 80 },
  ];
}

/** The stack's insertion points, bottom first: one under every layer and one at the top. */
export function stackSlots(count: number): DropTarget[] {
  return Array.from({ length: count + 1 }, (_, at): DropTarget => ({ kind: "stackSlot", at }));
}

/** Every layer a mask may land on, bottom first, so Up (a step of +1) walks up the drawn list. */
export function maskTargets(layers: Pick<StackLayer, "id">[]): DropTarget[] {
  return layers.map((layer, index): DropTarget => ({ kind: "maskLayer", id: layer.id, index }));
}

/** The target `step` places from `current` in `targets`, wrapping at both ends. */
export function cycleTarget(
  targets: DropTarget[],
  current: DropTarget | null,
  step: 1 | -1,
): DropTarget | null {
  if (targets.length === 0) return null;
  const key = targetKey(current);
  const at = key === null ? -1 : targets.findIndex((target) => targetKey(target) === key);
  if (at < 0) return step > 0 ? (targets[0] ?? null) : (targets[targets.length - 1] ?? null);
  return targets[(at + step + targets.length) % targets.length] ?? null;
}

/** Two targets are the same place when this matches: the keyboard's cursor is kept on a cycle. */
export function targetKey(target: DropTarget | null): string | null {
  if (!target) return null;
  switch (target.kind) {
    case "canvas":
      return "canvas";
    case "device":
      return `device:${target.device}`;
    case "zone":
      return `zone:${target.device}:${target.zone}`;
    case "rect":
      return "rect";
    case "stackSlot":
      return `slot:${target.at}`;
    case "row":
      return `row:${target.id}`;
    case "maskLayer":
      return `maskLayer:${target.id}`;
  }
}

/**
 * A layer moved to an insertion point counted before it leaves its place: `at` is where the layer
 * would sit among the others, so a move downwards lands one index lower once it is lifted out.
 */
export function reorderToSlot(layers: StackLayer[], id: string, at: number): StackLayer[] {
  const from = layers.findIndex((layer) => layer.id === id);
  if (from < 0) return layers;
  const to = at > from ? at - 1 : at;
  return moveLayer(layers, id, to);
}

/** The kind of payload a source carries, as dropTargets names it. */
export function payloadOf(source: DragSource): DragPayloadKind {
  return source.kind;
}