import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// "New effect" (#257 phase 4, spec §5): the empty state is the stage with the library open, a
// faint "Drag an effect here" outline, and three starter stacks. A starter lands whole, as one
// undo step.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=new&state=stack");
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

async function openNew() {
  window.history.replaceState({}, "", "/effects/?stack=new&state=complete,stack");
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
  await screen.findByRole("heading", { name: "New stack" });
}

describe("the empty builder", () => {
  it("shows the stage's drop outline, the library and the three starters", async () => {
    await openNew();
    expect(screen.getByText("Drag an effect here")).toBeTruthy();
    expect(screen.getByText("Start with a stack")).toBeTruthy();
    // The library is open beside the empty stage.
    expect(await screen.findByRole("button", { name: "Add Solid" })).toBeTruthy();
    for (const name of ["Sunset glow", "Warm flame", "Sparkle sky"]) {
      expect(screen.getByRole("button", { name: `Add the ${name} starter stack` })).toBeTruthy();
    }
    // Three tiles, each named and sized.
    const starters = screen.getByRole("region", { name: "Start with a stack" });
    expect(within(starters).getAllByRole("button")).toHaveLength(3);
  });

  it("adds a starter's whole stack in one click, and one undo takes it all back", async () => {
    await openNew();
    const starter = screen.getByRole("button", { name: "Add the Sunset glow starter stack" });
    fireEvent.click(starter);
    await waitFor(() => expect(layerNames()).toEqual(["Sparkles", "Heat", "Sunset"]));
    // One gesture, one step: a single undo leaves the builder empty again.
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(layerNames()).toHaveLength(0));
    expect(screen.getByText("Drag an effect here")).toBeTruthy();
  });
});
