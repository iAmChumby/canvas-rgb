import { describe, expect, it } from "vitest";
import { dotAt, dotsBetween, layoutDots, type Dot } from "@/views/patterns/dotLayout";
import type { Arrangement } from "@/proto/Arrangement";
import type { BuiltOn } from "@/proto/BuiltOn";

// The dots of the pattern editor: where each LED is drawn in the device's box, which dot a press
// lands on, and which dots a drag crossed. Pure, so every rule is settled without a canvas.

function builtOn(
  led_count: number,
  arrangement: Arrangement,
  grid?: { width: number; height: number },
): BuiltOn {
  return { led_count, arrangement, ...(grid ? { grid } : {}) };
}

/** The dots of one row, in drawing order. Rows are horizontal, so a row is one y. */
function row(dots: Dot[], at: number): Dot[] {
  return dots.filter((dot) => dot.y === at);
}

describe("the dots of a device", () => {
  it("ring dots start at the top and go clockwise", () => {
    const dots = layoutDots(builtOn(12, "ring"), 400, 400);
    expect(dots).toHaveLength(12);
    expect(dots.map((dot) => dot.index)).toEqual([...Array(12).keys()]);

    const top = dots[0]!;
    // 0 is the top of the circle, on the centre line between left and right.
    expect(top.x).toBeCloseTo(200, 5);
    expect(top.y).toBeLessThan(200);
    // Clockwise: a quarter of the way round is on the right, half is the bottom, three quarters
    // the left. The y grows downward, as the canvas path's ring does.
    expect(dots[3]!.x).toBeGreaterThan(200);
    expect(dots[3]!.y).toBeCloseTo(200, 5);
    expect(dots[6]!.y).toBeGreaterThan(200);
    expect(dots[6]!.x).toBeCloseTo(200, 5);
    expect(dots[9]!.x).toBeLessThan(200);
    expect(dots[9]!.y).toBeCloseTo(200, 5);
    expect(dots[1]!.y).toBeGreaterThan(top.y);

    // Every dot sits on one circle, and that circle stays inside the box.
    const radius = 200 - top.y;
    for (const dot of dots) {
      const d = Math.hypot(dot.x - 200, dot.y - 200);
      expect(d).toBeCloseTo(radius, 5);
      expect(dot.x - dot.r).toBeGreaterThanOrEqual(0);
      expect(dot.x + dot.r).toBeLessThanOrEqual(400);
      expect(dot.y - dot.r).toBeGreaterThanOrEqual(0);
      expect(dot.y + dot.r).toBeLessThanOrEqual(400);
    }
    expect(radius).toBeGreaterThan(150);
  });

  it("a strip longer than the width wraps into rows and keeps every dot reachable", () => {
    const width = 300;
    const height = 260;
    const dots = layoutDots(builtOn(512, "strip"), width, height);

    // Every LED is drawn once, in order.
    expect(dots).toHaveLength(512);
    expect(dots.map((dot) => dot.index)).toEqual([...Array(512).keys()]);

    // It wraps: more than one row, each row read left to right, rows top to bottom.
    const ys = [...new Set(dots.map((dot) => dot.y))];
    expect(ys.length).toBeGreaterThan(1);
    expect([...ys].sort((a, b) => a - b)).toEqual(ys);
    let seen = -1;
    for (const y of ys) {
      const line = row(dots, y);
      expect(line.length).toBeGreaterThan(0);
      const xs = line.map((dot) => dot.x);
      expect([...xs].sort((a, b) => a - b)).toEqual(xs);
      expect(line[0]!.index).toBe(seen + 1);
      seen = line[line.length - 1]!.index;
    }
    expect(seen).toBe(511);

    // Every dot is inside the box, and its radius has the 3 px floor.
    for (const dot of dots) {
      expect(dot.r).toBeGreaterThanOrEqual(3);
      expect(dot.x - dot.r).toBeGreaterThanOrEqual(0);
      expect(dot.x + dot.r).toBeLessThanOrEqual(width);
      expect(dot.y - dot.r).toBeGreaterThanOrEqual(0);
      expect(dot.y + dot.r).toBeLessThanOrEqual(height);
    }

    // Reachable: a press on a dot's own centre finds that dot, every time.
    for (const dot of dots) expect(dotAt(dots, dot.x, dot.y)).toBe(dot.index);
  });

  it("dotAt finds the dot under the pointer and null in a gap", () => {
    const dots = layoutDots(builtOn(12, "ring"), 400, 400);
    const dot = dots[5]!;
    expect(dotAt(dots, dot.x, dot.y)).toBe(5);
    // A pixel off its centre is still its dot.
    expect(dotAt(dots, dot.x + 1, dot.y + 1)).toBe(5);
    // The hole in the middle of the ring is a gap: no dot is near it.
    expect(dotAt(dots, 200, 200)).toBeNull();
    // And the corner of the box, far outside the ring.
    expect(dotAt(dots, 0, 0)).toBeNull();
  });

  it("dotsBetween skips no dot on a fast drag across a 512-LED strip", () => {    const dots = layoutDots(builtOn(512, "strip"), 900, 400);
    const first = row(dots, dots[0]!.y);
    const from = first[0]!;
    const to = first[first.length - 1]!;

    // One jump from the start of a row to its end takes every dot between, in order.
    const hit = dotsBetween(dots, { x: from.x, y: from.y }, { x: to.x, y: to.y });
    expect(hit[0]).toBe(from.index);
    expect(hit[hit.length - 1]).toBe(to.index);
    for (let i = 1; i < hit.length; i++) expect(hit[i]).toBe(hit[i - 1]! + 1);

    // A press is one dot; a drag backwards takes the same dots.
    expect(dotsBetween(dots, { x: from.x, y: from.y }, { x: from.x, y: from.y })).toEqual([0]);
    const back = dotsBetween(dots, { x: to.x, y: to.y }, { x: from.x, y: from.y });
    expect(back).toEqual(hit);

    // A segment nowhere near the device takes nothing.
    expect(dotsBetween(dots, { x: 0, y: 0 }, { x: 900, y: 0 })).toEqual([]);
  });

  it("every dot stays in the box on the densest devices", () => {
    // The sizes the editor is looked at on, and the ones the spine's cap allows: a strip of 300 and
    // of 4096 in the well a phone gives, a ring of 4096, and a 5 by 4 grid.
    const cases: { builtOn: BuiltOn; width: number; height: number }[] = [
      { builtOn: builtOn(30, "strip"), width: 390, height: 500 },
      { builtOn: builtOn(300, "strip"), width: 342, height: 400 },
      { builtOn: builtOn(4096, "strip"), width: 342, height: 400 },
      { builtOn: builtOn(4096, "ring"), width: 342, height: 400 },
      { builtOn: builtOn(20, "grid", { width: 5, height: 4 }), width: 600, height: 800 },
    ];
    for (const one of cases) {
      const dots = layoutDots(one.builtOn, one.width, one.height);
      expect(dots).toHaveLength(one.builtOn.led_count);
      for (const dot of dots) {
        expect(dot.r).toBeGreaterThan(0);
        expect(dot.x - dot.r).toBeGreaterThanOrEqual(0);
        expect(dot.x + dot.r).toBeLessThanOrEqual(one.width);
        expect(dot.y - dot.r).toBeGreaterThanOrEqual(0);
        expect(dot.y + dot.r).toBeLessThanOrEqual(one.height);
        expect(dotAt(dots, dot.x, dot.y)).toBe(dot.index);
      }
    }
  });
});
