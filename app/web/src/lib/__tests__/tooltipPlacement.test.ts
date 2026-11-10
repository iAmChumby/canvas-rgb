import { describe, expect, it } from "vitest";
import { boundsOf, placeTooltip, TIP_MARGIN, type TipPlacementInput } from "@/lib/tooltipPlacement";

const windowOf = (width: number, height: number) => boundsOf({ width, height });
const at = (left: number, top: number, width = 28, height = 28) => ({ left, top, width, height });

const place = (input: Partial<TipPlacementInput>): ReturnType<typeof placeTooltip> =>
  placeTooltip({
    ...(input as TipPlacementInput),
    gap: input.gap ?? 6,
    margin: input.margin ?? TIP_MARGIN,
    side: input.side ?? "top",
    align: input.align ?? "center",
  });

describe("placeTooltip (#271)", () => {
  it("leaves a bubble with room exactly where its CSS puts it", () => {
    // Nothing collides: the centred bubble above the trigger, unchanged.
    expect(
      place({
        trigger: at(700, 400),
        bubble: { width: 111, height: 30.3 },
        bounds: windowOf(1440, 900),
      }),
    ).toEqual({ side: "top", left: 658.5, top: 363.7 });
  });

  it("shifts a bubble left so its right end stays 8px inside the window", () => {
    // The Layout "Place Channel 1" case: a 28px key at the window's right edge.
    expect(
      place({
        trigger: at(1851, 381.6),
        bubble: { width: 119.4, height: 30.3 },
        bounds: windowOf(1920, 1040),
      }),
    ).toEqual({ side: "top", left: 1792.6, top: 345.3 });
  });

  it("shifts a bubble right so its left end stays 8px inside the window", () => {
    expect(
      place({
        trigger: at(0, 300, 20, 20),
        bubble: { width: 100, height: 30 },
        bounds: windowOf(1024, 768),
      }),
    ).toEqual({ side: "top", left: 8, top: 264 });
  });

  it("flips to the bottom when the top has no room", () => {
    expect(
      place({
        trigger: at(100, 4),
        bubble: { width: 100, height: 30 },
        bounds: windowOf(390, 844),
      }),
    ).toEqual({ side: "bottom", left: 64, top: 38 });
  });

  it("flips to the top when the bottom has no room", () => {
    expect(
      place({
        trigger: at(500, 740),
        bubble: { width: 100, height: 30 },
        bounds: windowOf(1024, 768),
        side: "bottom",
      }),
    ).toEqual({ side: "top", left: 464, top: 704 });
  });

  it("keeps the start edge inside a window too small for the bubble", () => {
    // 300x100 cannot fit a 200x60 window: the margin wins, and the far edge gives way.
    expect(
      place({
        trigger: at(10, 20, 20, 20),
        bubble: { width: 300, height: 100 },
        bounds: windowOf(200, 60),
      }),
    ).toEqual({ side: "top", left: 8, top: 8 });
  });

  it("keeps the alignment the CSS asks for, and only then shifts", () => {
    const endAligned = { trigger: at(1700, 300), bubble: { width: 119.4, height: 30.3 }, align: "end" as const };
    expect(place({ ...endAligned, bounds: windowOf(1920, 1040) })).toEqual({
      side: "top",
      left: 1608.6,
      top: 263.7,
    });
    // The same key at the edge: the end alignment still passes it, so the shift takes over.
    expect(place({ ...endAligned, trigger: at(1892, 300), bounds: windowOf(1920, 1040) })).toEqual({
      side: "top",
      left: 1792.6,
      top: 263.7,
    });
  });

  it("keeps a bubble beside its trigger on the right, and flips it left at the edge", () => {
    const bubble = { width: 200, height: 30 };
    expect(place({ trigger: at(100, 300), bubble, bounds: windowOf(1920, 1040), side: "right" })).toEqual({
      side: "right",
      left: 134,
      top: 299,
    });
    expect(place({ trigger: at(1890, 300), bubble, bounds: windowOf(1920, 1040), side: "right" })).toEqual({
      side: "left",
      left: 1684,
      top: 299,
    });
  });

  it("keeps a bubble inside the pane that clips it, not only the window", () => {
    const bounds = { left: 200, top: 55, width: 1700, height: 985 };
    expect(place({ trigger: at(1851, 381.6), bubble: { width: 119.4, height: 30.3 }, bounds })).toEqual({
      side: "top",
      left: 1772.6,
      top: 345.3,
    });
  });
});
