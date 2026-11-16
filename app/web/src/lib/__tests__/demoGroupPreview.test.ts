import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { demoGroupFrame } from "@/lib/demoGroupPreview";
import { patchSnap } from "@/lib/store";
import type { Scene } from "@/proto/Scene";
import type { ScenesResponse } from "@/proto/ScenesResponse";

// The demo's group preview (#378): the picture one group draws, in the daemon's own format.
// The daemon sends black for a group set to Off, nothing at all for a pattern, and nothing for
// a group the scene does not hold. The demo keeps the same rules.

const SCENE: Scene = {
  id: "gaming",
  name: "Gaming",
  rev: 1,
  groups: [
    {
      id: "g-one",
      name: "One",
      members: ["a#z0"],
      plays: { type: "effect", effect: "aurora", props: {} },
      mode: "flow_across",
      brightness: 100,
    },
    {
      id: "g-dim",
      name: "Dim",
      members: ["a#z0"],
      plays: { type: "effect", effect: "aurora", props: {} },
      mode: "flow_across",
      brightness: 50,
    },
    {
      id: "g-off",
      name: "Off",
      members: [],
      plays: { type: "off" },
      mode: "flow_across",
      brightness: 100,
    },
    {
      id: "g-pattern",
      name: "Pattern",
      members: [],
      plays: { type: "effect", effect: "pattern:Intake", props: {} },
      mode: "flow_across",
      brightness: 100,
    },
  ],
};

const ANSWER: ScenesResponse = {
  active: "gaming",
  scenes: [SCENE],
  group_states: [],
};

const brightest = (rgb: Uint8Array): number => {
  let top = 0;
  for (const v of rgb) if (v > top) top = v;
  return top;
};

beforeEach(() => {
  patchSnap({ scenes: ANSWER });
});

afterEach(() => {
  patchSnap({ scenes: null });
});

describe("the demo's group preview", () => {
  it("sends the group's own picture, at the size the daemon sends", () => {
    const rgb = demoGroupFrame("g-one", 0);
    expect(rgb).not.toBeNull();
    expect(rgb!.length).toBe(80 * 50 * 3);
    expect(brightest(rgb!)).toBeGreaterThan(0);
  });

  it("has the group's brightness on it, as the LEDs get it", () => {
    const full = demoGroupFrame("g-one", 0)!;
    const half = demoGroupFrame("g-dim", 0)!;
    expect(brightest(half)).toBeLessThan(brightest(full));
    expect(brightest(half)).toBeGreaterThan(0);
  });

  it("sends black for a group set to Off", () => {
    const rgb = demoGroupFrame("g-off", 0)!;
    expect(rgb.length).toBe(80 * 50 * 3);
    expect(brightest(rgb)).toBe(0);
  });

  it("sends nothing for a pattern: it is drawn on the lights, not on a canvas", () => {
    expect(demoGroupFrame("g-pattern", 0)).toBeNull();
  });

  it("sends nothing for a group the scene does not hold", () => {
    expect(demoGroupFrame("g-nowhere", 0)).toBeNull();
  });
});
