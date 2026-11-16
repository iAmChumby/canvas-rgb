// The two arithmetic helpers the chain flow is built on: the even split the count step shows
// first, the boundary a row's LED count moves, and the runs the daemon is told to light. The
// split must agree with the daemon's own `even_split` (`crates/canvas-daemon/src/chain_plan.rs`),
// so the screen and the file it writes say the same numbers.

import { describe, expect, it } from "vitest";

import { PART_COLOURS, defaultName, renameForKind } from "@/views/chain/partColours";
import { evenSplit, moveBoundary, runsFor } from "@/views/chain/split";

describe("the even split", () => {
  it("splits 96 into 7 the way the daemon does: 14 14 14 14 14 13 13", () => {
    expect(evenSplit(96, 7)).toEqual([14, 14, 14, 14, 14, 13, 13]);
    // The remainder goes to the first parts, and every LED is accounted for.
    expect(evenSplit(96, 7).reduce((n, leds) => n + leds, 0)).toBe(96);
  });

  it("splits evenly when the length divides by the count", () => {
    expect(evenSplit(70, 7)).toEqual([10, 10, 10, 10, 10, 10, 10]);
    expect(evenSplit(1, 1)).toEqual([1]);
  });

  it("refuses a count of zero", () => {
    expect(() => evenSplit(96, 0)).toThrow();
  });

  it("refuses more parts than the length has LEDs", () => {
    expect(() => evenSplit(4, 5)).toThrow();
  });
});

describe("moving a boundary", () => {
  it("gives the difference to the next row and keeps the total", () => {
    expect(moveBoundary([14, 14, 14, 14, 14, 13, 13], 0, 16)).toEqual([16, 12, 14, 14, 14, 13, 13]);
    expect(moveBoundary([14, 14, 14, 14, 14, 13, 13], 2, 10)).toEqual([14, 14, 10, 18, 14, 13, 13]);
  });

  it("changes the total on the last row", () => {
    const moved = moveBoundary([10, 10, 10], 2, 14);
    expect(moved).toEqual([10, 10, 14]);
    expect(moved.reduce((n, leds) => n + leds, 0)).toBe(34);
  });

  it("leaves no row below one LED", () => {
    // The next row can give up everything but its last LED.
    expect(moveBoundary([10, 10], 0, 40)).toEqual([19, 1]);
    // And the row being moved cannot go below one either.
    expect(moveBoundary([10, 10], 0, 0)).toEqual([1, 19]);
    expect(moveBoundary([5], 0, 0)).toEqual([1]);
  });
});

describe("the runs the daemon is told to light", () => {
  it("are consecutive from zero, each in its own colour", () => {
    const runs = runsFor([14, 14, 13]);
    expect(runs).toEqual([
      { start: 0, count: 14, color: PART_COLOURS[0] },
      { start: 14, count: 14, color: PART_COLOURS[1] },
      { start: 28, count: 13, color: PART_COLOURS[2] },
    ]);
  });

  it("repeats the colour set on a chain longer than it", () => {
    const lengths = PART_COLOURS.map(() => 1).concat([1]);
    const runs = runsFor(lengths);
    expect(runs[PART_COLOURS.length]?.color).toBe(PART_COLOURS[0]);
  });
});

describe("the colour set", () => {
  it("holds at least ten fixed colours, each a #rrggbb string", () => {
    expect(PART_COLOURS.length).toBeGreaterThanOrEqual(10);
    for (const colour of PART_COLOURS) expect(colour).toMatch(/^#[0-9a-f]{6}$/);
  });

  it("has no two entries the same", () => {
    expect(new Set(PART_COLOURS).size).toBe(PART_COLOURS.length);
  });

  it("keeps every colour well clear of white", () => {
    // A part's colour must read as a colour on an LED, not as the white the step is drawn in.
    for (const colour of PART_COLOURS) {
      const [r, g, b] = [1, 3, 5].map((at) => parseInt(colour.slice(at, at + 2), 16));
      const distance = Math.hypot(255 - (r ?? 0), 255 - (g ?? 0), 255 - (b ?? 0));
      expect(distance, `${colour} is too near white`).toBeGreaterThan(120);
    }
  });

  it("starts away from the blue the editor's chrome uses", () => {
    // The first colour marks the last LED of the chain (step 1); a blue there reads as chrome.
    const first = PART_COLOURS[0] ?? "";
    const [r, , b] = [1, 3, 5].map((at) => parseInt(first.slice(at, at + 2), 16));
    expect(r ?? 0).toBeGreaterThan(150);
    expect(b ?? 0).toBeLessThan(120);
  });
});

describe("a part's name", () => {
  it("says Fan, Strip or Device, counted from one", () => {
    expect(defaultName("fan", 1)).toBe("Fan 1");
    expect(defaultName("strip", 2)).toBe("Strip 2");
    expect(defaultName("other", 3)).toBe("Device 3");
  });

  it("follows a new kind unless the owner typed a name", () => {
    expect(renameForKind("Fan 1", false, "strip", 1)).toBe("Strip 1");
    expect(renameForKind("Top intake", true, "strip", 1)).toBe("Top intake");
    // A name that is only the old default is the app's, not the owner's.
    expect(renameForKind("Fan 2", false, "other", 2)).toBe("Device 2");
  });
});
