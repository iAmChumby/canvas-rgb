// A stack whose mask effect is gone: the builder says so next to the mask Select (#256). The demo
// state is built once per page from the URL's `state`, so this file's own URL carries the
// `stack-gone-mask` scenario.

import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Gone%20mask&state=complete,stack,stack-gone-mask");
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

describe("a mask effect that is gone", () => {
  it("says so beside the mask Select", async () => {
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
    await screen.findByRole("heading", { name: "Gone mask" });
    await waitFor(() => expect(screen.getAllByRole("listitem").length).toBeGreaterThanOrEqual(1));

    const text = await screen.findByText(
      "This mask effect is gone. Choose another, or remove the mask.",
    );
    expect(text.closest(".rd-mask")).not.toBeNull();
  });
});
