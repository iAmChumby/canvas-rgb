// Animated layer masks (#256): the demo compositor and the builder's model.

import { describe, expect, it } from "vitest";
import { composeStack } from "@/lib/stackFrame";
import {
  htmlLayers,
  makeMask,
  maskRefusal,
  patchMask,
  setLayerMask,
  duplicateLayer,
} from "@/views/effects/stackDraft";
import type { LayerMask } from "@/proto/LayerMask";
import type { PropValue } from "@/proto/PropValue";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";

function color(value: string): PropValue {
  return { type: "color", value };
}

function layer(over: Partial<StackLayer> = {}): StackLayer {
  return {
    id: "l1",
    name: "Base",
    effect: "solid",
    props: { color: color("#ffffff") },
    area: { kind: "canvas" },
    blend: "normal",
    opacity: 100,
    visible: true,
    ...over,
  };
}

function mask(over: Partial<LayerMask> = {}): LayerMask {
  return {
    effect: "solid",
    props: { color: color("#000000") },
    source: "luma",
    invert: false,
    amount: 100,
    ...over,
  };
}

function frameOf(layerMask: LayerMask): number[] {
  const stack: Stack = { id: "stack:test", layers: [layer({ mask: layerMask })] };
  return [...composeStack(stack, null, 0, 8, 4)];
}

describe("a masked layer in the demo compositor", () => {
  it("paints nothing under a black mask and the layer whole under a white one", () => {
    expect(frameOf(mask()).every((value) => value === 0)).toBe(true);
    expect(frameOf(mask({ props: { color: color("#ffffff") } })).every((value) => value === 255)).toBe(true);
  });

  it("swaps what shows when the mask is inverted", () => {
    expect(frameOf(mask({ invert: true })).every((value) => value === 255)).toBe(true);
  });

  it("blends between no mask and the mask's coverage by the amount", () => {
    const half = frameOf(mask({ amount: 50 }));
    expect(half.every((value) => value === 128)).toBe(true);
    const none = frameOf(mask({ amount: 0 }));
    expect(none.every((value) => value === 255)).toBe(true);
  });

  it("reads brightness or alpha, as the daemon does", () => {
    // A saturated red mask: its luma is 21 % of the layer, its strongest channel is all of it.
    const luma = frameOf(mask({ props: { color: color("#ff0000") } }));
    expect(luma[0]).toBe(54);
    const alpha = frameOf(mask({ props: { color: color("#ff0000") }, source: "alpha" }));
    expect(alpha[0]).toBe(255);
  });

  it("draws a layer without a mask exactly as before", () => {
    const stack: Stack = { id: "stack:test", layers: [layer()] };
    const frame = composeStack(stack, null, 0, 8, 4);
    expect([...frame].every((value) => value === 255)).toBe(true);
  });
});

describe("the builder's mask model", () => {
  it("starts a mask at Brightness, not inverted, applied fully", () => {
    const made = makeMask("fire", null, {});
    expect(made).toEqual({ effect: "fire", props: {}, source: "luma", invert: false, amount: 100 });
  });

  it("counts an HTML mask toward the HTML cap", () => {
    expect(htmlLayers([layer()])).toBe(0);
    expect(htmlLayers([layer({ mask: mask({ effect: "html:spark" }) })])).toBe(1);
    expect(htmlLayers([layer({ effect: "html:a" }), layer({ id: "l2", mask: mask({ effect: "html:b" }) })])).toBe(2);
  });

  it("refuses an HTML mask when the cap is full, unless it replaces an HTML mask", () => {
    const full = [layer({ effect: "html:a" }), layer({ id: "l2", mask: mask({ effect: "html:b" }) })];
    expect(maskRefusal(full, "l1", "html:c")).toBe("A stack holds 2 HTML layers");
    expect(maskRefusal(full, "l2", "html:c")).toBeNull();
    expect(maskRefusal([layer()], "l1", "stack:Other")).toBe("A stack cannot be a mask");
    expect(maskRefusal([layer()], "l1", "fire")).toBeNull();
  });

  it("sets, patches and removes a mask without touching the layer", () => {
    const before = [layer()];
    const withMask = setLayerMask(before, "l1", mask({ effect: "fire" }));
    expect(withMask[0]?.mask?.effect).toBe("fire");
    expect(before[0]?.mask).toBeUndefined();
    const patched = patchMask(withMask, "l1", { invert: true, amount: 40 });
    expect(patched[0]?.mask).toMatchObject({ effect: "fire", invert: true, amount: 40 });
    expect(setLayerMask(patched, "l1", null)[0]?.mask).toBeUndefined();
  });

  it("copies a layer's mask with the layer, so the two do not share one", () => {
    const answer = duplicateLayer([layer({ mask: mask({ effect: "fire" }) })], "l1");
    expect(answer.id).not.toBeNull();
    const copy = answer.layers[1]?.mask;
    expect(copy?.effect).toBe("fire");
    expect(copy).not.toBe(answer.layers[0]?.mask);
    expect(copy?.props).not.toBe(answer.layers[0]?.mask?.props);
  });
});
