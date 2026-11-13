// The building blocks of a scene group, as boxes on the group's own canvas (320 x 200).
//
// The builder's stage draws the group being tuned: every member's LEDs at the points the daemon
// samples (lib/arrangement.ts). Its area picker names the group's devices, and a layer's Device
// area is every light of that device in the group. A light or a device outside the group is not
// on this canvas; a rectangle is drawn in it directly.
//
// `buildGroupCanvas` takes a group; `wholeLightsCanvas` takes every light as one flow-across
// group, which is what the builder shows when it was not opened from a group.

import { buildGroupPlan, CANVAS_HEIGHT, CANVAS_WIDTH } from "@/lib/arrangement";
import type { Arrangement } from "@/proto/Arrangement";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Group } from "@/proto/Group";
import type { GroupMode } from "@/proto/GroupMode";
import type { LayerArea } from "@/proto/LayerArea";
import type { LightView } from "@/proto/LightView";

/** A box in canvas units. */
export interface CanvasBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface CanvasLight {
  id: string;
  label: string;
  device_id: string;
  zone_index: number;
  arrangement: Arrangement;
  reversed: boolean;
  /** The box the light's LEDs cover: its slot, or the cells of its matrix. */
  box: CanvasBox;
  /** Each sampled LED's centre, in canvas units. */
  leds: { x: number; y: number }[];
}

/** A group's members on its canvas. */
export interface GroupCanvas {
  /** The group's id, or `all` for the whole-lights group. */
  id: string;
  name: string;
  mode: GroupMode;
  lights: CanvasLight[];
  /** The devices with a light in this group, in first-seen order. */
  deviceIds: string[];
}

const unionBox = (boxes: CanvasBox[]): CanvasBox | null => {
  const first = boxes[0];
  if (!first) return null;
  let x0 = first.x;
  let y0 = first.y;
  let x1 = first.x + first.width;
  let y1 = first.y + first.height;
  for (const box of boxes.slice(1)) {
    x0 = Math.min(x0, box.x);
    y0 = Math.min(y0, box.y);
    x1 = Math.max(x1, box.x + box.width);
    y1 = Math.max(y1, box.y + box.height);
  }
  return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
};

/** Builds the canvas of a group from its members, mode and the lights the daemon lists. */
export function buildGroupCanvas(
  group: Pick<Group, "id" | "name" | "members" | "mode">,
  lights: readonly LightView[],
  devices: readonly DeviceInfo[],
): GroupCanvas {
  const plan = buildGroupPlan(
    group.members,
    group.mode,
    lights.map((l) => l.light),
    devices,
  );
  const out: CanvasLight[] = [];
  for (const samples of plan.lights) {
    const view = lights.find((l) => l.light.id === samples.light);
    if (!view) continue;
    const boxes: CanvasBox[] = [];
    const leds: { x: number; y: number }[] = [];
    for (const led of samples.leds) {
      if (!led.sampled) continue;
      boxes.push({ x: led.bounds[0], y: led.bounds[1], width: led.bounds[2] - led.bounds[0], height: led.bounds[3] - led.bounds[1] });
      leds.push({ x: led.center_x, y: led.center_y });
    }
    const box = unionBox(boxes);
    // A light with no sampled LED has no place on the canvas: the daemon draws it black.
    if (!box) continue;
    out.push({
      id: view.light.id,
      label: view.label,
      device_id: view.light.device_id,
      zone_index: view.light.zone_index,
      arrangement: view.light.arrangement,
      reversed: view.light.reversed,
      box,
      leds,
    });
  }
  const deviceIds: string[] = [];
  for (const light of out) if (!deviceIds.includes(light.device_id)) deviceIds.push(light.device_id);
  return { id: group.id, name: group.name, mode: group.mode, lights: out, deviceIds };
}

/** Every light as one flow-across group: what the builder shows outside a group. */
export function wholeLightsCanvas(
  lights: readonly LightView[],
  devices: readonly DeviceInfo[],
): GroupCanvas {
  return buildGroupCanvas(
    { id: "all", name: "Every light", members: lights.map((l) => l.light.id), mode: "flow_across" },
    lights,
    devices,
  );
}

/** The lights an area names on this canvas. */
export function lightsOfArea(canvas: GroupCanvas, area: LayerArea): CanvasLight[] {
  if (area.kind !== "device" && area.kind !== "zone") return [];
  return canvas.lights.filter(
    (light) => light.device_id === area.device && (area.kind === "device" || light.zone_index === area.zone),
  );
}

/** Where an area paints on the canvas, or null when it has no place on it. */
export function areaBox(canvas: GroupCanvas | null, area: LayerArea): CanvasBox | null {
  if (area.kind === "canvas") return { x: 0, y: 0, width: CANVAS_WIDTH, height: CANVAS_HEIGHT };
  if (area.kind === "rect") return { x: area.x, y: area.y, width: area.width, height: area.height };
  if (!canvas) return null;
  return unionBox(lightsOfArea(canvas, area).map((light) => light.box));
}

/** True when the area has a place on this canvas. */
export function areaOnCanvas(canvas: GroupCanvas | null, area: LayerArea): boolean {
  if (area.kind === "canvas" || area.kind === "rect") return true;
  if (!canvas) return false;
  return lightsOfArea(canvas, area).length > 0;
}

function deviceName(devices: readonly DeviceInfo[] | null, id: string): string {
  return devices?.find((device) => device.id === id)?.name ?? id.replace(/^[a-z0-9]+:/i, "");
}

/** What an area is called in a layer row and in the picked layer's area line. */
export function areaWords(canvas: GroupCanvas | null, area: LayerArea, devices: DeviceInfo[] | null): string {
  if (area.kind === "canvas") return "Whole canvas";
  if (area.kind === "rect") return `Rectangle ${Math.round(area.width)} × ${Math.round(area.height)}`;
  if (!areaOnCanvas(canvas, area)) return "Not in this group";
  if (area.kind === "device") return deviceName(devices, area.device);
  const light = canvas?.lights.find((l) => l.device_id === area.device && l.zone_index === area.zone);
  if (light) return light.label;
  return `${deviceName(devices, area.device)} · Zone ${area.zone + 1}`;
}

/** The lights under a canvas point, smallest box first: the smallest one wins a pick. */
export function lightsUnder(canvas: GroupCanvas | null, point: { x: number; y: number }): CanvasLight[] {
  return (canvas?.lights ?? [])
    .filter(
      (light) =>
        light.box.width > 0 &&
        light.box.height > 0 &&
        point.x >= light.box.x &&
        point.x <= light.box.x + light.box.width &&
        point.y >= light.box.y &&
        point.y <= light.box.y + light.box.height,
    )
    .sort((a, b) => a.box.width * a.box.height - b.box.width * b.box.height);
}

/** Each device the group holds, with the name the picker shows. */
export function canvasDevices(
  canvas: GroupCanvas | null,
  devices: DeviceInfo[] | null,
): { id: string; name: string }[] {
  return (canvas?.deviceIds ?? []).map((id) => ({ id, name: deviceName(devices, id) }));
}

/** The LEDs a light draws on the stage, capped so a huge light never makes a huge drawing. */
export const MAX_DRAWN_LEDS = 600;
