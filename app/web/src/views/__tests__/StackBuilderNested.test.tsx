import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// A nested stack (D95.4.8, D95.5.8): the row says Nested stack and the picked layer explains
// that a stack cannot contain one. The demo's ?state=stack-nested seeds the stack, so this state
// has its own file: the demo is seeded once per module.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Nested%20demo&state=stack-nested");
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  globalThis.IntersectionObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  } as unknown as typeof IntersectionObserver;
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
  Range.prototype.getClientRects = (() =>
    ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} }) as unknown as DOMRectList) as typeof Range.prototype.getClientRects;
  Range.prototype.getBoundingClientRect = (() =>
    ({ x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) }) as DOMRect) as typeof Range.prototype.getBoundingClientRect;
});

afterEach(cleanup);

function layerNames(): string[] {
  return screen
    .getAllByRole("listitem")
    .map((row) => row.querySelector(".layer-name")?.textContent ?? "")
    .filter(Boolean);
}

describe("a stack inside a stack", () => {
  it("says Nested stack in the row, and the picked layer explains it", async () => {
    window.history.replaceState({}, "", "/effects/?stack=Nested%20demo&state=complete,stack,stack-nested");
    await refreshAll();
    render(
      <>
        <NotifyHost />
        <StripSlotsProvider>
          <div className="strip-slots">
            <StripSlot kind="title" />
            <StripSlot kind="actions" />
          </div>
          <EffectsView />
        </StripSlotsProvider>
      </>,
    );
    await screen.findByTestId("stack-builder");
    await waitFor(() => expect(layerNames()).toEqual(["Sparkles", "Twin", "Base"]));
    expect(screen.getByText("Nested stack")).toBeTruthy();
    const row = screen.getByText("Twin").closest("li");
    expect(row).not.toBeNull();
    if (row) fireEvent.click(row);
    expect(await screen.findByText(/A stack cannot contain a stack/)).toBeTruthy();
  });
});
