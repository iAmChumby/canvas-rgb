// The guided setup: what has to be mapped, in what order, and what a split does.
//
// The setup is not a wizard with its own state: it works on the layout. A part is mapped when
// its `part.mapped` says so, a split is a run of LEDs in a placement's `leds`, and the count of
// a zone is the device's own. That is why the progress survives a restart: it lives in the
// layout the editor saves.

import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DeviceKind } from "@/proto/DeviceKind";
import type { Layout } from "@/proto/Layout";
import type { PartKind } from "@/proto/PartKind";
import type { PlacedZone } from "@/proto/PlacedZone";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { ZoneShape } from "@/proto/ZoneShape";
import { slotsForBox } from "@/lib/caseForms";

/** A zone of a device, as the setup sees it. */
export interface SetupZone {
  deviceId: string;
  zoneIndex: number;
  deviceName: string;
  name: string;
  kind: DeviceKind;
  ledCount: number;
  resizable: boolean;
  minLeds: number;
  maxLeds: number;
  /** The placements of this zone in the layout, in the layout's order. */
  placements: number[];
  /** The zone is worth walking: its count can change, or it is a long line of LEDs. */
  doubtful: boolean;
  /** Every part of this zone is confirmed. */
  mapped: boolean;
}

/** One part of a zone: a run of the zone's LEDs. */
export interface Segment {
  /** The first LED of the zone this run starts at. */
  start: number;
  /** How many LEDs it owns. */
  count: number;
  /** The placement that owns it, in the layout. */
  placement: number | null;
}

/**
 * The zone is worth walking when its count can change, or when it is a long line of LEDs: one
 * fan's worth is sixteen, so anything longer than that is more than one part.
 */
export const FAN_LEDS = 16;

export function isDoubtful(zone: ZoneInfo): boolean {
  if (zone.resizable) return true;
  return zone.kind.type === "linear" && zone.led_count > FAN_LEDS;
}

/** Every zone of every device, with the layout's state. */
export function setupZones(layout: Layout, devices: DeviceInfo[] | null): SetupZone[] {
  const out: SetupZone[] = [];
  for (const device of devices ?? []) {
    for (const zone of device.zones) {
      const placements = layout.zones
        .map((z, index) => ({ z, index }))
        .filter(({ z }) => z.device_id === device.id && z.zone_index === zone.index)
        .map(({ index }) => index);
      out.push({
        deviceId: device.id,
        zoneIndex: zone.index,
        deviceName: device.name,
        name: zone.name,
        kind: device.kind,
        ledCount: zone.led_count,
        resizable: zone.resizable,
        minLeds: zone.min_leds,
        maxLeds: zone.max_leds,
        placements,
        doubtful: isDoubtful(zone),
        mapped: placements.length > 0 && placements.every((i) => layout.zones[i]?.part?.mapped === true),
      });
    }
  }
  return out;
}

/** The runs of a zone's LEDs the layout holds. */
export function segmentsOf(zone: SetupZone, layout: Layout): Segment[] {
  const withRun = zone.placements
    .map((i) => ({ i, z: layout.zones[i] }))
    .filter((p): p is { i: number; z: PlacedZone } => !!p.z && !!p.z.leds)
    .sort((a, b) => (a.z.leds?.start ?? 0) - (b.z.leds?.start ?? 0));
  if (withRun.length > 0) {
    return withRun.map(({ i, z }) => ({ start: z.leds!.start, count: z.leds!.count, placement: i }));
  }
  // One part owns the whole zone.
  const whole = zone.placements.find((i) => !layout.zones[i]?.leds);
  if (whole !== undefined) {
    return [{ start: 0, count: zone.ledCount, placement: whole }];
  }
  return [];
}

/** The run the given LED is in. */
export function segmentAt(segments: Segment[], led: number): Segment | null {
  for (const s of segments) {
    if (led >= s.start && led < s.start + s.count) return s;
  }
  return segments.length ? (segments[segments.length - 1] ?? null) : null;
}

/**
 * The runs after a split at the given LED: the run the LED is in becomes two, the second
 * starting at that LED. `null` when the LED is already the start of a run: there is nothing to
 * split there.
 */
export function splitAt(segments: Segment[], led: number): Segment[] | null {
  const at = segmentAt(segments, led);
  if (!at) return null;
  if (at.start === led) return null;
  const out: Segment[] = [];
  for (const s of segments) {
    const end = s.start + s.count;
    if (led > s.start && led < end) {
      out.push({ start: s.start, count: led - s.start, placement: s.placement });
      out.push({ start: led, count: end - led, placement: null });
    } else {
      out.push(s);
    }
  }
  return out;
}

/** The parts with lights, and how many of them are confirmed. */
export function progress(layout: Layout, devices: DeviceInfo[] | null): { mapped: number; total: number } {
  let mapped = 0;
  let total = 0;
  for (const z of layout.zones) {
    const device = devices?.find((d) => d.id === z.device_id);
    const zone = device?.zones.find((x) => x.index === z.zone_index);
    if (!zone || zone.led_count <= 0) continue;
    total++;
    if (z.part?.mapped === true) mapped++;
  }
  return { mapped, total };
}

/** One part with lights, for the progress dots. */
export interface PartDot {
  /** The part's index in the layout. */
  index: number;
  mapped: boolean;
}

/** The parts with lights, in the layout's order: one dot each. */
export function progressParts(layout: Layout, devices: DeviceInfo[] | null): PartDot[] {
  const out: PartDot[] = [];
  layout.zones.forEach((z, index) => {
    const device = devices?.find((d) => d.id === z.device_id);
    const zone = device?.zones.find((x) => x.index === z.zone_index);
    if (!zone || zone.led_count <= 0) return;
    out.push({ index, mapped: z.part?.mapped === true });
  });
  return out;
}

/**
 * The kinds a part may be, in the order the setup offers them: the things that really sit on
 * an ARGB header, and a plain box for the rest.
 */
export const SETUP_KINDS: PartKind[] = ["fan", "strip", "pump", "radiator", "generic"];

/** The name of a kind, in the setup's words. */
export const SETUP_KIND_LABEL: Record<string, string> = {
  fan: "Fan",
  strip: "Strip",
  pump: "Pump",
  radiator: "Radiator",
  generic: "Box",
};

/** The device kinds whose parts are fixed: one click, no walk. */
export function isFixedKind(kind: DeviceKind): boolean {
  return kind === "keyboard" || kind === "mouse" || kind === "dram" || kind === "headset" || kind === "microphone";
}

/** The shape a part of that kind takes: a fan's LEDs are a ring, the rest a line. */
export function shapeForKind(kind: PartKind): ZoneShape {
  return kind === "fan" ? { type: "ring" } : { type: "strip" };
}

/**
 * Where a new part goes: the next free place of its kind in the case, so a fan lands in a fan's
 * place and a stick of RAM in a RAM slot. With no case, it goes beside the part it came from,
 * and the owner drags it where it belongs.
 */
export function placeForPart(
  layout: Layout,
  kind: PartKind,
  template: PlacedZone,
): { x: number; y: number; width: number; height: number } {
  const caseBox = layout.landmarks?.find((l) => l.kind === "case" && l.visible !== false);
  if (caseBox) {
    const slots = slotsForBox({
      x: caseBox.x,
      y: caseBox.y,
      width: caseBox.width,
      height: caseBox.height,
    });
    const want =
      kind === "fan" ? "fan" : kind === "ram" ? "ram" : kind === "gpu" ? "gpu" : kind === "strip" ? "edge" : null;
    if (want) {
      const taken = (s: { x: number; y: number; w: number; h: number }) =>
        layout.zones.some(
          (z) => z.x < s.x + s.w && s.x < z.x + z.width && z.y < s.y + s.h && s.y < z.y + z.height,
        );
      const slot = slots.find((s) => s.kind === want && !taken(s));
      if (slot) return { x: slot.x, y: slot.y, width: slot.w, height: slot.h };
    }
  }
  // Beside the part it came from.
  const width = kind === "fan" ? 30 : template.width;
  const height = kind === "fan" ? 30 : template.height;
  return {
    x: Math.max(0, Math.min(320 - width, template.x + template.width + 3)),
    y: Math.max(0, Math.min(200 - height, template.y)),
    width,
    height,
  };
}
