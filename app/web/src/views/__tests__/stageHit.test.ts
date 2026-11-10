// The stage's hit order and keys (#257 phase 1): which layer a press picks, and which layer Tab
// moves to. Pure, so the rules are tested without a browser.

import { describe, expect, it } from "vitest";
import type { LayerArea } from "@/proto/LayerArea";
import type { Layout } from "@/proto/Layout";
import type { StackLayer } from "@/proto/StackLayer";
import {
  areaAsRect,
  cycleLayer,
  layerAtPoint,
  stageFrames,
  stageHitOrder,
} from "@/views/effects/stackDraft";

const desk: Layout = {
  id: "desk",
  name: "Desk",
  render_scale: 50,
  sampling: "area_average",
  case_form: "none",
  landmarks: [],
  effect: null,
  zones: [
    {
      device_id: "kb:1",
      zone_index: 0,
      x: 20,
      y: 150,
      width: 100,
      height: 40,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "strip" },
      label: "Keyboard",
    },
    {
      device_id: "mouse:1",
      zone_index: 0,
      x: 130,
      y: 150,
      width: 30,
      height: 30,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "strip" },
    },
  ],
};

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
});
