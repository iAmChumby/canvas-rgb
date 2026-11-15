// The stage's hit order and keys (#257 phase 1): which layer a press picks, and which layer Tab
// moves to. Pure, so the rules are tested without a browser.

import { describe, expect, it } from "vitest";
import type { LayerArea } from "@/proto/LayerArea";
import type { StackLayer } from "@/proto/StackLayer";
import {
  areaAsRect,
  cycleLayer,
  layerAtPoint,
  partLabelPlan,
  pointerToStage,
  stageFrames,
  stageHitOrder,
} from "@/views/effects/stackDraft";
import { canvasWith } from "@/lib/__tests__/groupCanvasFixture";

const desk = canvasWith([
  { id: "kb:1#z0", label: "Keyboard", device: "kb:1", zone: 0, x: 20, y: 150, width: 100, height: 40 },
  { id: "mouse:1#z0", label: "Mouse", device: "mouse:1", zone: 0, x: 130, y: 150, width: 30, height: 30 },
]);

function layer(id: string, area: LayerArea, extra: Partial<StackLayer> = {}): StackLayer {
  return {
    id,
    name: `Layer ${id}`,
    effect: "solid",
    props: {},
    area,
    blend: "normal",
    opacity: 100,
    visible: true,
    ...extra,
  };
}

const canvas = (): LayerArea => ({ kind: "canvas" });
const rect = (x: number, y: number, width: number, height: number): LayerArea => ({
  kind: "rect",
  x,
  y,
  width,
  height,
});

describe("the frames the stage draws", () => {
  it("draws every visible layer and skips a layer that is switched off", () => {
    const layers = [layer("l1", canvas()), layer("l2", rect(10, 10, 20, 20), { visible: false }), layer("l3", rect(0, 0, 8, 8))];
    expect(stageFrames(layers, desk).map((frame) => frame.id)).toEqual(["l1", "l3"]);
  });

  it("skips an area whose device is not on the desk", () => {
    const layers = [layer("l1", { kind: "device", device: "gone:1" })];
    expect(stageFrames(layers, desk)).toEqual([]);
  });

  it("measures a canvas frame as the whole picture", () => {
    expect(stageFrames([layer("l1", canvas())], desk)[0]?.area).toBe(320 * 200);
  });
});

describe("a press on the stage", () => {
  it("takes the smallest area under the point", () => {
    const layers = [layer("big", rect(0, 0, 100, 100)), layer("small", rect(40, 40, 10, 10))];
    expect(layerAtPoint(layers, desk, { x: 45, y: 45 })).toBe("small");
    expect(layerAtPoint(layers, desk, { x: 5, y: 5 })).toBe("big");
  });

  it("puts a whole-canvas layer last, however big its neighbours are not", () => {
    const layers = [layer("canvas", canvas()), layer("wide", rect(0, 0, 319, 199))];
    // The rectangle is nearly the whole picture, and the canvas is all of it: the smaller wins.
    expect(layerAtPoint(layers, desk, { x: 100, y: 100 })).toBe("wide");
  });

  it("keeps the whole-canvas layer last even under a bigger rectangle", () => {
    // A canvas frame is 320 x 200; a rectangle can never be bigger, so this is the ordering rule
    // stated the other way round: a canvas never beats a rectangle at a point both hold.
    const layers = [layer("canvas", canvas()), layer("zone", { kind: "zone", device: "kb:1", zone: 0 })];
    expect(layerAtPoint(layers, desk, { x: 60, y: 160 })).toBe("zone");
  });

  it("gives an equal area to the layer painted on top", () => {
    const layers = [layer("lower", rect(10, 10, 20, 20)), layer("upper", rect(10, 10, 20, 20))];
    expect(layerAtPoint(layers, desk, { x: 20, y: 20 })).toBe("upper");
  });

  it("answers null on the empty desk", () => {
    expect(layerAtPoint([layer("l1", rect(10, 10, 20, 20))], desk, { x: 200, y: 20 })).toBeNull();
    expect(layerAtPoint([], desk, { x: 0, y: 0 })).toBeNull();
  });

  it("orders frames best first without changing the list it was given", () => {
    const frames = stageFrames([layer("canvas", canvas()), layer("small", rect(0, 0, 4, 4))], desk);
    const order = stageHitOrder(frames).map((frame) => frame.id);
    expect(order).toEqual(["small", "canvas"]);
    expect(frames.map((frame) => frame.id)).toEqual(["canvas", "small"]);
  });
});

describe("Tab through the layers", () => {
  const layers = [layer("l1", canvas()), layer("l2", canvas()), layer("l3", canvas())];

  it("walks top first and wraps", () => {
    expect(cycleLayer(layers, "l3", 1)).toBe("l2");
    expect(cycleLayer(layers, "l2", 1)).toBe("l1");
    expect(cycleLayer(layers, "l1", 1)).toBe("l3");
  });

  it("walks the other way with Shift", () => {
    expect(cycleLayer(layers, "l3", -1)).toBe("l1");
    expect(cycleLayer(layers, "l1", -1)).toBe("l2");
  });

  it("starts at the top when nothing is picked", () => {
    expect(cycleLayer(layers, null, 1)).toBe("l3");
    expect(cycleLayer(layers, null, -1)).toBe("l1");
    expect(cycleLayer([], null, 1)).toBeNull();
  });
});

describe("Make rectangle", () => {
  it("turns a canvas area into the picture's own box", () => {
    expect(areaAsRect(canvas(), desk)).toEqual({ kind: "rect", x: 0, y: 0, width: 320, height: 200 });
  });

  it("turns a zone into the box it covers", () => {
    expect(areaAsRect({ kind: "zone", device: "kb:1", zone: 0 }, desk)).toEqual({
      kind: "rect",
      x: 20,
      y: 150,
      width: 100,
      height: 40,
    });
  });

  it("leaves a rectangle alone", () => {
    const area = rect(10, 20, 30, 40);
    expect(areaAsRect(area, desk)).toBe(area);
  });

  it("offers nothing for an area that is not on the desk", () => {
    expect(areaAsRect({ kind: "device", device: "gone:1" }, desk)).toBeNull();
  });

  it("rounds outward, so the box covers every light it came from", () => {
    // 10.4 .. 30.6: rounded to the nearest, the box would lose the edges.
    const zone = canvasWith([
      { id: "z:0#z0", label: "Z", device: "z:0", x: 10.4, y: 20.5, width: 20.2, height: 20.6 },
    ]);
    const made = areaAsRect({ kind: "device", device: "z:0" }, zone);
    expect(made).toEqual({ kind: "rect", x: 10, y: 20, width: 21, height: 22 });
    if (!made || made.kind !== "rect") throw new Error("expected a rectangle");
    expect(made.x).toBeLessThanOrEqual(10.4);
    expect(made.y).toBeLessThanOrEqual(20.5);
    expect(made.x + made.width).toBeGreaterThanOrEqual(30.6);
    expect(made.y + made.height).toBeGreaterThanOrEqual(41.1);
  });

  it("covers the union of two lights of one device exactly, to the unit", () => {
    const both = canvasWith([
      { id: "z:0#z0", label: "One", device: "z:0", zone: 0, x: 10.2, y: 10.2, width: 10, height: 10 },
      { id: "z:0#z1", label: "Two", device: "z:0", zone: 1, x: 40.7, y: 30.1, width: 9.6, height: 9.6 },
    ]);
    expect(areaAsRect({ kind: "device", device: "z:0" }, both)).toEqual({ kind: "rect", x: 10, y: 10, width: 41, height: 30 });
  });
});

describe("a frame at opacity 0", () => {
  it("keeps its frame, so the layer can still be found and picked", () => {
    const layers = [layer("ghost", rect(10, 10, 40, 40), { opacity: 0 })];
    expect(stageFrames(layers, desk).map((frame) => frame.id)).toEqual(["ghost"]);
    expect(layerAtPoint(layers, desk, { x: 20, y: 20 })).toBe("ghost");
  });
});

describe("pointer to canvas on the stage", () => {
  const corners = (box: { left: number; top: number; width: number; height: number }) => ({
    nw: pointerToStage(box.left, box.top, box),
    se: pointerToStage(box.left + box.width, box.top + box.height, box),
  });

  it("maps a box with the canvas ratio one to one", () => {
    const box = { left: 10, top: 20, width: 640, height: 400 };
    expect(pointerToStage(10 + 320, 20 + 200, box)).toEqual({ x: 160, y: 100 });
    expect(corners(box).se).toEqual({ x: 320, y: 200 });
  });

  it("takes each axis from its own side when the height is clamped", () => {
    // A 640 x 300 box: 2 px per unit across, 1.5 px per unit down. A scale from the width alone
    // would put the bottom edge at y = 150 and a press in the lower half on the wrong layer.
    const box = { left: 0, top: 0, width: 640, height: 300 };
    expect(pointerToStage(640, 300, box)).toEqual({ x: 320, y: 200 });
    expect(pointerToStage(320, 150, box)).toEqual({ x: 160, y: 100 });
  });

  it("clamps a point outside the box to its edge", () => {
    const box = { left: 0, top: 0, width: 640, height: 400 };
    expect(pointerToStage(-50, 900, box)).toEqual({ x: 0, y: 200 });
  });
});

describe("the Parts names", () => {
  const part = (key: string, label: string, x: number, y: number, width: number, height: number) => ({
    key,
    label,
    x,
    y,
    width,
    height,
  });

  it("leaves a name off an outline narrower than three characters", () => {
    // At 2 px per unit a 10-unit outline is 20 px wide: no room for three characters and padding.
    const shown = partLabelPlan([part("tiny", "Fan", 10, 10, 10, 20), part("ok", "Fan", 100, 10, 40, 20)], 2);
    expect(shown.has("tiny")).toBe(false);
    expect(shown.has("ok")).toBe(true);
  });

  it("leaves a name off an outline too short for a line", () => {
    expect(partLabelPlan([part("flat", "Strip", 10, 10, 100, 4)], 2).size).toBe(0);
  });

  it("never lets two names land on each other, and the larger outline keeps its name", () => {
    const shown = partLabelPlan([part("fan", "Fan 1", 12, 12, 40, 30), part("case", "Case", 10, 10, 200, 100)], 2);
    expect([...shown]).toEqual(["case"]);
  });

  it("keeps two names that sit apart", () => {
    const shown = partLabelPlan([part("a", "Keyboard", 10, 10, 100, 40), part("b", "Mouse", 150, 10, 40, 40)], 2);
    expect(shown.has("a") && shown.has("b")).toBe(true);
  });

  it("shows nothing before the picture is measured", () => {
    expect(partLabelPlan([part("a", "Keyboard", 10, 10, 100, 40)], 0).size).toBe(0);
  });
});
