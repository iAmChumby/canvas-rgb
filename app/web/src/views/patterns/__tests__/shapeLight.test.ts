import { describe, expect, it } from "vitest";
import { referenceLight, shapesOf } from "@/views/patterns/shapeLight";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { LightView } from "@/proto/LightView";
import type { ZoneInfo } from "@/proto/ZoneInfo";

const view = (id: string, leds: number, arrangement: "strip" | "ring" | "grid", connected = true): LightView => ({
  light: { id: `${id}#z0`, device_id: id, zone_index: 0, arrangement, reversed: false } as LightView["light"],
  led_count: leds,
  connected,
  label: id,
});
const device = (id: string, kind: ZoneInfo["kind"], leds: number): DeviceInfo =>
  ({ id, name: id, zones: [{ index: 0, name: "z", kind, led_count: leds, min_leds: leds, max_leds: leds, resizable: false }], connected: true }) as unknown as DeviceInfo;

describe("the light a pattern's shape picker writes to", () => {
  it("is a connected light with the pattern's LED count, preferring its own shape", () => {
    const lights = [view("off", 8, "ring", false), view("strip", 8, "strip"), view("ring", 8, "ring"), view("other", 12, "ring")];
    expect(referenceLight({ led_count: 8, arrangement: "ring" }, lights)?.light.id).toBe("ring#z0");
    expect(referenceLight({ led_count: 8, arrangement: "strip" }, lights)?.light.id).toBe("strip#z0");
    expect(referenceLight({ led_count: 99, arrangement: "ring" }, lights)).toBeNull();
    expect(referenceLight({ led_count: 8, arrangement: "ring" }, null)).toBeNull();
  });

  it("is a matrix zone of the same cells for a grid pattern, never a line that happens to be 8 LEDs", () => {
    const lights = [view("line", 8, "strip"), view("pad", 8, "grid")];
    const devices = [
      device("line", { type: "linear" }, 8),
      device("pad", { type: "matrix", width: 4, height: 2, map: [] }, 8),
    ];
    const grid = { led_count: 8, arrangement: "grid" as const, grid: { width: 4, height: 2 } };
    expect(referenceLight(grid, lights, devices)?.light.id).toBe("pad#z0");
    expect(referenceLight(grid, [lights[0]!], devices)).toBeNull();
    expect(referenceLight({ ...grid, grid: { width: 2, height: 4 } }, lights, devices)).toBeNull();
  });
});

describe("the shapes the header offers", () => {
  it("are the zone's, with the pattern's own shape always among them", () => {
    expect(shapesOf({ led_count: 8, arrangement: "strip" }, undefined)).toEqual(["strip", "ring"]);
    expect(shapesOf({ led_count: 8, arrangement: "grid", grid: { width: 4, height: 2 } }, undefined)).toEqual(["strip", "ring", "grid"]);
    const linear = device("a", { type: "linear" }, 8).zones[0];
    // A grid pattern on a line zone: the picker still shows the grid it is in.
    expect(shapesOf({ led_count: 8, arrangement: "grid" }, linear)).toEqual(["strip", "ring", "grid"]);
    expect(shapesOf({ led_count: 1, arrangement: "single" }, undefined)).toEqual([]);
  });
});
