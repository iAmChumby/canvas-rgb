import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

// A built-in preset that takes a name the owner's preset already has (D2.2, A8): the user key
// is marked shadowed, in the ordinary strip and in the editor.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=html,presets,shadowed");
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

async function mount() {
  await refreshAll();
  render(
    <ToastProvider>
      <ToastHost />
      <EffectsView />
    </ToastProvider>,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Open Sample HTML" }));
  await screen.findByRole("heading", { name: "Sample HTML" });
}

describe("a user preset a built-in preset shadows", () => {
  it("marks the owner's key in the strip", async () => {
    await mount();
    const key = await screen.findByRole("button", { name: "Apply preset Calm (shadowed)" });
    expect(key.textContent).toContain("Calm");
    expect(key.textContent).toContain("shadowed");
    // The built-in of the same name is its own key, and is not marked.
    const builtIn = screen.getByRole("button", { name: "Apply preset Calm" });
    expect(builtIn.textContent).not.toContain("shadowed");
  });

  it("marks the owner's key in the editor too", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Edit code" }));
    await screen.findByTestId("effect-code");
    const group = screen.getByRole("group", { name: "Presets" });
    const mine = within(group).getByRole("button", { name: "Try Calm on the draft (shadowed)" });
    expect(mine.textContent).toContain("shadowed");
    expect(within(group).getByRole("button", { name: "Try Calm on the draft" }).textContent).not.toContain(
      "shadowed",
    );
  });
});
