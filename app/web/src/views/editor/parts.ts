// What a part is, in words and in numbers.
//
// A part is a physical object: a fan, a stick of RAM, a radiator, the case itself. The editor
// draws each kind with its own crude glyph (see `ZoneArt.tsx`), and the `facing` says which way
// it points, which is what decides the drawing: a fan that points at you shows its ring, one
// that points up or down is seen from the side and is a thin bar.
//
// The sizes here are the sizes the hardware really has, in canvas units at 4 mm to the unit: a
// 120 mm fan is 30 units, a 136 mm stick of RAM is 34 units tall. They are what a part starts
// out as, not a list of presets: the owner drags and resizes every one of them.

import type { DeviceKind } from "@/proto/DeviceKind";
import type { Facing } from "@/proto/Facing";
import type { PartKind } from "@/proto/PartKind";
import type { PlacedZone } from "@/proto/PlacedZone";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import { zoneKey } from "@/lib/zones";

/** The plain name of each kind of part. */
export const PART_LABEL: Record<PartKind, string> = {
  fan: "Fan",
  radiator: "Radiator",
  pump: "Pump",
  ram: "RAM",
  gpu: "Graphics card",
  cpu: "CPU",
  strip: "Strip",
  keyboard: "Keyboard",
  mouse: "Mouse",
  microphone: "Microphone",
  headset: "Headset",
  case: "Case",
  generic: "Box",
};

/** The kinds that can be added by hand: the parts that have no lights of their own. */
export const LANDMARK_KINDS: PartKind[] = ["cpu", "gpu", "generic", "case"];

/** What is picked on the canvas: a zone (a part with lights) or a landmark (one without). */
export interface Sel {
  kind: "zone" | "landmark";
  index: number;
}

/** The key of a picked part, for the editor's own maps. */
export function selKey(s: Sel): string {
  return `${s.kind}:${s.index}`;
}

/**
 * The key a part is locked under. A split zone has one key per run of LEDs, so two fans on one
 * header are locked apart.
 */
export function zoneLockKey(z: PlacedZone): string {
  return `zone:${zoneKey(z)}`;
}

/** The key a landmark is locked under. */
export function landmarkLockKey(index: number): string {
  return `landmark:${index}`;
}

/** What each part starts out as, in canvas units (4 mm to the unit). */
export const PART_SIZE: Record<PartKind, { width: number; height: number }> = {
  fan: { width: 30, height: 30 }, // 120 mm
  radiator: { width: 90, height: 10 }, // 360 x 40 mm
  pump: { width: 20, height: 20 }, // 80 mm across
  ram: { width: 6, height: 34 }, // 24 x 136 mm
  gpu: { width: 70, height: 14 }, // 280 x 56 mm
  cpu: { width: 16, height: 16 }, // 64 mm socket
  strip: { width: 120, height: 5 }, // 480 mm of strip
  keyboard: { width: 110, height: 30 }, // 440 x 120 mm
  mouse: { width: 22, height: 32 },
  microphone: { width: 16, height: 28 },
  headset: { width: 30, height: 30 },
  case: { width: 120, height: 122 }, // 48 x 49 cm
  generic: { width: 24, height: 24 },
};

/** The ways a part can point, in the order the picker lists them. */
export const FACINGS: Facing[] = ["viewer", "away", "up", "down", "left", "right"];

/** The plain name of a way of pointing. */
export const FACING_LABEL: Record<Facing, string> = {
  viewer: "At you",
  away: "Away",
  up: "Up",
  down: "Down",
  left: "Left",
  right: "Right",
};

/**
 * True when the part is seen from the side: a fan pointing up or down is a bar, not a ring.
 * Left and right are seen from the side too, but the bar stands up.
 */
export function isEdgeOn(facing: Facing): boolean {
  return facing === "up" || facing === "down" || facing === "left" || facing === "right";
}

/** True when the part is seen edge-on as a bar lying flat. */
export function isFlat(facing: Facing): boolean {
  return facing === "up" || facing === "down";
}

/**
 * The part a zone is, when the owner has not said: the device's kind, and a fan for anything
 * whose zone is called a fan. A layout saved before parts existed loads with no part at all,
 * and this is what it is drawn as.
 */
export function partForDevice(kind: DeviceKind, info: ZoneInfo): PartKind {
  switch (kind) {
    case "dram":
      return "ram";
    case "led_strip":
      return "strip";
    case "gpu":
      return "gpu";
    case "keyboard":
      return "keyboard";
    case "mouse":
      return "mouse";
    case "microphone":
      return "microphone";
    case "headset":
      return "headset";
    default:
      break;
  }
  if (info.name.toLowerCase().includes("fan") && info.led_count > 1) return "fan";
  if (kind === "motherboard") return "generic";
  return "generic";
}

/** The way a part points when nobody has said: at the reader, through the glass. */
export function defaultFacing(kind: PartKind): Facing {
  return kind === "fan" ? "viewer" : "viewer";
}

/**
 * Where a case stands in the onboarding: its bottom edge on the floor line and its middle on
 * this x. The default box, the reset and the guides all share it, so they nest.
 */
export const CASE_ANCHOR = { x: 160, y: 149 } as const;

/**
 * Typical towers, drawn faintly behind the case on the Case step to hold it against. They are
 * guides, never a picker: the default action is always "Looks right". Sizes in centimetres.
 */
export const CASE_GUIDES: { id: string; name: string; cmWide: number; cmTall: number }[] = [
  { id: "small", name: "Small tower", cmWide: 37, cmTall: 34 },
  { id: "mid", name: "Mid tower", cmWide: 48, cmTall: 49 },
  { id: "full", name: "Full tower", cmWide: 58, cmTall: 58 },
];
