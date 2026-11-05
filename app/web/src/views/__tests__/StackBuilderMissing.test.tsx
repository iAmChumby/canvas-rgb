import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// A layer whose effect is gone (D95.5.8), on a phone: the row says Missing, and the layer sheet
// says it is gone and offers Choose effect. The demo's ?state=stack-missing seeds the stack, so
// this state has its own file: the demo is seeded once per module.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=stack-missing");
  window.matchMedia = ((query: string) => ({
    matches: query.includes("max-width: 720px"),
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

async function open() {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack,stack-missing");
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
  await waitFor(() => expect(screen.getAllByRole("listitem").length).toBeGreaterThan(0));
}

describe("a layer whose effect is gone", () => {
  it("says Missing in its row, and the phone's sheet says it is gone and offers Choose effect", async () => {
    await open();
    expect(screen.getByText("Missing")).toBeTruthy();
    const tune = screen.getByRole("button", { name: "Tune Sparkles" });
    tune.focus();
    fireEvent.click(tune);
    const sheet = await screen.findByRole("dialog", { name: "Tune Sparkles" });
    expect(within(sheet).getByText("This effect is gone. Choose another.")).toBeTruthy();
    expect(within(sheet).getByRole("button", { name: "Choose effect for Sparkles" })).toBeTruthy();
  });
});
