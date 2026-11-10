// Where a drop lands (#257 phase 2). Everything here is pure: a pointer, the boxes on the page,
// the desk's layout and the payload say one target —
// Canvas (the whole-canvas strip) | Device | Zone (with Shift) | Rect | StackSlot | Row.
// The builder's drag session asks this at every move; the keyboard asks the same names, so a
// pointer and a keyboard can never mean two different things (spec §4).

import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { LayerArea } from "@/proto/LayerArea";
import type { Layout } from "@/proto/Layout";
import type { StackLayer } from "@/proto/StackLayer";
import { CANVAS_H, CANVAS_W, pointerToStage, type StageBox } from "@/views/effects/stackDraft";

/** What is being dragged: a library effect, or a layer already in the stack. */
export type DragPayloadKind = "effect" | "layer";

export type DropTarget =
  | { kind: "canvas" }
  | { kind: "device"; device: string }
  | { kind: "zone"; device: string; zone: number }
  | { kind: "rect"; x: number; y: number; width: number; height: number }
  /** An insertion point in the stack, bottom-first: 0 is under every layer, `at` is the top. */
  | { kind: "stackSlot"; at: number }
  /** A row of the layer list, top first, with that layer's bottom-first index. */
  | { kind: "row"; id: string; index: number };

/** One row of the layer list as it sits on the page. */
export interface DropRow {
  id: string;
  /** The layer's bottom-first index in the stack. */
  index: number;
  top: number;
  bottom: number;
}

/** Every drop zone on the page, in one registry (lib/dragSession.ts reads it off the DOM). */
export interface DropBoxes {
  /** The picture's own box. */
  stage: StageBox | null;
  /** The "Whole canvas" strip above the picture, shown while a drag is in progress. */
  strip: StageBox | null;
  /** The layer rows, top first. */
  rows: DropRow[];
}

/** A rectangle dropped on empty picture: this size, centred on the pointer (spec §4). */
export const NEW_RECT = { width: 120, height: 80 } as const;
/** The smallest a zone counts for a hit: a 1 × 1 zone is still a target. */
const MIN_HIT = 0.5;

const inBox = (point: { x: number; y: number }, box: StageBox | null): boolean =>
  box !== null &&
  box.width > 0 &&
  box.height > 0 &&
  point.x >= box.left &&
  point.x <= box.left + box.width &&
  point.y >= box.top &&
  point.y <= box.top + box.height;

/** A new rectangle centred on a canvas point, whole units, held inside the canvas. */
export function rectAtPoint(point: { x: number; y: number }): { x: number; y: number; width: number; height: number } {
  const width = NEW_RECT.width;
  const height = NEW_RECT.height;
  const x = Math.round(Math.min(CANVAS_W - width, Math.max(0, point.x - width / 2)));
  const y = Math.round(Math.min(CANVAS_H - height, Math.max(0, point.y - height / 2)));
  return { x, y, width, height };
}

/** The visible zones under a canvas point, smallest first: the smallest one wins a pick. */
function zonesUnder(
  layout: Layout | null,
  point: { x: number; y: number },
): Layout["zones"] {
  return (layout?.zones ?? [])
    .filter(
      (zone) =>
        zone.visible &&
        zone.width >= MIN_HIT &&
        zone.height >= MIN_HIT &&
        point.x >= zone.x &&
        point.x <= zone.x + zone.width &&
        point.y >= zone.y &&
        point.y <= zone.y + zone.height,
    )
    .sort((a, b) => a.width * a.height - b.width * b.height);
}

/** The stack slot a pointer over the layer rows names: an effect goes where the line is drawn. */
function slotAtRow(rows: DropRow[], point: { x: number; y: number }): DropTarget | null {
  for (const row of rows) {
    if (point.y < row.top || point.y > row.bottom) continue;
    const middle = row.top + (row.bottom - row.top) / 2;
    // Above the row's middle is above the row: the layer would sit one place higher.
    return { kind: "stackSlot", at: point.y <= middle ? row.index + 1 : row.index };
  }
  return null;
}

function rowAtPoint(rows: DropRow[], point: { x: number; y: number }): DropTarget | null {
  for (const row of rows) {
    if (point.y >= row.top && point.y <= row.bottom) return { kind: "row", id: row.id, index: row.index };
  }
  return null;
}

/**
 * The drop target under a pointer, or null when the pointer is over neither the stage nor the
 * stack. A pointer on the whole-canvas strip is the Canvas; on the picture, the smallest zone
 * under it is the Device (or that Zone with Shift), and empty picture is a new Rectangle. Over
 * the layer list, a dragged layer names the Row it is over and a dragged effect names the
 * StackSlot the insertion line would sit at.
 */
export function dropTargetAt({
  point,
  boxes,
  layout,
  payload,
  shift = false,
}: {
  point: { x: number; y: number };
  boxes: DropBoxes;
  layout: Layout | null;
  payload: DragPayloadKind;
  shift?: boolean;
}): DropTarget | null {
  if (inBox(point, boxes.strip)) return { kind: "canvas" };
  if (inBox(point, boxes.stage)) {
    const box = boxes.stage as StageBox;
    const at = pointerToStage(point.x, point.y, box);
    const zones = zonesUnder(layout, at);
    const first = zones[0];
    if (first) {
      return shift
        ? { kind: "zone", device: first.device_id, zone: first.zone_index }
        : { kind: "device", device: first.device_id };
    }
    return { kind: "rect", ...rectAtPoint(at) };
  }
  return payload === "layer" ? rowAtPoint(boxes.rows, point) : slotAtRow(boxes.rows, point);
}

/** One name for every target, for the live region and the ghost's label. */
export function dropTargetWords(
  target: DropTarget,
  { layout, devices, layers }: { layout: Layout | null; devices: DeviceInfo[] | null; layers: Pick<StackLayer, "id" | "name">[] },
): string {
  switch (target.kind) {
    case "canvas":
      return "Whole canvas";
    case "device":
      return deviceName(devices, target.device);
    case "zone": {
      const zone = layout?.zones.find(
        (placed) => placed.device_id === target.device && placed.zone_index === target.zone,
      );
      return `${deviceName(devices, target.device)} · ${zone?.label ?? `Zone ${target.zone + 1}`}`;
    }
    case "rect":
      return "New rectangle";
    case "stackSlot": {
      if (target.at <= 0) return "Bottom of the list";
      const below = layers[target.at - 1];
      if (target.at >= layers.length) return "Top of the list";
      return below ? `Above ${below.name}` : "Top of the list";
    }
    case "row": {
      const layer = layers.find((entry) => entry.id === target.id);
      return layer ? `Row ${layer.name}` : "Row";
    }
  }
}

function deviceName(devices: DeviceInfo[] | null, id: string): string {
  return devices?.find((device) => device.id === id)?.name ?? id.replace(/^[a-z0-9]+:/i, "");
}

/** The layer area a target places on: a stack position is order, not an area, so it has none. */
export function dropArea(target: DropTarget): LayerArea | null {
  switch (target.kind) {
    case "canvas":
      return { kind: "canvas" };
    case "device":
      return { kind: "device", device: target.device };
    case "zone":
      return { kind: "zone", device: target.device, zone: target.zone };
    case "rect":
      return { kind: "rect", x: target.x, y: target.y, width: target.width, height: target.height };
    case "stackSlot":
    case "row":
      return null;
  }
}
