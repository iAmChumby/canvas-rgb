import { describe, expect, it } from "vitest";
import { layoutRun } from "@/views/scenes/LedRun";
import type { LightView } from "@/proto/LightView";

// `layoutRun` places a group's lights across the run's canvas. Whatever the group holds, nothing
// may be placed past the canvas's edge: a light drawn off the end is a light the owner cannot see.

function light(id: string, leds: number, arrangement: "strip" | "ring" | "single" = "strip"): LightView {
  return {
    light: { id, device_id: id, zone_index: 0, arrangement, reversed: false },
    led_count: leds,
    connected: true,
    label: id,
  };
}

const sixteen: LightView[] = [
  light("a", 8),
  light("b", 8),
  light("c", 4),
  ...Array.from({ length: 10 }, (_, i) => light(`s${i}`, 60)),
  light("r1", 24, "ring"),
  light("r2", 24, "ring"),
  light("r3", 16, "ring"),
].slice(0, 16);

const WIDTHS = [185, 382, 640, 862];

describe("layoutRun", () => {
  it.each(WIDTHS)("keeps every light inside a %i px canvas", (width) => {
    const { slots } = layoutRun(sixteen, width, 52);
    const last = slots[slots.length - 1];
    expect(last).toBeDefined();
    expect((last?.x ?? 0) + (last?.w ?? 0)).toBeLessThanOrEqual(width);
    for (const s of slots) expect(s.x).toBeGreaterThanOrEqual(0);
  });

  it("keeps eight strips and three rings inside the narrowest canvas", () => {
    const mixed = [
      ...Array.from({ length: 8 }, (_, i) => light(`s${i}`, 57)),
      light("r1", 32, "ring"),
      light("r2", 32, "ring"),
      light("r3", 16, "ring"),
    ];
    const { slots } = layoutRun(mixed, 185, 52);
    const last = slots[slots.length - 1];
    expect((last?.x ?? 0) + (last?.w ?? 0)).toBeLessThanOrEqual(185);
  });

  it("leaves a gap between neighbouring lights", () => {
    const { slots } = layoutRun([light("a", 8), light("b", 8)], 400, 52);
    const [a, b] = slots;
    expect((b?.x ?? 0) - ((a?.x ?? 0) + (a?.w ?? 0))).toBeGreaterThan(0);
  });

  it("gives a lone short strip dots to look at, not a bar", () => {
    const { unit } = layoutRun([light("a", 8)], 382, 60);
    expect(unit).toBeGreaterThanOrEqual(7);
  });

  it("keeps a ring readable beside strips, even in the narrowest canvas", () => {
    const mixed = [light("a", 60), light("b", 60), light("r", 24, "ring")];
    const { slots } = layoutRun(mixed, 185, 52);
    const ring = slots.find((x) => x.ring);
    expect(ring?.w ?? 0).toBeGreaterThanOrEqual(18);
  });

  it("keeps 200 lights inside 185 px: the last edge is at or before the width", () => {
    const many = Array.from({ length: 200 }, (_, i) => light(`l${i}`, 8));
    const { slots } = layoutRun(many, 185, 60);
    expect(slots).toHaveLength(200);
    const last = slots[199];
    expect((last?.x ?? 0) + (last?.w ?? 0)).toBeLessThanOrEqual(185 + 1e-6);
    for (const s of slots) expect(s.x).toBeGreaterThanOrEqual(0);
  });

  it("keeps 16 lights inside a canvas of 0 px: nothing starts past the edge", () => {
    const { slots } = layoutRun(sixteen, 0, 60);
    for (const s of slots) {
      expect(s.x).toBeLessThanOrEqual(1e-6);
      expect(s.x + s.w).toBeLessThanOrEqual(1e-6);
    }
  });

  it("never places a light past the edge, for any count, width and mix", () => {
    const kinds: Array<"strip" | "ring" | "single"> = ["strip", "ring", "single"];
    const widths = [0, 1, 5, 11, 12, 13, 40, 120, 185, 382, 640, 862, 1400];
    const counts = [0, 1, 2, 3, 7, 16, 50, 200];
    const sizes = [1, 3, 8, 24, 57, 600];
    let checked = 0;
    for (const width of widths) {
      for (const count of counts) {
        for (const shift of [0, 1, 2]) {
          const lights = Array.from({ length: count }, (_, i) =>
            light(`l${i}`, sizes[(i + shift) % sizes.length] ?? 8, kinds[(i * 2 + shift) % kinds.length] ?? "strip"),
          );
          for (const height of [30, 52, 60]) {
            const { slots } = layoutRun(lights, width, height);
            checked += 1;
            for (const s of slots) {
              expect(s.x, `x of a slot, ${count} lights at ${width}`).toBeGreaterThanOrEqual(-1e-6);
              expect(s.w).toBeGreaterThanOrEqual(0);
              expect(s.x + s.w, `edge of a slot, ${count} lights at ${width} px`).toBeLessThanOrEqual(width + 1e-6);
            }
          }
        }
      }
    }
    expect(checked).toBe(13 * 8 * 3 * 3);
  });
});
