import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PatternTile } from "@/views/effects/PatternTile";
import type { Pattern } from "@/proto/Pattern";

// The library tile of a pattern: the device drawn as its dots, in the pattern's own colours - no
// canvas thumbnail, no request. The dot count and the colours are the whole picture the tile
// claims, so they are what is checked.

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const OFF = "#000000";

/** A ring of 12 with two LEDs on, as the demo's Intake pattern is. */
function ring(): Pattern {
  const colors = Array.from({ length: 12 }, (_, i) => (i === 0 ? "#ff2d55" : i === 7 ? "#00d8ff" : OFF));
  return {
    id: "pattern:Intake",
    built_on: { led_count: 12, arrangement: "ring" },
    frames: [{ colors, hold: 0 }],
    motion: { type: "none" },
  };
}

/** A strip of `leds` LEDs, the first three on. */
function strip(leds: number): Pattern {
  const colors = Array.from({ length: leds }, (_, i) => (i < 3 ? "#ff2d55" : OFF));
  return {
    id: "pattern:Underglow",
    built_on: { led_count: leds, arrangement: "strip" },
    frames: [{ colors, hold: 0 }],
    motion: { type: "none" },
  };
}

/** The dots drawn, in LED order. */
const dots = (root: HTMLElement): SVGCircleElement[] =>
  Array.from(root.querySelectorAll("circle")) as SVGCircleElement[];

const fillOf = (dot: SVGCircleElement): string | null => dot.getAttribute("fill");

describe("a pattern tile", () => {
  it("the pattern tile draws one dot per LED in the pattern's colours", () => {
    const { container } = render(<PatternTile pattern={ring()} size="md" />);
    const drawn = dots(container);
    // One dot per LED, and the two lit LEDs are drawn in their own colours.
    expect(drawn).toHaveLength(12);
    expect(fillOf(drawn[0]!)).toBe("#ff2d55");
    expect(fillOf(drawn[7]!)).toBe("#00d8ff");
    // An LED that is off is an unlit socket: the theme's own token, never a black dot.
    expect(fillOf(drawn[1]!)).toBe("var(--led-off)");
    // The tile says what it is, for a reader who cannot see the dots.
    expect(container.querySelector('[role="img"]')?.getAttribute("aria-label")).toBe(
      "Intake, ring of 12, 2 of 12 on",
    );
  });

  it("a long strip keeps one dot per LED", () => {
    const { container } = render(<PatternTile pattern={strip(300)} size="md" />);
    const drawn = dots(container);
    expect(drawn).toHaveLength(300);
    expect(fillOf(drawn[2]!)).toBe("#ff2d55");
    expect(fillOf(drawn[299]!)).toBe("var(--led-off)");
  });

  it("a pattern tile makes no thumbnail request", () => {
    const fetchSpy = vi.fn(() => Promise.reject(new Error("no network in a tile")));
    vi.stubGlobal("fetch", fetchSpy);
    const { container } = render(<PatternTile pattern={ring()} size="sm" />);
    expect(fetchSpy).not.toHaveBeenCalled();
    // Not the thumbnail component and not a canvas: the dots are the picture.
    expect(container.querySelector(".thumb")).toBeNull();
    expect(container.querySelector("canvas")).toBeNull();
  });
});
