import { describe, expect, it } from "vitest";
import { runningOf, setApplied } from "@/lib/runningPreset";
import type { EffectDetail } from "@/proto/EffectDetail";

const num = (value: number) => ({ type: "number", value }) as const;

function detail(props: EffectDetail["props"], id = "fx"): EffectDetail {
  return {
    schema: {
      id,
      name: "Fx",
      description: "",
      needs: { screen: false, audio: false },
      properties: [{ key: "speed", label: "Speed", kind: { type: "number", min: 0, max: 100, step: 1, default: 20 } }],
      presets: [{ name: "Ember", props: { speed: num(50) } }],
    },
    favorite: false,
    user_presets: [{ name: "Storm", props: { speed: num(60) } }],
    props,
  };
}

describe("runningOf", () => {
  it("is none for values that match no preset and were applied from none", () => {
    expect(runningOf(detail({ speed: num(33) }, "a"))).toBeNull();
  });

  it("names the built-in or user preset the values equal", () => {
    expect(runningOf(detail({ speed: num(50) }, "b"))).toEqual({ name: "Ember", modified: false });
    expect(runningOf(detail({ speed: num(60) }, "b"))).toEqual({ name: "Storm", modified: false });
  });

  it("marks the preset applied last as modified once the values moved off it", () => {
    setApplied("c", "Storm");
    expect(runningOf(detail({ speed: num(75) }, "c"))).toEqual({ name: "Storm", modified: true });
    expect(runningOf(detail({ speed: num(60) }, "c"))).toEqual({ name: "Storm", modified: false });
  });

  it("forgets an applied preset that was deleted", () => {
    setApplied("d", "Gone");
    expect(runningOf(detail({ speed: num(75) }, "d"))).toBeNull();
  });
});
