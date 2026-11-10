import { act, cleanup, render } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { useMoreBelow } from "@/views/effects/useMoreBelow";

// The inspector's fold cue shows only while the pane has more below it (#257 phase 4, fix pass 1).
// jsdom has no layout, so the pane's sizes are set by hand; the cue's look is measured in a browser.

beforeAll(() => {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

afterEach(cleanup);

function Pane() {
  const ref = useRef<HTMLDivElement>(null);
  useMoreBelow(ref, "l1");
  return (
    <div ref={ref} data-testid="pane">
      <p>a section</p>
    </div>
  );
}

function sized(el: HTMLElement, sizes: { scrollHeight: number; clientHeight: number; scrollTop: number }) {
  for (const [key, value] of Object.entries(sizes)) {
    Object.defineProperty(el, key, { configurable: true, writable: true, value });
  }
}

describe("useMoreBelow", () => {
  it("says there is more while the end is below the fold, and not once it is in view", () => {
    const view = render(<Pane />);
    const pane = view.getByTestId("pane");
    sized(pane, { scrollHeight: 800, clientHeight: 500, scrollTop: 0 });
    act(() => void pane.dispatchEvent(new Event("scroll")));
    expect(pane.dataset.more).toBe("true");
    (pane as unknown as { scrollTop: number }).scrollTop = 150;
    act(() => void pane.dispatchEvent(new Event("scroll")));
    expect(pane.dataset.more).toBe("true");
    (pane as unknown as { scrollTop: number }).scrollTop = 299;
    act(() => void pane.dispatchEvent(new Event("scroll")));
    expect(pane.dataset.more).toBe("false");
  });

  it("says nothing is below when the pane's content fits", () => {
    const view = render(<Pane />);
    const pane = view.getByTestId("pane");
    sized(pane, { scrollHeight: 400, clientHeight: 400, scrollTop: 0 });
    act(() => void pane.dispatchEvent(new Event("scroll")));
    expect(pane.dataset.more).toBe("false");
  });
});
