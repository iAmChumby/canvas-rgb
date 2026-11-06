// Issue #187, rules 3 + 4: rubber-band past drag limits, and no easing while held.
//
// The formula is iOS's resistance curve: past a limit, an overshoot `o` in a span of
// dimension `d` with constant `c` (0.55) moves only
//   resisted = (o * c * d) / (d + c * o)
// so small overshoots track the pointer (resisted ≈ c*o) and large ones level off
// toward `d`. On release the part settles back to the limit itself (clamp).
import { describe, expect, it } from "vitest";
import { rubberBand, settleRect, settleToLimit } from "@/lib/rubberBand";
import { resizeBy } from "@/views/PreviewStage";

describe("rubberBand (#187 rule 3)", () => {
  it("inside the limits the position equals the pointer exactly (rule 4: no easing while held)", () => {
    expect(rubberBand(150, 0, 320)).toBe(150);
    expect(rubberBand(0, 0, 320)).toBe(0);
    expect(rubberBand(320, 0, 320)).toBe(320);
  });

  it("past the limit the position is past the limit but less than the pointer's overshoot", () => {
    // Pointer wants 420 in a 0..320 span: 100 past the limit.
    const p = rubberBand(420, 0, 320);
    expect(p).toBeGreaterThan(320);
    expect(p).toBeLessThan(420);
  });

  it("past the lower limit it is below the limit but less than the pointer's overshoot", () => {
    const p = rubberBand(-80, 0, 320);
    expect(p).toBeLessThan(0);
    expect(p).toBeGreaterThan(-80);
  });

  it("resistance diminishes: double the overshoot is less than double the resisted move", () => {
    const a = rubberBand(320 + 50, 0, 320) - 320;
    const b = rubberBand(320 + 100, 0, 320) - 320;
    expect(b).toBeLessThan(2 * a);
    expect(b).toBeGreaterThan(a);
  });

  it("on release it settles back to the limit", () => {
    const held = rubberBand(420, 0, 320);
    expect(settleToLimit(held, 0, 320)).toBe(320);
    expect(settleToLimit(rubberBand(-80, 0, 320), 0, 320)).toBe(0);
    expect(settleToLimit(150, 0, 320)).toBe(150);
  });
});

describe("settleRect (#187, the release in the canvas)", () => {
  function inCanvas(r: { x: number; y: number; width: number; height: number }): boolean {
    return r.x >= 0 && r.y >= 0 && r.x + r.width <= 320 && r.y + r.height <= 200;
  }

  it("a negative or oversized size settles to a rect fully inside the canvas", () => {
    const r = settleRect({ x: -100, y: 500, width: 1000, height: -10 }, 320, 200, 2);
    expect(r.width).toBe(320);
    expect(r.height).toBe(2);
    expect(r.x).toBe(0);
    expect(r.y).toBe(198);
    expect(inCanvas(r)).toBe(true);
  });

  it("position is settled against the settled size, never the pre-settle size", () => {
    // Held at x=420 with w=2 on a 320 canvas: the old order (position first,
    // against w=2) saved x=420 with w=2 — fully outside the canvas.
    const r = settleRect({ x: 420, y: 50, width: 2, height: 20 }, 320, 200, 2);
    expect(r.x).toBe(318);
    expect(inCanvas(r)).toBe(true);
    const r2 = settleRect({ x: 290, y: 190, width: 1000, height: 1000 }, 320, 200, 2);
    expect(inCanvas(r2)).toBe(true);
  });
});

describe("resizeBy with Shift (#187, the ratio-locked pair)", () => {
  const CANVAS_W = 320;
  const CANVAS_H = 200;
  const MIN = 2;

  it("a shift-resize past the canvas keeps both dimensions within [MIN, CANVAS] while held", () => {
    const o = { x: 100, y: 75, width: 100, height: 50, rotation_deg: 0 };
    const next = resizeBy(o, { id: "se", sx: 1, sy: 1 }, { x: 4000, y: 4000 }, true, false);
    expect(next.width).toBeGreaterThanOrEqual(MIN);
    expect(next.width).toBeLessThanOrEqual(CANVAS_W);
    expect(next.height).toBeGreaterThanOrEqual(MIN);
    expect(next.height).toBeLessThanOrEqual(CANVAS_H);
    expect(next.width / next.height).toBeCloseTo(2, 6);
  });

  it("a shift edge-resize past the canvas keeps both dimensions within [MIN, CANVAS] while held", () => {
    const o = { x: 100, y: 75, width: 100, height: 50, rotation_deg: 0 };
    const next = resizeBy(o, { id: "e", sx: 1, sy: 0 }, { x: 4000, y: 75 }, true, false);
    expect(next.width).toBeGreaterThanOrEqual(MIN);
    expect(next.width).toBeLessThanOrEqual(CANVAS_W);
    expect(next.height).toBeGreaterThanOrEqual(MIN);
    expect(next.height).toBeLessThanOrEqual(CANVAS_H);
  });

  it("an extreme shift-resize settles to a rect fully inside the canvas", () => {
    const o = { x: 100, y: 75, width: 100, height: 50, rotation_deg: 0 };
    const next = resizeBy(o, { id: "se", sx: 1, sy: 1 }, { x: -4000, y: -4000 }, true, false);
    const settled = settleRect(next, CANVAS_W, CANVAS_H, MIN);
    expect(settled.x).toBeGreaterThanOrEqual(0);
    expect(settled.x + settled.width).toBeLessThanOrEqual(CANVAS_W);
    expect(settled.y).toBeGreaterThanOrEqual(0);
    expect(settled.y + settled.height).toBeLessThanOrEqual(CANVAS_H);
  });
});
