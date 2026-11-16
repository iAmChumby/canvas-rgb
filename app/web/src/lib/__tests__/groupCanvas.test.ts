// The builder stage's frames: a member's frame is the slot it stands in, not the union of
// the small cells its LEDs read (#372).
//
// A strip's LEDs read small square cells at mid-height, so the union of those cells leaves
// the ends of the slot empty. Eight LEDs in a 320-unit slot lost 33 units of frame at the
// two ends, and the frame the owner draws, picks and drags is the strip's.

import { describe, expect, it } from "vitest";
import { buildGroupCanvas, wholeLightsCanvas } from "@/lib/groupCanvas";
import type { Arrangement } from "@/proto/Arrangement";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { GroupMode } from "@/proto/GroupMode";
import type { Light } from "@/proto/Light";
import type { LightView } from "@/proto/LightView";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { ZoneKind } from "@/proto/ZoneKind";

function zone(index: number, kind: ZoneKind, ledCount: number): ZoneInfo {
  return {
    index,
    name: `zone${index}`,
    kind,
    led_count: ledCount,
    min_leds: ledCount,
    max_leds: ledCount,
    resizable: false,
  };
}

function device(id: string, zones: ZoneInfo[]): DeviceInfo {
  return {
    id,
    name: id,
    vendor: "test",
    kind: "other",
    adapter: "fake",
    adapter_index: 0,
    serial: "",
    location: "",
    zones,
    connected: true,
  };
}

function lightView(id: string, deviceId: string, arrangement: Arrangement, ledCount: number): LightView {
  const light: Light = {
    id,
    device_id: deviceId,
    zone_index: 0,
    arrangement,
    reversed: false,
  };
  return { light, led_count: ledCount, connected: true, label: id };
}

/** A strip of `leds` LEDs on a device of its own. */
function strip(id: string, leds: number): LightView {
  return lightView(id, `dev-${id}`, "strip", leds);
}

function group(members: string[], mode: GroupMode) {
  return { id: "g", name: "Group", members, mode };
}

/** The first light of a canvas, or a thrown error: the tests know their lists are not empty. */
function firstLight(canvas: ReturnType<typeof buildGroupCanvas>) {
  const light = canvas.lights[0];
  if (light === undefined) throw new Error("the canvas has no light");
  return light;
}

describe("the builder's frames (#372)", () => {
  it("gives a strip the whole slot, not the union of its cells", () => {
    const devices = [device("dev-a", [zone(0, { type: "linear" }, 8)])];
    const lights = [strip("a", 8)];
    const canvas = buildGroupCanvas(group(["a"], "each_its_own"), lights, devices);
    const box = firstLight(canvas).box;
    expect(box.x).toBe(0);
    expect(box.width).toBe(320);
    expect(box.y).toBe(0);
    expect(box.height).toBe(200);
  });

  it("gives each strip its own share in flow across", () => {
    const devices = [device("dev-a", [zone(0, { type: "linear" }, 8)]), device("dev-b", [zone(0, { type: "linear" }, 8)])];
    const lights = [strip("a", 8), strip("b", 8)];
    const canvas = buildGroupCanvas(group(["a", "b"], "flow_across"), lights, devices);
    expect(canvas.lights.map((l) => l.box.x)).toEqual([0, 160]);
    expect(canvas.lights.map((l) => l.box.width)).toEqual([160, 160]);
  });

  it("keeps a ring's frame where its cells are", () => {
    // A ring is not a strip: its frame is the union of its cells, unchanged by #372.
    const devices = [device("dev-a", [zone(0, { type: "linear" }, 8)])];
    const lights = [lightView("a", "dev-a", "ring", 8)];
    const canvas = buildGroupCanvas(group(["a"], "each_its_own"), lights, devices);
    const box = firstLight(canvas).box;
    expect(box.width).toBeLessThan(320);
    expect(box.height).toBeLessThan(200);
  });

  it("gives every light of the whole-lights canvas its share", () => {
    const devices = [device("dev-a", [zone(0, { type: "linear" }, 8)]), device("dev-b", [zone(0, { type: "linear" }, 8)])];
    const lights = [strip("a", 8), strip("b", 8)];
    const canvas = wholeLightsCanvas(lights, devices);
    expect(canvas.lights.map((l) => l.box.width)).toEqual([160, 160]);
    expect(canvas.lights.map((l) => l.box.x)).toEqual([0, 160]);
  });
});
