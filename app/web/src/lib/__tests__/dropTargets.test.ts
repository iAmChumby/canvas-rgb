// The drop targets (#257 phase 2): one pure table from a pointer to what a drop would mean.
// The desk is the demo layout, so the coordinates are the ones the screenshots use.

import { describe, expect, it } from "vitest";
import { DEVICES, LAYOUT_DESK } from "@/lib/fixtures";
import {
  dropArea,
  dropTargetAt,
  dropTargetWords,
  rectAtPoint,
  type DropBoxes,
} from "@/lib/dropTargets";

/** The stage is 640 x 400 css pixels at (0, 100): two pixels per canvas unit. */
const STAGE = { left: 0, top: 100, width: 640, height: 400 };
const STRIP = { left: 0, top: 76, width: 640, height: 24 };
const ROWS = [
  { id: "l3", index: 2, top: 500, bottom: 554 },
  { id: "l2", index: 1, top: 554, bottom: 608 },
  { id: "l1", index: 0, top: 608, bottom: 662 },
];
const boxes: DropBoxes = { stage: STAGE, strip: STRIP, rows: ROWS };
/** A canvas point as a client point on the mocked stage. */
const at = (x: number, y: number) => ({ x: x * 2, y: 100 + y * 2 });
const layers = [
  { id: "l1", name: "Sunset" },
  { id: "l2", name: "Heat" },
  { id: "l3", name: "Sparkles" },
];
const names = { layout: LAYOUT_DESK, devices: DEVICES, layers };

describe("a pointer on the stage", () => {
  it("lands on the device of the smallest zone under it", () => {
    // The keyboard's 110 x 30 zone, inside both whole-canvas layers and nothing else.
    expect(dropTargetAt({ point: at(71, 173), boxes, layout: LAYOUT_DESK, payload: "effect" })).toEqual({
      kind: "device",
      device: "royuan:3151:4015",
    });
  });

  it("lands on that one zone with Shift", () => {
    expect(
      dropTargetAt({ point: at(71, 173), boxes, layout: LAYOUT_DESK, payload: "effect", shift: true }),
    ).toEqual({ kind: "zone", device: "royuan:3151:4015", zone: 0 });
  });

  it("lands on a new rectangle in empty picture, centred on the pointer", () => {
    expect(dropTargetAt({ point: at(200, 100), boxes, layout: LAYOUT_DESK, payload: "effect" })).toEqual({
      kind: "rect",
      x: 140,
      y: 60,
      width: 120,
      height: 80,
    });
  });

  it("lands on the whole canvas on the strip above the picture", () => {
    expect(dropTargetAt({ point: { x: 100, y: 90 }, boxes, layout: LAYOUT_DESK, payload: "effect" })).toEqual({
      kind: "canvas",
    });
  });

  it("holds a new rectangle inside the canvas at the edges", () => {
    expect(rectAtPoint({ x: 2, y: 2 })).toEqual({ x: 0, y: 0, width: 120, height: 80 });
    expect(rectAtPoint({ x: 318, y: 198 })).toEqual({ x: 200, y: 120, width: 120, height: 80 });
  });

  it("has no target off the stage and off the stack", () => {
    expect(dropTargetAt({ point: { x: 900, y: 900 }, boxes, layout: LAYOUT_DESK, payload: "effect" })).toBeNull();
  });
});

describe("a pointer on the layer list", () => {
  const slot = (y: number) => dropTargetAt({ point: { x: 300, y }, boxes, layout: LAYOUT_DESK, payload: "effect" });

  it("names the insertion slot an effect would take", () => {
    // Above the top row's middle: above Sparkles, which is the top of the list.
    expect(slot(510)).toEqual({ kind: "stackSlot", at: 3 });
    // Below the top row's middle: between Heat and Sparkles.
    expect(slot(550)).toEqual({ kind: "stackSlot", at: 2 });
    // Under the bottom row: under Sunset, the bottom of the list.
    expect(slot(660)).toEqual({ kind: "stackSlot", at: 0 });
  });

  it("names the row a layer would take", () => {
    expect(
      dropTargetAt({ point: { x: 300, y: 570 }, boxes, layout: LAYOUT_DESK, payload: "layer" }),
    ).toEqual({ kind: "row", id: "l2", index: 1 });
  });
});

describe("what a target is called and what it places", () => {
  it("says a device by its own name and a zone by its label", () => {
    expect(dropTargetWords({ kind: "device", device: "royuan:3151:4015" }, names)).toBe(
      "ROYUAN MonsGeek Keyboard",
    );
    expect(dropTargetWords({ kind: "zone", device: "royuan:3151:4015", zone: 0 }, names)).toBe(
      "ROYUAN MonsGeek Keyboard · Zone 1",
    );
    expect(dropTargetWords({ kind: "rect", x: 1, y: 2, width: 3, height: 4 }, names)).toBe("New rectangle");
    expect(dropTargetWords({ kind: "stackSlot", at: 3 }, names)).toBe("Top of the list");
    expect(dropTargetWords({ kind: "stackSlot", at: 2 }, names)).toBe("Above Heat");
    expect(dropTargetWords({ kind: "stackSlot", at: 0 }, names)).toBe("Bottom of the list");
    expect(dropTargetWords({ kind: "row", id: "l1", index: 0 }, names)).toBe("Row Sunset");
  });

  it("places an area for every stage target, and none for a stack position", () => {
    expect(dropArea({ kind: "canvas" })).toEqual({ kind: "canvas" });
    expect(dropArea({ kind: "device", device: "d" })).toEqual({ kind: "device", device: "d" });
    expect(dropArea({ kind: "zone", device: "d", zone: 2 })).toEqual({ kind: "zone", device: "d", zone: 2 });
    expect(dropArea({ kind: "rect", x: 4, y: 5, width: 6, height: 7 })).toEqual({
      kind: "rect",
      x: 4,
      y: 5,
      width: 6,
      height: 7,
    });
    expect(dropArea({ kind: "stackSlot", at: 1 })).toBeNull();
    expect(dropArea({ kind: "row", id: "l1", index: 0 })).toBeNull();
  });
});
