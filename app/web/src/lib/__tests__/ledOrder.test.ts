import { describe, expect, it } from "vitest";
import {
  clientToView,
  directionPatch,
  effectiveFirstLed,
  identifyLed,
  nearestLed,
  notchDot,
  stepFirstLed,
} from "@/lib/ledOrder";
import type { Dot } from "@/views/patterns/dotLayout";

const dots: Dot[] = [0, 1, 2, 3].map((index) => ({ index, x: index * 10, y: 0, r: 3 }));

describe("the first LED's keys", () => {
  it("moves one LED with each arrow, wrapping round the run", () => {
    expect(stepFirstLed(0, "ArrowRight", 8)).toBe(1);
    expect(stepFirstLed(0, "ArrowDown", 8)).toBe(1);
    expect(stepFirstLed(3, "ArrowLeft", 8)).toBe(2);
    expect(stepFirstLed(0, "ArrowUp", 8)).toBe(7);
    expect(stepFirstLed(7, "ArrowRight", 8)).toBe(0);
  });

  it("Home resets to the first LED and other keys do nothing", () => {
    expect(stepFirstLed(7, "Home", 8)).toBe(0);
    expect(stepFirstLed(3, "Escape", 8)).toBeNull();
    expect(stepFirstLed(3, "ArrowRight", 0)).toBeNull();
  });
});

describe("the notch's geometry", () => {
  it("sits on the dot the owner marked, wrapping an out-of-range value", () => {
    expect(notchDot(dots, 2)?.index).toBe(2);
    expect(notchDot(dots, 4)?.index).toBe(0);
    expect(notchDot([], 0)).toBeNull();
  });

  it("drags to the nearest LED under the pointer", () => {
    expect(nearestLed(dots, 21, 1)).toBe(2);
    expect(nearestLed(dots, 4, 0)).toBe(0);
  });
});

describe("the first LED in force", () => {
  it("is the owner's LED, else LED 0 forward and the last LED reversed (the old flip)", () => {
    expect(effectiveFirstLed(3, false, 8)).toBe(3);
    expect(effectiveFirstLed(3, true, 8)).toBe(3);
    expect(effectiveFirstLed(0, true, 8)).toBe(0);
    expect(effectiveFirstLed(undefined, false, 8)).toBe(0);
    expect(effectiveFirstLed(undefined, true, 8)).toBe(7);
    expect(effectiveFirstLed(9, true, 8)).toBe(7);
  });

  it("keeps the first LED first when the direction turns", () => {
    expect(directionPatch(true, undefined, false, 8)).toEqual({ reversed: true, first_led: 0 });
    expect(directionPatch(false, undefined, true, 8)).toEqual({ reversed: false, first_led: 7 });
    expect(directionPatch(true, 3, false, 8)).toEqual({ reversed: true, first_led: 3 });
  });
});

describe("identify addresses a LED of the zone", () => {
  it("adds the light's slice start to its first LED", () => {
    expect(identifyLed({ start: 12 }, 3)).toBe(15);
    expect(identifyLed(null, 3)).toBe(3);
    expect(identifyLed(undefined, 0)).toBe(0);
  });
});

describe("the pointer in a letterboxed view", () => {
  const rect = { left: 10, top: 20, width: 76, height: 36 };

  it("maps a point through the bands of a wide box round a square view", () => {
    // 132x132 in 76x36: scale 36/132, centred sideways with a 20px band on each side.
    const scale = 36 / 132;
    const offsetX = (76 - 132 * scale) / 2;
    const at = clientToView(rect, 132, 132, 10 + offsetX + 66 * scale, 20 + 18);
    expect(at.x).toBeCloseTo(66, 6);
    expect(at.y).toBeCloseTo(66, 6);
  });

  it("agrees with a square box, where there is no band", () => {
    const at = clientToView({ left: 0, top: 0, width: 400, height: 400 }, 200, 200, 100, 300);
    expect(at).toEqual({ x: 50, y: 150 });
  });
});
