// Issue #187, rules 3 + 4: rubber-band past drag limits, and no easing while held.
//
// The formula is iOS's resistance curve: past a limit, an overshoot `o` in a span of
// dimension `d` with constant `c` (0.55) moves only
//   resisted = (o * c * d) / (d + c * o)
// so small overshoots track the pointer (resisted ≈ c*o) and large ones level off
// toward `d`. On release the part settles back to the limit itself (clamp).
import { describe, expect, it } from "vitest";
import { rubberBand, settleToLimit } from "@/lib/rubberBand";

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
