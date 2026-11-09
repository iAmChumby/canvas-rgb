// The demo's White colour matches the daemon's curve (#50).

import { describe, expect, it } from "vitest";
import { kelvinToLed, kelvinToRgb } from "@/lib/kelvin";
import { composeStack } from "@/lib/stackFrame";
import type { Stack } from "@/proto/Stack";

describe("kelvinToRgb", () => {
  it("matches the published fit the daemon is pinned to", () => {
    expect(kelvinToRgb(2700)).toEqual([255, 167, 87]);
    expect(kelvinToRgb(4000)).toEqual([255, 206, 166]);
  });

  it("keeps a blue floor at the warm end and clamps out-of-range values", () => {
    expect(kelvinToRgb(1800)[2]).toBe(31);
    expect(kelvinToRgb(500)).toEqual(kelvinToRgb(1800));
    expect(kelvinToRgb(Number.NaN)).toEqual(kelvinToRgb(1800));
    expect(kelvinToRgb(90000)).toEqual(kelvinToRgb(6500));
  });
});

describe("kelvinToLed", () => {
  it("decodes the display white to linear light, as the daemon sends it (#50)", () => {
    expect(kelvinToLed(1800)).toEqual([255, 54, 3]);
    expect(kelvinToLed(2700)).toEqual([255, 98, 25]);
  });
});

describe("a White layer in a demo stack", () => {
  it("draws one flat LED white of its temperature", () => {
    const stack: Stack = {
      id: "stack:test",
      layers: [
        {
          id: "l1",
          name: "White",
          effect: "white",
          props: { temperature: { type: "number", value: 2700 } },
          area: { kind: "canvas" },
          blend: "normal",
          opacity: 100,
          visible: true,
        },
      ],
    };
    const frame = composeStack(stack, null, 0.5, 16, 10);
    for (let i = 0; i < 16 * 10; i++) {
      expect([frame[i * 3], frame[i * 3 + 1], frame[i * 3 + 2]]).toEqual([255, 98, 25]);
    }
  });
});
