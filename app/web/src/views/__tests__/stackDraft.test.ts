// The stack builder's draft rules (D95.6 test 15): the layer list, the values, the session's
// epoch and the sequence. Pure functions, so the test is about the rules and not the DOM.

import { describe, expect, it } from "vitest";
import type { LayerArea } from "@/proto/LayerArea";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { StackLayer } from "@/proto/StackLayer";
import {
  addLayer,
  addLayerRefusal,
  duplicateLayer,
  layerNameProblem,
  makeLayer,
  moveBounds,
  moveLayer,
  newEpoch,
  nextLayerId,
  patchLayer,
  removeLayer,
  resizeBounds,
  stackNameProblem,
  StackSession,
  toggleLayer,
  layersForServer,
} from "@/views/effects/stackDraft";

const color = (value: string) => ({ type: "color" as const, value });

const schema: PropertySchema = {
  id: "solid",
  name: "Solid",
  description: "",
  needs: { screen: false, audio: false },
  properties: [{ key: "color", label: "Color", kind: { type: "color", default: "#ff0000" } }],
  presets: [],
};

function layer(id: string, name = `Layer ${id}`, props = {}): StackLayer {
  return {
    id,
    name,
    effect: "solid",
    props: { color: color("#112233"), ...props },
    area: { kind: "canvas" },
    blend: "normal",
    opacity: 100,
    visible: true,
  };
}

describe("the layer list", () => {
  it("adds a layer on top, and gives it the next free id", () => {
    const first = makeLayer("solid", "Solid", schema, { color: color("#000000") });
    expect(first.props.color).toEqual(color("#000000"));
    const list = addLayer([layer("l1")], { ...first, id: "l2" });
    expect(list.map((entry) => entry.id)).toEqual(["l1", "l2"]);
    expect(nextLayerId(list)).toBe("l3");
  });

  it("stops at eight layers and at two HTML layers", () => {
    const eight = Array.from({ length: 8 }, (_, i) => layer(`l${i + 1}`));
    expect(addLayerRefusal(eight.slice(0, 7))).toBeNull();
    expect(addLayerRefusal(eight)).toBe("A stack holds 8 layers");
    expect(addLayerRefusal([layer("l1", "A", {})].map((entry) => ({ ...entry, effect: "html:a" })))).toBeNull();
    const html = [
      { ...layer("l1"), effect: "html:a" },
      { ...layer("l2"), effect: "html:b" },
    ];
    expect(addLayerRefusal(html, "html:c")).toBe("A stack holds 2 HTML layers");
  });

  it("removes a layer by id", () => {
    expect(removeLayer([layer("l1"), layer("l2")], "l1").map((entry) => entry.id)).toEqual(["l2"]);
  });

  it("reorders, and every layer keeps its own props", () => {
    const list = [
      layer("l1", "One", { color: color("#111111") }),
      layer("l2", "Two", { color: color("#222222") }),
      layer("l3", "Three", { color: color("#333333") }),
    ];
    const moved = moveLayer(list, "l3", 0);
    expect(moved.map((entry) => entry.name)).toEqual(["Three", "One", "Two"]);
    expect(moved[0]?.props.color).toEqual(color("#333333"));
    expect(moved[1]?.props.color).toEqual(color("#111111"));
    expect(moved[2]?.props.color).toEqual(color("#222222"));
    // Moving past the end clamps instead of losing the layer.
    expect(moveLayer(list, "l1", 99).map((entry) => entry.id)).toEqual(["l2", "l3", "l1"]);
  });

  it("duplicates with a fresh id, and the copy's values are its own", () => {
    const list = [layer("l1", "Heat"), layer("l2", "Sunset")];
    const answer = duplicateLayer(list, "l1");
    expect(answer.id).toBe("l3");
    expect(answer.layers.map((entry) => entry.id)).toEqual(["l1", "l3", "l2"]);
    expect(answer.layers[1]?.name).toBe("Heat copy");
    const copy = answer.layers[1];
    if (copy) copy.props.color = color("#999999");
    expect(answer.layers[0]?.props.color).toEqual(color("#112233"));
  });

  it("keeps a duplicate out of the HTML cap", () => {
    const list = [
      { ...layer("l1"), effect: "html:a" },
      { ...layer("l2"), effect: "html:b" },
    ];
    expect(duplicateLayer(list, "l1").id).toBeNull();
  });

  it("toggles the eye, renames, and changes blend, opacity, area and a prop", () => {
    let list = [layer("l1")];
    list = toggleLayer(list, "l1");
    expect(list[0]?.visible).toBe(false);
    list = patchLayer(list, "l1", { name: "Renamed" });
    expect(list[0]?.name).toBe("Renamed");
    list = patchLayer(list, "l1", { blend: "add", opacity: 40 });
    expect(list[0]?.blend).toBe("add");
    expect(list[0]?.opacity).toBe(40);
    const area: LayerArea = { kind: "rect", x: 16, y: 150, width: 110, height: 46 };
    list = patchLayer(list, "l1", { area });
    expect(list[0]?.area).toEqual(area);
    list = patchLayer(list, "l1", { props: { color: color("#abcdef") } });
    expect(list[0]?.props.color).toEqual(color("#abcdef"));
  });

  it("rejects a layer name it cannot save", () => {
    expect(layerNameProblem("Heat")).toBeNull();
    expect(layerNameProblem(" Heat")).not.toBeNull();
    expect(layerNameProblem("")).not.toBeNull();
    expect(layerNameProblem("Heat/2")).not.toBeNull();
    expect(layerNameProblem("x".repeat(41))).not.toBeNull();
  });
});

describe("the session", () => {
  it("makes a 16-character epoch when the builder opens", () => {
    const a = newEpoch();
    const b = newEpoch();
    expect(a).toMatch(/^[0-9a-f]{16}$/);
    expect(b).toMatch(/^[0-9a-f]{16}$/);
    expect(a).not.toBe(b);
  });

  it("starts at 1 and moves past a dropped PUT so it is never resent", () => {
    const session = new StackSession("0123456789abcdef");
    expect(session.next()).toEqual({ epoch: "0123456789abcdef", seq: 1 });
    // The daemon says a newer PUT owns the preview at seq 4: this change lost.
    session.observe({ epoch: "0123456789abcdef", seq: 4 });
    // The next change is above the owner, so it wins; nothing was resent on its own.
    expect(session.next()).toEqual({ epoch: "0123456789abcdef", seq: 5 });
  });

  it("ignores an answer from another epoch", () => {
    const session = new StackSession("aaaaaaaaaaaaaaaa");
    session.next();
    session.observe({ epoch: "bbbbbbbbbbbbbbbb", seq: 40 });
    expect(session.next().seq).toBe(2);
  });
});

describe("the stack name", () => {
  it("is checked before the save asks the daemon", () => {
    expect(stackNameProblem("Sunset glow", [])).toBeNull();
    expect(stackNameProblem("", [])).not.toBeNull();
    expect(stackNameProblem("preview", [])).toContain("preview");
    expect(stackNameProblem("Sunset glow", ["sunset glow"])).not.toBeNull();
    expect(stackNameProblem("con", [])).toContain("Windows");
    expect(stackNameProblem("a".repeat(61), [])).not.toBeNull();
  });
});

describe("resizing a rectangle", () => {
  const inside = (b: { x: number; y: number; width: number; height: number }) =>
    b.x >= 0 && b.y >= 0 && b.x + b.width <= 320 && b.y + b.height <= 200 && b.width >= 1 && b.height >= 1;

  it("keeps a one-unit rectangle in the canvas when its left edge is dragged right", () => {
    const r = resizeBounds({ x: 0, y: 0, width: 1, height: 1 }, -1, -1, 50, 50);
    expect(r).toEqual({ x: 0, y: 0, width: 1, height: 1 });
  });

  it("keeps a one-unit rectangle at the right edge in the canvas when its right edge is dragged right", () => {
    const r = resizeBounds({ x: 319, y: 199, width: 1, height: 1 }, 1, 1, 50, 50);
    expect(r).toEqual({ x: 319, y: 199, width: 1, height: 1 });
    expect(inside(resizeBounds({ x: 319, y: 0, width: 1, height: 1 }, 1, 0, 9, 0))).toBe(true);
  });

  it("never grows a two-unit area when an edge is dragged inward", () => {
    const r = resizeBounds({ x: 10, y: 10, width: 2, height: 2 }, -1, 0, 30, 0);
    expect(r.width).toBe(2);
    expect(r.x).toBe(10);
  });

  it("stops at four units on a larger area, and stays inside on every edge and corner", () => {
    const start = { x: 100, y: 80, width: 50, height: 30 };
    for (const sx of [-1, 0, 1] as const) {
      for (const sy of [-1, 0, 1] as const) {
        for (const d of [-999, -5, 0, 5, 999]) {
          const r = resizeBounds(start, sx, sy, d, -d, d % 2 === 0);
          expect(inside(r)).toBe(true);
        }
      }
    }
    expect(resizeBounds(start, -1, 0, 999, 0).width).toBe(4);
  });

  it("rounds the edges, not the width, so a half-unit never leaves the canvas", () => {
    // x 219.5 with the right edge clamped to 320: width 100.5 rounds to 221 - 220 = 100, not 101.
    const r = resizeBounds({ x: 219.5, y: 100, width: 50, height: 50 }, 1, 0, 9999, 0);
    expect(r.x + r.width).toBe(320);
    const b = resizeBounds({ x: 10, y: 199.5, width: 5, height: 0.5 }, 0, 1, 0, 9999);
    expect(b.y + b.height).toBeLessThanOrEqual(200);
  });

  it("stays inside the canvas for every half-unit start, handle and delta", () => {
    for (const x of [0, 0.5, 219.5, 300.5, 315.5]) {
      for (const y of [0, 0.5, 149.5, 195.5]) {
        for (const w of [1, 4.5, 100.5]) {
          for (const h of [1, 4.5, 45.5]) {
            if (x + w > 320 || y + h > 200) continue;
            for (const sx of [-1, 0, 1] as const) {
              for (const sy of [-1, 0, 1] as const) {
                for (const d of [-999, -7.5, 0.5, 7.5, 999]) {
                  for (const r of [
                    resizeBounds({ x, y, width: w, height: h }, sx, sy, d, -d),
                    resizeBounds({ x, y, width: w, height: h }, sx, sy, d, d, true),
                    moveBounds({ x, y, width: w, height: h }, d, -d),
                  ]) {
                    expect(r.x).toBeGreaterThanOrEqual(0);
                    expect(r.y).toBeGreaterThanOrEqual(0);
                    expect(r.x + r.width).toBeLessThanOrEqual(320);
                    expect(r.y + r.height).toBeLessThanOrEqual(200);
                    expect(Number.isInteger(r.width) && Number.isInteger(r.height)).toBe(true);
                  }
                }
              }
            }
          }
        }
      }
    }
  });

  it("moves a rectangle whole inside the canvas", () => {
    expect(moveBounds({ x: 10, y: 10, width: 300, height: 190 }, 500, 500)).toEqual({ x: 20, y: 10, width: 300, height: 190 });
  });
});

describe("layersForServer (#118)", () => {
  const layer = (area: StackLayer["area"]): StackLayer =>
    ({ id: "l1", name: "Base", effect: "solid", props: {}, opacity: 100, blend: "normal", enabled: true, area }) as unknown as StackLayer;

  it("clamps a stored rect that runs off the canvas to its on-canvas box", () => {
    const [out] = layersForServer([layer({ kind: "rect", x: -3, y: 5, width: 10, height: 10 })]);
    expect(out!.area).toEqual({ kind: "rect", x: 0, y: 5, width: 7, height: 10 });
    const [right] = layersForServer([layer({ kind: "rect", x: 315, y: 195, width: 10, height: 10 })]);
    expect(right!.area).toEqual({ kind: "rect", x: 315, y: 195, width: 5, height: 5 });
  });

  it("leaves an in-canvas rect and a canvas area untouched (same object)", () => {
    const inside = layer({ kind: "rect", x: 0, y: 0, width: 320, height: 200 });
    const whole = layer({ kind: "canvas" } as StackLayer["area"]);
    const out = layersForServer([inside, whole]);
    expect(out[0]).toBe(inside);
    expect(out[1]).toBe(whole);
  });

  it("turns a rect wholly off the canvas into an empty one", () => {
    const [out] = layersForServer([layer({ kind: "rect", x: 400, y: 10, width: 10, height: 10 })]);
    expect(out!.area).toEqual({ kind: "rect", x: 320, y: 10, width: 0, height: 10 });
  });
});
