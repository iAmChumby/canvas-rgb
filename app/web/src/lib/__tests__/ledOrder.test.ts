import { describe, expect, it } from "vitest";
import { nearestLed, notchDot, stepFirstLed } from "@/lib/ledOrder";
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
