// The generic cases the desk can draw behind the zones, and the standard slots in them.
//
// A case is a drawing and a set of snap slots, nothing more: the effect is still sampled where
// each zone sits, so any setup can be modelled by dragging a zone anywhere. The slots are the
// places a PC has everywhere: fan positions, RAM slots, the GPU slot, the case edges where an
// ARGB strip runs, and a desk shelf for the keyboard, mouse, mic and headset.
//
// The mid tower's table is the daemon's too (`mid_tower_slots` in
// `crates/canvas-daemon/src/first_run.rs`, which lays out a new install): keep the two the same.

import type { CaseForm } from "@/proto/CaseForm";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DeviceKind } from "@/proto/DeviceKind";
import type { PlacedZone } from "@/proto/PlacedZone";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { ZoneShape } from "@/proto/ZoneShape";

export type SlotKind = "fan" | "ram" | "gpu" | "edge" | "desk";

export interface Slot {
  id: string;
  kind: SlotKind;
  /** The plain name of the place: "Front fan 1", "RAM slot 2", "Top edge", "Keyboard". */
  label: string;
  /** A desk place made for one kind of peripheral. */
  peripheral?: DeviceKind;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The measurements one case is drawn from. */
export interface CaseDims {
  x: number;
  y: number;
  w: number;
  h: number;
  /** The fan size. */
  fan: number;
  front: number;
  top: number;
  rear: number;
  ram: number;
  edges: number;
  /** Closed panels with a glass side; an open frame has rails instead. */
  panels: boolean;
}

const DIMS: Record<Exclude<CaseForm, "none">, CaseDims> = {
  mid_tower: { x: 10, y: 6, w: 250, h: 138, fan: 32, front: 3, top: 2, rear: 1, ram: 4, edges: 3, panels: true },
  full_tower: { x: 6, y: 4, w: 292, h: 144, fan: 34, front: 3, top: 3, rear: 1, ram: 4, edges: 3, panels: true },
  small_form_factor: { x: 60, y: 14, w: 170, h: 124, fan: 28, front: 2, top: 1, rear: 1, ram: 2, edges: 3, panels: true },
  open_frame: { x: 10, y: 8, w: 250, h: 134, fan: 32, front: 3, top: 2, rear: 1, ram: 4, edges: 3, panels: false },
};

/** The plain words of the form-factor picker. */
export const CASE_OPTIONS: { value: CaseForm; label: string }[] = [
  { value: "none", label: "No case" },
  { value: "mid_tower", label: "Mid tower" },
  { value: "full_tower", label: "Full tower" },
  { value: "small_form_factor", label: "Small (ITX)" },
  { value: "open_frame", label: "Open frame" },
];

export interface CaseSpec {
  form: Exclude<CaseForm, "none">;
  dims: CaseDims;
  /** The motherboard: where it sits inside the case. */
  board: { x: number; y: number; w: number; h: number };
  slots: Slot[];
}

/** The y where the desk shelf starts. */
export const DESK_Y = 152;

export function caseSpec(form: CaseForm): CaseSpec | null {
  if (form === "none") return null;
  const d = DIMS[form];
  const s = d.fan;
  const slots: Slot[] = [];
  const fy0 = d.y + 10;
  for (let i = 0; i < d.front; i++) {
    slots.push({ id: `front-fan-${i + 1}`, kind: "fan", label: `Front fan ${i + 1}`, x: d.x + 6, y: fy0 + i * (s + 4), w: s, h: s });
  }
  const topX0 = d.x + 6 + s + 10;
  for (let i = 0; i < d.top; i++) {
    slots.push({ id: `top-fan-${i + 1}`, kind: "fan", label: `Top fan ${i + 1}`, x: topX0 + i * (s + 4), y: d.y + 10, w: s, h: s });
  }
  for (let i = 0; i < d.rear; i++) {
    slots.push({ id: "rear-fan", kind: "fan", label: "Rear fan", x: d.x + d.w - 10 - s, y: d.y + 10, w: s, h: s });
  }
  const bx = d.x + 6 + s + 10;
  const by = d.y + 10 + s + 8;
  const bw = d.w - (bx - d.x) - 10;
  const bh = d.y + d.h - 16 - by;
  const rw = 9;
  const rh = 48;
  const total = d.ram * rw + (d.ram - 1) * 3;
  const start = bx + bw - 8 - total;
  for (let i = 0; i < d.ram; i++) {
    slots.push({ id: `ram-${i + 1}`, kind: "ram", label: `RAM slot ${i + 1}`, x: start + i * (rw + 3), y: by + 6, w: rw, h: rh });
  }
  slots.push({ id: "gpu", kind: "gpu", label: "Graphics card", x: bx + 6, y: Math.round(by + bh * 0.55), w: Math.round(bw * 0.58), h: 14 });
  const edges: Slot[] = [
    { id: "top-edge", kind: "edge", label: "Top edge", x: d.x + 10, y: d.y + 3, w: d.w - 20, h: 4 },
    { id: "bottom-edge", kind: "edge", label: "Bottom edge", x: d.x + 10, y: d.y + d.h - 8, w: d.w - 20, h: 4 },
    { id: "rear-edge", kind: "edge", label: "Rear edge", x: d.x + d.w - 6, y: d.y + 16, w: 4, h: d.h - 32 },
  ];
  slots.push(...edges.slice(0, d.edges));
  slots.push({ id: "desk-keyboard", kind: "desk", label: "Keyboard", peripheral: "keyboard", x: 14, y: 158, w: 110, h: 30 });
  slots.push({ id: "desk-mouse", kind: "desk", label: "Mouse", peripheral: "mouse", x: 132, y: 156, w: 22, h: 32 });
  slots.push({ id: "desk-mic", kind: "desk", label: "Mic", peripheral: "microphone", x: 162, y: 158, w: 16, h: 28 });
  slots.push({ id: "desk-headset", kind: "desk", label: "Headset", peripheral: "headset", x: 186, y: 157, w: 30, h: 30 });
  for (let i = 0; i < 3; i++) {
    slots.push({ id: `desk-${i + 1}`, kind: "desk", label: `Desk ${i + 1}`, x: 226 + 30 * i, y: 159, w: 26, h: 26 });
  }
  return { form, dims: d, board: { x: bx, y: by, w: bw, h: bh }, slots };
}

/** True when the centre of the zone's box lies in the slot. */
export function zoneInSlot(z: { x: number; y: number; width: number; height: number }, s: Slot): boolean {
  const cx = z.x + z.width / 2;
  const cy = z.y + z.height / 2;
  return cx >= s.x && cx <= s.x + s.w && cy >= s.y && cy <= s.y + s.h;
}

/** The slot whose centre is nearest the zone's centre, within `reach` canvas units. */
export function nearestSlot(
  z: { x: number; y: number; width: number; height: number },
  spec: CaseSpec,
  reach: number,
): Slot | null {
  const cx = z.x + z.width / 2;
  const cy = z.y + z.height / 2;
  let best: Slot | null = null;
  let bestD = reach;
  for (const s of spec.slots) {
    const d = Math.hypot(s.x + s.w / 2 - cx, s.y + s.h / 2 - cy);
    if (d <= bestD) {
      best = s;
      bestD = d;
    }
  }
  return best;
}

/** The shape a zone gets by default (the daemon's `default_shape`). */
function shapeFor(kind: DeviceKind, zone: ZoneInfo): ZoneShape {
  if (zone.kind.type === "matrix") return { type: "auto" };
  if (zone.name.toLowerCase().includes("fan") && zone.led_count > 1) return { type: "ring" };
  if (zone.kind.type === "single" || zone.led_count === 1) return { type: "point" };
  if (kind === "keyboard") return { type: "grid", columns: Math.max(1, Math.ceil(zone.led_count / 6)) };
  return { type: "strip" };
}

const KIND_RANK: Record<DeviceKind, number> = {
  motherboard: 0,
  led_strip: 1,
  keyboard: 2,
  mouse: 3,
  microphone: 4,
  headset: 5,
  dram: 6,
  gpu: 7,
  other: 8,
};

/** The slot a zone belongs in, among those not yet taken. Mirrors the daemon's `pick_slot`. */
function pickSlot(kind: DeviceKind, zone: ZoneInfo, slots: Slot[], used: boolean[]): number {
  const free = (want: (s: Slot) => boolean) => slots.findIndex((s, i) => !used[i] && want(s));
  const fanLike = zone.name.toLowerCase().includes("fan") && zone.led_count > 1;
  const single = zone.kind.type === "single" || zone.led_count === 1;
  let own = -1;
  if (fanLike) own = free((s) => s.kind === "fan");
  else if (kind === "dram") own = free((s) => s.kind === "ram");
  else if (kind === "gpu" && !single) own = free((s) => s.kind === "gpu");
  else if (kind === "keyboard" || kind === "mouse" || kind === "microphone" || kind === "headset") {
    own = free((s) => s.kind === "desk" && s.peripheral === kind);
  } else if (!single) own = free((s) => s.kind === "edge");
  if (own >= 0) return own;
  // A spare desk place is for what lives on a desk; a fan, RAM or strip with no slot left stays
  // in the case (the caller staggers it).
  const deskThing =
    kind === "keyboard" ||
    kind === "mouse" ||
    kind === "microphone" ||
    kind === "headset" ||
    (single && kind !== "dram" && kind !== "gpu" && !fanLike);
  return deskThing ? free((s) => s.kind === "desk" && !s.peripheral) : -1;
}

/**
 * Puts every zone of every connected device in its place in the case, as the first run does:
 * fans in the fan positions, RAM in the RAM slots, the GPU in the GPU slot, header and strip
 * zones along the case edges, and the keyboard, mouse, mic and headset on the desk shelf. A zone
 * keeps its name, its direction, its visibility and a shape the owner chose; zones the daemon
 * no longer lists stay where they are. With no case nothing moves.
 */
export function autoPlace(form: CaseForm, devices: DeviceInfo[] | null, existing: PlacedZone[]): PlacedZone[] {
  const spec = caseSpec(form);
  if (!spec || !devices) return existing;
  const slots = spec.slots;
  const used = slots.map(() => false);
  const sorted = [...devices].sort(
    (a, b) => KIND_RANK[a.kind] - KIND_RANK[b.kind] || a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
  );
  const out: PlacedZone[] = [];
  const placed = new Set<string>();
  let overflow = 0;
  for (const device of sorted) {
    for (const zone of device.zones) {
      if (zone.led_count === 0) continue;
      const old = existing.find((p) => p.device_id === device.id && p.zone_index === zone.index);
      const at = pickSlot(device.kind, zone, slots, used);
      let x: number;
      let y: number;
      let w: number;
      let h: number;
      if (at >= 0) {
        used[at] = true;
        const s = slots[at]!;
        [x, y, w, h] = [s.x, s.y, s.w, s.h];
      } else {
        overflow += 1;
        [x, y, w, h] = [150 + 6 * overflow, 60 + 6 * overflow, 24, 24];
      }
      placed.add(`${device.id}#${zone.index}`);
      out.push({
        device_id: device.id,
        zone_index: zone.index,
        x,
        y,
        width: w,
        height: h,
        rotation_deg: 0,
        reversed: old?.reversed ?? false,
        visible: old?.visible ?? true,
        shape: old && old.shape.type !== "auto" ? old.shape : shapeFor(device.kind, zone),
        ...(old?.label ? { label: old.label } : {}),
      });
    }
  }
  // Zones the daemon does not list (a device that is gone) keep their place.
  for (const p of existing) if (!placed.has(`${p.device_id}#${p.zone_index}`)) out.push(p);
  return out;
}
