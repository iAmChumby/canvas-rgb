import { act, cleanup, render } from "@testing-library/react";
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
    // The Peripherals "Read again" label laid out on one line, measured in the static export at
    // 1920x1040 (headless Chromium, the label in the bubble's own 14px face): 853 px of text,
    // 871 px with the bubble's 8 px side padding and 1 px border. In a 390 px window that is
    // wider than the window, so the bubble has to wrap.
    const { bubble } = show(rect(129.5, 218.2, 116, 28), 871, 30.3);
    expect(bubble.style.maxWidth).toBe("374px");
    expect(bubble.style.whiteSpace).toBe("normal");
    expect(Number.parseFloat(bubble.style.left)).toBe(8 - 129.5);
  });

  it("places a wrapped bubble by its wrapped height, measured after the wrap", () => {
    viewport(390, 844);
    const { container } = render(
      <Tooltip label="Reads the settings from the mouse again, to pick up changes made on the mouse itself. The mouse may stutter briefly while it reads.">
        <button type="button">Read again</button>
      </Tooltip>,
    );
    const tip = container.querySelector(".tip") as HTMLElement;
    const bubble = container.querySelector(".tip-bubble") as HTMLElement;
    vi.spyOn(tip, "getBoundingClientRect").mockReturnValue(rect(129.5, 218.2, 116, 28));
    // The same measured 871 px on one line (30.3 px tall: the 20.3 px line and the padding);
    // three lines of 374 px once the component lets it wrap.
    vi.spyOn(bubble, "getBoundingClientRect").mockImplementation(() =>
      bubble.style.whiteSpace === "normal" ? rect(0, 0, 374, 90) : rect(0, 0, 871, 30.3),
    );
    tip.dispatchEvent(new Event("pointerenter"));
    // Above the trigger by the gap and the wrapped height: 218.2 - 6 - 90 = 122.2, so -96 inside the tip.
    expect(Number.parseFloat(bubble.style.top)).toBeCloseTo(-96, 3);
  });

  it("moves again once per frame on scroll, and lets go of the window on leave and on unmount", () => {
    viewport(1920, 1040);
    // The earlier tests leave their bubbles shown and listening; this one counts only its own.
    cleanup();
    const frames: FrameRequestCallback[] = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => frames.push(cb));
    const add = vi.spyOn(window, "addEventListener");
    const remove = vi.spyOn(window, "removeEventListener");
    const { container, unmount } = render(
      <Tooltip label="Place Channel 1">
        <button type="button">+</button>
      </Tooltip>,
    );
    const tip = container.querySelector(".tip") as HTMLElement;
    const bubble = container.querySelector(".tip-bubble") as HTMLElement;
    const trigger = vi.spyOn(tip, "getBoundingClientRect").mockReturnValue(rect(1851, 381.6, 28, 28));
    vi.spyOn(bubble, "getBoundingClientRect").mockReturnValue(rect(0, 0, 119.4, 30.3));
    tip.dispatchEvent(new Event("pointerenter"));
    expect(add.mock.calls.filter(([t]) => t === "scroll" || t === "resize")).toHaveLength(2);

    // Three scrolls in one frame are one placement, run when the frame comes.
    const before = trigger.mock.calls.length;
    const queued = frames.length;
    for (let i = 0; i < 3; i++) window.dispatchEvent(new Event("scroll"));
    expect(frames.length - queued).toBe(1);
    expect(trigger.mock.calls.length).toBe(before);
    frames[queued]?.(0);
    expect(trigger.mock.calls.length).toBe(before + 1);

    tip.dispatchEvent(new Event("pointerleave"));
    const removed = () => remove.mock.calls.filter(([t]) => t === "scroll" || t === "resize").length;
    expect(removed()).toBe(2);
    // Shown again, then unmounted while shown: the window listeners still come off.
    tip.dispatchEvent(new Event("pointerenter"));
    unmount();
    expect(removed()).toBe(4);
    vi.restoreAllMocks();
  });
});

describe("the bubble and the press (#384)", () => {
  /** The tip as the trigger, its bubble and the mark that holds the bubble back. */
  function tipOf(props: { holdOnPress?: boolean }) {
    const { container } = render(
      <Tooltip label="Move Case fans" {...props}>
        <button type="button">Move</button>
      </Tooltip>,
    );
    return container.querySelector(".tip") as HTMLElement;
  }

  /** A press on the control: the mark on the tip is set in the state it holds. */
  function press(tip: HTMLElement) {
    act(() => {
      tip.dispatchEvent(new Event("pointerdown"));
    });
  }

  it("a press on a normal control does not take its bubble away", () => {
    const tip = tipOf({});
    press(tip);
    // The bubble stays: a press on a button is not a drag over the page.
    expect(tip.getAttribute("data-hold")).toBeNull();
  });

  it("a control that asks for it holds its bubble back while it is pressed", () => {
    const tip = tipOf({ holdOnPress: true });
    press(tip);
    expect(tip.getAttribute("data-hold")).toBe("true");
    // The pointer lets go and leaves the control: the bubble comes back.
    act(() => {
      window.dispatchEvent(new Event("pointerup"));
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 600, clientY: 600 }));
    });
    expect(tip.getAttribute("data-hold")).toBeNull();
  });

  it("a press on a control that does not ask for it leaves the bubble alone, and so does a second press", () => {
    const tip = tipOf({});
    press(tip);
    act(() => {
      window.dispatchEvent(new Event("pointerup"));
    });
    press(tip);
    expect(tip.getAttribute("data-hold")).toBeNull();
  });
});
