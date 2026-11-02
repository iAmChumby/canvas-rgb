import { describe, expect, it } from "vitest";
import type { EffectDetail } from "@/proto/EffectDetail";
import { stackLayerNames } from "@/lib/stackLayers";

/** A stack schema with the property labels the daemon derives: `{layer} · {property}`. */
function schemaWith(labels: string[], description = ""): EffectDetail["schema"] {
  return {
    id: "stack:Test",
    name: "Test",
    description,
    needs: { audio: false, screen: false },
    properties: labels.map((label, index) => ({ key: `l${index + 1}.speed`, label })),
    presets: [],
    thumbnail: null,
  } as unknown as EffectDetail["schema"];
}

describe("stackLayerNames", () => {
  it("reads each layer's name from the part before the property label", () => {
    const names = stackLayerNames(schemaWith(["Front fan · Speed", "Heat · Tint"]));
    expect(names).toEqual(["Front fan", "Heat"]);
  });

  it("never splits a layer name on the separator it holds", () => {
    expect(stackLayerNames(schemaWith(["Front · fan · Speed"]))).toEqual(["Front · fan"]);
  });

  it("falls back to the description's names when no layer has a property", () => {
    expect(stackLayerNames(schemaWith([], "2 layers: Gradient, Fire"))).toEqual(["Gradient", "Fire"]);
  });
});
