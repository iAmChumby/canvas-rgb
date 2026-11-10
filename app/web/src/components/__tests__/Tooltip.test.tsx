import { render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Tooltip } from "@/components/Tooltip";

const rect = (left: number, top: number, width: number, height: number) =>
  ({
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
    x: left,
    y: top,
    toJSON: () => ({}),
  }) as DOMRect;

const original = { width: document.documentElement.clientWidth, height: document.documentElement.clientHeight };

function viewport(width: number, height: number) {
  Object.defineProperty(document.documentElement, "clientWidth", { configurable: true, value: width });
  Object.defineProperty(document.documentElement, "clientHeight", { configurable: true, value: height });
}

afterEach(() => {
  viewport(original.width, original.height);
});

/** The tip as the trigger, its bubble, and the measured size jsdom cannot lay out. */
function show(trigger: DOMRect, bubbleWidth: number, bubbleHeight: number) {
  const { container } = render(
    <Tooltip label="Place Channel 1">
      <button type="button">+</button>
    </Tooltip>,
  );
  const tip = container.querySelector(".tip") as HTMLElement;
  const bubble = container.querySelector(".tip-bubble") as HTMLElement;
  vi.spyOn(tip, "getBoundingClientRect").mockReturnValue(trigger);
  vi.spyOn(bubble, "getBoundingClientRect").mockReturnValue(rect(0, 0, bubbleWidth, bubbleHeight));
  tip.dispatchEvent(new Event("pointerenter"));
  return { tip, bubble };
}

describe("Tooltip (#271)", () => {
  it("pulls the bubble inside the window when its key sits at the right edge", () => {
    viewport(1920, 1040);
    const { bubble } = show(rect(1851, 381.6, 28, 28), 119.4, 30.3);
    // 1920 - 8 - 119.4 = 1792.6, which is 58.4px left of the trigger.
    expect(Number.parseFloat(bubble.style.left)).toBeCloseTo(-58.4, 3);
    expect(Number.parseFloat(bubble.style.top)).toBeCloseTo(-36.3, 3);
    expect(bubble.style.right).toBe("auto");
    expect(bubble.style.transform).toBe("none");
  });

  it("shifts a bubble at the left edge inwards, not off the screen", () => {
    viewport(390, 844);
    const { bubble } = show(rect(0, 300, 24, 24), 100, 30);
    expect(Number.parseFloat(bubble.style.left)).toBe(8);
  });

  it("wraps a bubble the window has no room for and keeps it inside", () => {
    viewport(390, 844);
    // The Peripherals "Read again" label laid out on one line: 774.8px in a 390px window.
    const { bubble } = show(rect(129.5, 218.2, 116, 28), 774.8, 30.3);
    expect(bubble.style.maxWidth).toBe("374px");
    expect(bubble.style.whiteSpace).toBe("normal");
    expect(Number.parseFloat(bubble.style.left)).toBe(8 - 129.5);
  });
});
