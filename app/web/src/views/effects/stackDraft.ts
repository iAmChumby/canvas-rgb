// The stack builder's model (D95.5): the draft's layers, the values, the area's names and
// geometry, and the session's epoch/sequence. Everything here is pure, so the builder's rules
// are tested without a DOM (D95.6 tests 15–17).

import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { LayerArea } from "@/proto/LayerArea";
import type { Layout } from "@/proto/Layout";
import type { PropKind } from "@/proto/PropKind";
import type { Props } from "@/proto/props";
import type { PropValue } from "@/proto/PropValue";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { StackLayer } from "@/proto/StackLayer";
import type { StackLayerState } from "@/proto/StackLayerState";

/** `STACK_MAX_LAYERS`, `STACK_HTML_MAX`, `STACK_NAME_MAX` and `LAYER_NAME_MAX` from the spine. */
export const MAX_LAYERS = 8;
export const MAX_HTML_LAYERS = 2;
export const MAX_STACK_NAME = 60;
export const MAX_LAYER_NAME = 40;
/** The canvas the engine composes over (`CANVAS_WIDTH` × `CANVAS_HEIGHT`). */
export const CANVAS_W = 320;
export const CANVAS_H = 200;

export function isHtmlEffect(effect: string): boolean {
  return effect.startsWith("html:");
}

export function htmlLayers(layers: StackLayer[]): number {
  return layers.filter((layer) => isHtmlEffect(layer.effect)).length;
}

/** The next free layer id (`l1` … `l8`), or null when the stack is full. */
export function nextLayerId(layers: StackLayer[]): string | null {
  for (let n = 1; n <= MAX_LAYERS; n++) {
    const id = `l${n}`;
    if (!layers.some((layer) => layer.id === id)) return id;
  }
  return null;
}

/** Why a layer cannot be added, or null when one can. */
export function addLayerRefusal(layers: StackLayer[], effect?: string): string | null {
  if (layers.length >= MAX_LAYERS) return `A stack holds ${MAX_LAYERS} layers`;
  if (effect && isHtmlEffect(effect) && htmlLayers(layers) >= MAX_HTML_LAYERS) {
    return `A stack holds ${MAX_HTML_LAYERS} HTML layers`;
  }
  return null;
}

/** The value a property kind starts at. */
export function defaultValue(kind: PropKind): PropValue {
  switch (kind.type) {
    case "color":
      return { type: "color", value: kind.default };
    case "number":
      return { type: "number", value: kind.default };
    case "bool":
      return { type: "bool", value: kind.default };
    case "choice":
      return { type: "choice", value: kind.default };
    case "hue":
      return { type: "hue", value: kind.default };
    case "angle":
      return { type: "angle", value: kind.default };
  }
}

/** A layer's props over the schema's defaults: the map its controls show. */
export function layerValues(schema: PropertySchema | null | undefined, props: Props): Props {
  const values: Props = {};
  for (const def of schema?.properties ?? []) values[def.key] = defaultValue(def.kind);
  return { ...values, ...props };
}

/** A new layer, whole-canvas, Normal, 100 %, visible, with the effect's saved values or its defaults. */
export function makeLayer(
  effect: string,
  name: string,
  schema: PropertySchema | null | undefined,
  props: Props = {},
): StackLayer {
  return {
    id: "l1",
    name: name.slice(0, MAX_LAYER_NAME),
    effect,
    props: { ...layerValues(schema, props) },
    area: { kind: "canvas" },
    blend: "normal",
    opacity: 100,
    visible: true,
  };
}

/** The layer list with a new layer on top (the last one painted). */
export function addLayer(layers: StackLayer[], layer: StackLayer): StackLayer[] {
  return [...layers, layer];
}

export function removeLayer(layers: StackLayer[], id: string): StackLayer[] {
  return layers.filter((layer) => layer.id !== id);
}

/** Moves a layer to a bottom-first index; the others keep their order. */
export function moveLayer(layers: StackLayer[], id: string, to: number): StackLayer[] {
  const from = layers.findIndex((layer) => layer.id === id);
  if (from < 0) return layers;
  const target = Math.max(0, Math.min(layers.length - 1, to));
  if (target === from) return layers;
  const next = [...layers];
  const [moved] = next.splice(from, 1);
  if (!moved) return layers;
  next.splice(target, 0, moved);
  return next;
}

/** A copy with the layer's values and a fresh id, placed directly above it. */
export function duplicateLayer(
  layers: StackLayer[],
  id: string,
): { layers: StackLayer[]; id: string | null } {
  const from = layers.findIndex((layer) => layer.id === id);
  const source = layers[from];
  if (!source) return { layers, id: null };
  const fresh = nextLayerId(layers);
  if (!fresh) return { layers, id: null };
  if (isHtmlEffect(source.effect) && htmlLayers(layers) >= MAX_HTML_LAYERS) {
    return { layers, id: null };
  }
  const copy: StackLayer = {
    ...source,
    id: fresh,
    name: `${source.name} copy`.slice(0, MAX_LAYER_NAME),
    props: { ...source.props },
    area: { ...source.area },
  };
  const next = [...layers];
  next.splice(from + 1, 0, copy);
  return { layers: next, id: fresh };
}

export function patchLayer(
  layers: StackLayer[],
  id: string,
  patch: Partial<StackLayer>,
): StackLayer[] {
  return layers.map((layer) => (layer.id === id ? { ...layer, ...patch } : layer));
}

export function toggleLayer(layers: StackLayer[], id: string): StackLayer[] {
  return layers.map((layer) =>
    layer.id === id ? { ...layer, visible: !layer.visible } : layer,
  );
}

/** Every layer's values, namespaced `{layer.id}.{key}`: what a stack preset stores (D95.1.7). */
export function mergedProps(
  layers: StackLayer[],
  schemas: Record<string, PropertySchema | null | undefined>,
): Props {
  const merged: Props = {};
  for (const layer of layers) {
    const values = layerValues(schemas[layer.effect], layer.props);
    for (const [key, value] of Object.entries(values)) {
      merged[`${layer.id}.${key}`] = value;
    }
  }
  return merged;
}

/**
 * A merged map put back into the layers: each layer takes its own keys, and a key its effect
 * does not have is dropped, as the daemon's activate drops one (D95.4.3).
 */
export function applyMergedProps(
  layers: StackLayer[],
  merged: Props,
  schemas: Record<string, PropertySchema | null | undefined>,
): StackLayer[] {
  return layers.map((layer) => {
    const prefix = `${layer.id}.`;
    const known = new Set((schemas[layer.effect]?.properties ?? []).map((def) => def.key));
    const props: Props = { ...layer.props };
    for (const [key, value] of Object.entries(merged)) {
      if (!key.startsWith(prefix)) continue;
      const own = key.slice(prefix.length);
      // Only the keys the effect has. A schema that is not here yet is not a licence to write
      // any key: the layer keeps its own values until the schema says what it takes.
      if (!known.has(own)) continue;
      props[own] = value;
    }
    return { ...layer, props };
  });
}

/** The stack's schema as the API derives it: every layer's properties, namespaced (D95.4.3). */
export function stackSchema(
  name: string,
  layers: StackLayer[],
  schemas: Record<string, PropertySchema | null | undefined>,
): PropertySchema {
  const properties: PropertyDef[] = [];
  const needs = { screen: false, audio: false };
  const names: string[] = [];
  for (const layer of layers) {
    const schema = schemas[layer.effect];
    if (schema) {
      needs.screen ||= schema.needs.screen;
      needs.audio ||= schema.needs.audio;
      names.push(schema.name);
      for (const def of schema.properties) {
        properties.push({
          key: `${layer.id}.${def.key}`,
          label: `${layer.name} · ${def.label}`,
          kind: def.kind,
        });
      }
    } else {
      names.push(layer.effect);
    }
  }
  const count = names.length;
  return {
    id: `stack:${name}`,
    name,
    description: count === 0 ? "No layers yet" : `${count} ${count === 1 ? "layer" : "layers"}: ${names.join(", ")}`,
    needs,
    properties,
    presets: [],
  };
}

// ---------------------------------------------------------------- names

const RESERVED = new Set([
  "con", "prn", "aux", "nul",
  "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9",
  "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
]);

function nameProblem(name: string, max: number, what: string): string | null {
  const trimmed = name.trim();
  if (trimmed.length === 0) return `A ${what} needs a name`;
  if (trimmed !== name) return `"${name}" has a space at either end`;
  if (!/^[A-Za-z0-9 ._-]+$/.test(name)) {
    return `"${name}" may only hold letters, digits, spaces, '-', '_' and '.'`;
  }
  if (name.includes("..")) return `"${name}" must not hold '..'`;
  if (name.endsWith(" ") || name.endsWith(".")) return `"${name}" must not end in a space or a dot`;
  const base = name.split(".")[0] ?? name;
  if (RESERVED.has(base.toLowerCase())) return `"${base}" is a name Windows reserves`;
  if ([...name].length > max) return `A ${what} is at most ${max} characters`;
  return null;
}

/** What is wrong with a stack's name, or null. The daemon is the authority; this is the prompt. */
export function stackNameProblem(name: string, others: string[]): string | null {
  const basic = nameProblem(name, MAX_STACK_NAME, "stack name");
  if (basic) return basic;
  if (name.toLowerCase() === "preview") return `"${name}" is reserved for the stack preview route`;
  if (others.some((other) => other.toLowerCase() === name.toLowerCase())) {
    return `"${name}" is already another stack's name`;
  }
  return null;
}

export function layerNameProblem(name: string): string | null {
  return nameProblem(name, MAX_LAYER_NAME, "layer name");
}

// ---------------------------------------------------------------- the session

/** 16 random hex characters, made when the builder opens (D95.4.4). */
export function newEpoch(): string {
  const bytes = new Uint8Array(8);
  if (typeof globalThis.crypto?.getRandomValues === "function") {
    globalThis.crypto.getRandomValues(bytes);
  } else {
    for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  }
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/**
 * A builder session's epoch and sequence. `next()` is the sequence for the next PUT; `observe()`
 * takes the daemon's answer, so a superseded PUT (`applied: false`) advances the sequence past
 * the one that owns the preview. A dropped PUT is never resent: only a new change sends again.
 */
export class StackSession {
  readonly epoch: string;
  seq: number;
  constructor(epoch: string = newEpoch()) {
    this.epoch = epoch;
    this.seq = 0;
  }

  next(): { epoch: string; seq: number } {
    this.seq += 1;
    return { epoch: this.epoch, seq: this.seq };
  }

  observe(answer: { epoch?: string; seq?: number }): void {
    if (answer.epoch !== undefined && answer.epoch === this.epoch && answer.seq !== undefined && answer.seq > this.seq) {
      this.seq = answer.seq;
    }
  }
}

// ---------------------------------------------------------------- areas

export interface Bounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * The layers as the daemon must receive them (#118): every rect area clamped to the canvas.
 * The daemon refuses an edge past the canvas, but a stack saved before that rule can still hold
 * one, and the editor sends stored areas back as they are. The compositor already draws such a
 * layer into its on-canvas box, so clamping to that box changes nothing on screen and lets the
 * stack save again. A rect wholly off the canvas becomes an empty one, which paints nothing,
 * as before.
 */
export function layersForServer(layers: StackLayer[]): StackLayer[] {
  return layers.map((layer) => {
    const area = layer.area;
    if (area.kind !== "rect") return layer;
    const x0 = Math.min(CANVAS_W, Math.max(0, area.x));
    const y0 = Math.min(CANVAS_H, Math.max(0, area.y));
    const x1 = Math.min(CANVAS_W, Math.max(x0, area.x + area.width));
    const y1 = Math.min(CANVAS_H, Math.max(y0, area.y + area.height));
    if (x0 === area.x && y0 === area.y && x1 - x0 === area.width && y1 - y0 === area.height) return layer;
    return { ...layer, area: { kind: "rect", x: x0, y: y0, width: x1 - x0, height: y1 - y0 } };
  });
}

/** Where an area paints on the 320 × 200 canvas: a rect, or the union of its placed zones. */
export function areaBounds(area: LayerArea, layout: Layout | null): Bounds | null {
  if (area.kind === "canvas") return { x: 0, y: 0, width: CANVAS_W, height: CANVAS_H };
  if (area.kind === "rect") return { x: area.x, y: area.y, width: area.width, height: area.height };
  const zones = (layout?.zones ?? []).filter(
    (zone) =>
      zone.visible &&
      zone.device_id === area.device &&
      (area.kind === "device" || zone.zone_index === area.zone),
  );
  const first = zones[0];
  if (!first) return null;
  let x0 = first.x;
  let y0 = first.y;
  let x1 = first.x + first.width;
  let y1 = first.y + first.height;
  for (const zone of zones.slice(1)) {
    x0 = Math.min(x0, zone.x);
    y0 = Math.min(y0, zone.y);
    x1 = Math.max(x1, zone.x + zone.width);
    y1 = Math.max(y1, zone.y + zone.height);
  }
  return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
}

function deviceName(devices: DeviceInfo[] | null, id: string): string {
  return devices?.find((device) => device.id === id)?.name ?? id.replace(/^[a-z0-9]+:/i, "");
}

/** A device or a zone on the layout, by the id and index the area names. */
export function areaIsOnDesk(area: LayerArea, layout: Layout | null): boolean {
  if (area.kind === "canvas" || area.kind === "rect") return true;
  if (!layout) return false;
  return layout.zones.some(
    (zone) =>
      zone.visible &&
      zone.device_id === area.device &&
      (area.kind === "device" || zone.zone_index === area.zone),
  );
}

/** What an area is called in a layer row and in the picked layer's area line. */
export function areaName(
  area: LayerArea,
  layout: Layout | null,
  devices: DeviceInfo[] | null,
): string {
  if (area.kind === "canvas") return "Whole canvas";
  if (area.kind === "rect") {
    return `Rectangle ${Math.round(area.width)} × ${Math.round(area.height)}`;
  }
  if (!areaIsOnDesk(area, layout)) return "Not on the desk";
  if (area.kind === "device") return deviceName(devices, area.device);
  const zone = layout?.zones.find(
    (placed) => placed.device_id === area.device && placed.zone_index === area.zone,
  );
  const name = deviceName(devices, area.device);
  return zone?.label ? `${name} · ${zone.label}` : `${name} · Zone ${area.zone + 1}`;
}

/** A rectangle's position, said under its size. */
export function areaPosition(area: LayerArea): string | null {
  if (area.kind !== "rect") return null;
  return `x ${Math.round(area.x)}, y ${Math.round(area.y)}`;
}

export function roundRect(bounds: Bounds): Bounds {
  return {
    x: Math.round(bounds.x),
    y: Math.round(bounds.y),
    width: Math.round(bounds.width),
    height: Math.round(bounds.height),
  };
}

// ---------------------------------------------------------------- picker geometry

export interface PickerView {
  /** 1 is the whole desk in the stage. */
  zoom: number;
  panX: number;
  panY: number;
}

export interface StageBox {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Pointer → canvas through the stage's view (D95.2.6): `base = stage.clientWidth / 320`,
 * `canvas_x = css_x / (base × zoom) − pan_x`. A point outside the desk clamps at its edge, so a
 * drag that starts beside the desk still draws from the edge.
 */
export function pointerToCanvas(
  clientX: number,
  clientY: number,
  box: StageBox,
  view: PickerView,
): { x: number; y: number } {
  const base = box.width / CANVAS_W;
  const k = base * view.zoom || 1;
  const x = (clientX - box.left) / k - view.panX;
  const y = (clientY - box.top) / k - view.panY;
  return {
    x: Math.min(CANVAS_W, Math.max(0, x)),
    y: Math.min(CANVAS_H, Math.max(0, y)),
  };
}

/** The rectangle two canvas points draw, whole pixels and at least one. */
export function rectFromDrag(a: { x: number; y: number }, b: { x: number; y: number }): Bounds {
  const x0 = Math.min(a.x, b.x);
  const y0 = Math.min(a.y, b.y);
  const x1 = Math.max(a.x, b.x);
  const y1 = Math.max(a.y, b.y);
  const x = Math.round(Math.min(CANVAS_W - 1, Math.max(0, x0)));
  const y = Math.round(Math.min(CANVAS_H - 1, Math.max(0, y0)));
  return {
    x,
    y,
    width: Math.max(1, Math.round(Math.min(CANVAS_W - x, x1 - x0))),
    height: Math.max(1, Math.round(Math.min(CANVAS_H - y, y1 - y0))),
  };
}

/** The smallest a dragged rectangle gets, in canvas units. */
export const MIN_AREA = 4;

/**
 * A rectangle from its four edges, each rounded to a whole unit on its own, so the right and
 * bottom edges never round past the canvas the way a rounded x plus a rounded width can
 * (219.5 + 100.5 is 220 + 101 = 321).
 */
export function boundsFromEdges(x0: number, y0: number, x1: number, y1: number): Bounds {
  const left = Math.round(x0);
  const top = Math.round(y0);
  return { x: left, y: top, width: Math.round(x1) - left, height: Math.round(y1) - top };
}

/** A rectangle moved by a canvas-unit delta, kept whole inside the canvas. */
export function moveBounds(start: Bounds, dx: number, dy: number): Bounds {
  const x = Math.min(CANVAS_W - start.width, Math.max(0, start.x + dx));
  const y = Math.min(CANVAS_H - start.height, Math.max(0, start.y + dy));
  return boundsFromEdges(x, y, x + start.width, y + start.height);
}

/**
 * A rectangle resized by dragging one handle by a canvas-unit delta. `sx` and `sy` are the
 * handle's direction (-1 the left or top edge, 1 the right or bottom, 0 an axis it leaves alone);
 * the opposite edge stays put. The result stays inside the canvas and at least `MIN_AREA` each
 * way. `keepAspect` (a corner with Shift) holds the starting ratio.
 */
export function resizeBounds(
  start: Bounds,
  sx: -1 | 0 | 1,
  sy: -1 | 0 | 1,
  dx: number,
  dy: number,
  keepAspect = false,
): Bounds {
  const right = start.x + start.width;
  const bottom = start.y + start.height;
  // The smallest a side gets is MIN_AREA, or the side's own length when it started shorter (the
  // picker draws one-unit rectangles): a resize never grows a small area and never leaves the canvas.
  const minW = Math.min(MIN_AREA, start.width);
  const minH = Math.min(MIN_AREA, start.height);
  let x0 = start.x;
  let x1 = right;
  let y0 = start.y;
  let y1 = bottom;
  if (sx < 0) x0 = Math.max(0, Math.min(right - minW, start.x + dx));
  if (sx > 0) x1 = Math.min(CANVAS_W, Math.max(start.x + minW, right + dx));
  if (sy < 0) y0 = Math.max(0, Math.min(bottom - minH, start.y + dy));
  if (sy > 0) y1 = Math.min(CANVAS_H, Math.max(start.y + minH, bottom + dy));
  if (keepAspect && sx !== 0 && sy !== 0) {
    const room = {
      w: sx > 0 ? CANVAS_W - start.x : right,
      h: sy > 0 ? CANVAS_H - start.y : bottom,
    };
    const most = Math.min(room.w / start.width, room.h / start.height);
    const least = Math.max(minW / start.width, minH / start.height);
    const want = Math.max((x1 - x0) / start.width, (y1 - y0) / start.height);
    const scale = Math.min(most, Math.max(least, want));
    const w = start.width * scale;
    const h = start.height * scale;
    if (sx < 0) x0 = right - w;
    else x1 = start.x + w;
    if (sy < 0) y0 = bottom - h;
    else y1 = start.y + h;
  }
  return boundsFromEdges(x0, y0, x1, y1);
}

export interface AreaReadout {
  /** The line's first part: "Rectangle 110 × 46", "Whole canvas", "Keyboard · Zone 1". */
  title: string;
  /** The muted second part: "x 16, y 150". */
  detail: string | null;
}

export function areaReadout(
  area: LayerArea,
  layout: Layout | null,
  devices: DeviceInfo[] | null,
): AreaReadout {
  if (area.kind === "rect") {
    return {
      title: `Rectangle ${Math.round(area.width)} × ${Math.round(area.height)}`,
      detail: areaPosition(area),
    };
  }
  return { title: areaName(area, layout, devices), detail: null };
}

/** The tone a layer's status is drawn in: warn for a layer that is not running. */
export function statusWords(
  state: StackLayerState | undefined,
  lastMs: number | null | undefined,
): string | null {
  if (!state || state === "ok") return null;
  const ms = lastMs === null || lastMs === undefined ? "" : ` · ${Math.round(lastMs)} ms`;
  switch (state) {
    case "held":
      return `Held: too slow${ms}`;
    case "faulted":
      return `Faulted: too slow${ms}`;
    case "missing_effect":
      return "Missing";
    case "nested_stack":
      return "Nested stack";
    case "no_area":
      return "Not on the desk";
  }
}
