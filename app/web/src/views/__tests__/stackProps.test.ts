// Merged props (D95.6 test 16): `{layer.id}.{key}` there and back, and a stack's schema lists
// every layer's properties.

import { describe, expect, it } from "vitest";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { StackLayer } from "@/proto/StackLayer";
import type { PropValue } from "@/proto/PropValue";
import { applyMergedProps, mergedProps, stackSchema } from "@/views/effects/stackDraft";

const color = (value: string): PropValue => ({ type: "color", value });
const number = (value: number): PropValue => ({ type: "number", value });

const solid: PropertySchema = {
  id: "solid",
  name: "Solid",
  description: "",
  needs: { screen: false, audio: false },
  properties: [{ key: "color", label: "Color", kind: { type: "color", default: "#000000" } }],
  presets: [],
};
const fire: PropertySchema = {
  id: "fire",
  name: "Fire",
  description: "",
  needs: { screen: false, audio: false },
  properties: [
    { key: "intensity", label: "Intensity", kind: { type: "number", min: 0, max: 100, step: 1, default: 55 } },
    {
      key: "palette",
      label: "Palette",
      kind: { type: "choice", options: ["Classic", "Blue"], default: "Classic" },
    },
  ],
  presets: [],
};

const layers: StackLayer[] = [
  {
    id: "l1",
    name: "Sunset",
    effect: "solid",
    props: { color: color("#ff2d55") },
    area: { kind: "canvas" },
    blend: "normal",
    opacity: 100,
    visible: true,
  },
  {
    id: "l2",
    name: "Heat",
    effect: "fire",
    props: { intensity: number(72) },
    area: { kind: "rect", x: 16, y: 150, width: 110, height: 46 },
    blend: "add",
    opacity: 70,
    visible: true,
  },
];
const schemas: Record<string, PropertySchema> = { solid, fire };

describe("the merged map", () => {
  it("names every value by its layer", () => {
    const merged = mergedProps(layers, schemas);
    expect(merged["l1.color"]).toEqual(color("#ff2d55"));
    expect(merged["l2.intensity"]).toEqual(number(72));
    // A key the layer did not set comes from its effect's default.
    expect(merged["l2.palette"]).toEqual({ type: "choice", value: "Classic" });
  });

  it("goes back into the layers, and a key an effect does not have is dropped", () => {
    const merged = mergedProps(layers, schemas);
    const next = applyMergedProps(layers, { ...merged, "l1.stray": number(1), "l2.intensity": number(90) }, schemas);
    expect(next[0]?.props.color).toEqual(color("#ff2d55"));
    expect(next[0]?.props.stray).toBeUndefined();
    expect(next[1]?.props.intensity).toEqual(number(90));
  });

  it("round-trips", () => {
    const merged = mergedProps(layers, schemas);
    const back = applyMergedProps(layers, merged, schemas);
    expect(mergedProps(back, schemas)).toEqual(merged);
  });
});

describe("the stack's schema", () => {
  it("lists every layer's properties with its own id and label", () => {
    const schema = stackSchema("Sunset glow", layers, schemas);
    expect(schema.id).toBe("stack:Sunset glow");
    expect(schema.name).toBe("Sunset glow");
    expect(schema.description).toBe("2 layers: Solid, Fire");
    expect(schema.properties.map((def) => def.key)).toEqual(["l1.color", "l2.intensity", "l2.palette"]);
    expect(schema.properties[0]?.label).toBe("Sunset · Color");
    expect(schema.properties[1]?.label).toBe("Heat · Intensity");
  });

  it("does not name a layer whose effect is gone", () => {
    const gone = [{ ...layers[1]!, effect: "html:gone" }];
    const schema = stackSchema("Gone", gone, schemas);
    expect(schema.properties).toEqual([]);
    expect(schema.description).toBe("1 layer: html:gone");
  });
});
