// The Gradient tool's maths (#468): stops to LED colours, blended in OKLab, with Wrap.

import { describe, expect, it } from "vitest";
import { colourAt, gradientColours, ledNear, ledPosition, seedStops } from "@/lib/gradient";
import { mixOklab, toOklab } from "@/lib/oklab";
import { spinColours } from "@/lib/spin";

const RED = "#ff0000";
const GREEN = "#00ff00";
const BLUE = "#0000ff";
const THIRDS = [
  { at: 0, colour: RED },
  { at: 1 / 3, colour: GREEN },
  { at: 2 / 3, colour: BLUE },
];

/** How far apart two colours look: the distance in OKLab. */
function distance(a: string, b: string): number {
  const [l1, a1, b1] = toOklab(a);
  const [l2, a2, b2] = toOklab(b);
  return Math.hypot(l1 - l2, a1 - a2, b1 - b2);
}

describe("the gradient across a ring", () => {
  it("puts each stop's colour on the LED at its position", () => {
    const ring = gradientColours(THIRDS, 12, true);
    expect(ring[0]).toBe(RED);
    expect(ring[4]).toBe(GREEN);
    expect(ring[8]).toBe(BLUE);
  });

  it("blends between stops in OKLab, not RGB", () => {
    const ring = gradientColours(THIRDS, 12, true);
    expect(ring[2]).toBe(mixOklab(RED, GREEN, 0.5));
    expect(ring[2]).not.toBe("#808000");
  });

  it("meets the last stop to the first across the seam: no jump, however many LEDs", () => {
    for (const leds of [8, 12, 24, 40]) {
      const ring = gradientColours(THIRDS, leds, true);
      // The step across the seam (last LED to LED 0) is no bigger than the steps inside the ring.
      const steps = ring.map((c, i) => distance(c, ring[(i + 1) % leds] as string));
      const seam = steps[leds - 1] as number;
      expect(seam, `${leds} LEDs`).toBeLessThanOrEqual(Math.max(...steps.slice(0, leds - 1)) + 1e-9);
      // And the position one lap round reads the first stop's colour.
      expect(distance(colourAt(THIRDS, 1 - 1e-9, true), RED)).toBeLessThan(1e-4);
    }
  });

  it("shows no seam once it is spun: every lap-fraction keeps the same smooth ring", () => {
    const ring = gradientColours(THIRDS, 24, true);
    const base = Math.max(...ring.map((c, i) => distance(c, ring[(i + 1) % 24] as string)));
    for (let t = 0; t < 4; t += 0.17) {
      const spun = spinColours(ring, 0.25, t);
      const worst = Math.max(...spun.map((c, i) => distance(c, spun[(i + 1) % 24] as string)));
      // A blend between LEDs makes steps no larger than the ring's own.
      expect(worst, `at ${t}s`).toBeLessThanOrEqual(base + 0.01);
    }
  });

  it("without Wrap the colours before the first stop and after the last hold", () => {
    const stops = [
      { at: 0.25, colour: RED },
      { at: 0.75, colour: BLUE },
    ];
    const line = gradientColours(stops, 9, false);
    expect(line[0]).toBe(RED);
    expect(line[1]).toBe(RED);
    expect(line[8]).toBe(BLUE);
    expect(line[7]).toBe(BLUE);
    expect(line[4]).toBe(mixOklab(RED, BLUE, 0.5));
  });
});

describe("stops", () => {
  it("seed three stops spaced round the hue wheel", () => {
    const ring = seedStops(0, true);
    expect(ring.map((s) => s.at)).toEqual([0, 1 / 3, 2 / 3]);
    expect(new Set(ring.map((s) => s.colour)).size).toBe(3);
    const line = seedStops(200, false);
    expect(line.map((s) => s.at)).toEqual([0, 0.5, 1]);
  });

  it("map LEDs to positions and back", () => {
    for (const wrap of [true, false]) {
      for (let i = 0; i < 12; i++) expect(ledNear(ledPosition(i, 12, wrap), 12, wrap)).toBe(i);
    }
    expect(ledNear(0.999, 12, true)).toBe(0);
    expect(ledPosition(0, 1, true)).toBe(0);
  });

  it("with no stops, or one, are black or flat", () => {
    expect(colourAt([], 0.3, true)).toBe("#000000");
    expect(gradientColours([{ at: 0.5, colour: GREEN }], 5, true)).toEqual(Array(5).fill(GREEN));
  });
});
