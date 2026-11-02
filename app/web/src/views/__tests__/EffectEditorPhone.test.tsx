import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

// The editor at 390 px (D2.2): the code is a full-screen layer over the pushed sheet, closed on
// load, with Back, and the sheet's own Back still asks before unsaved edits are left.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=html");
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
  fireEvent.click(screen.getByRole("button", { name: "Edit code" }));
  await screen.findByTestId("effect-editor");
}

const layer = () => screen.queryByRole("dialog", { name: "The effect's code" });

describe("the editor at 390 px", () => {
  it("opens with the code closed, shows it as a layer, and Back returns to the effect", async () => {
    await mount();
    expect(layer()).toBeNull();
    expect(screen.getByRole("button", { name: "Show code" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Show code" }));
    expect(layer()).toBeTruthy();
    await screen.findByTestId("effect-code");
    fireEvent.click(screen.getByRole("button", { name: "Back to the effect" }));
    expect(layer()).toBeNull();
    // The editor is still there, with its preview and its actions. On a phone the way out is
    // the sheet's Back, so there is no separate Done to crowd the row.
    expect(screen.getByTestId("effect-editor")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try on the lights" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Done" })).toBeNull();
  });

  it("asks before the sheet's Back leaves unsaved edits", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Show code" }));
    const content = await screen.findByTestId("effect-code");
    const editor = content.querySelector(".cm-content") as HTMLElement;
    editor.focus();
    fireEvent.keyDown(editor, { key: "a", code: "KeyA", keyCode: 65, ctrlKey: true });
    await act(async () => {
      fireEvent.paste(editor, { clipboardData: { getData: () => "<html></html>" } });
    });
    fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    expect(await screen.findByRole("alertdialog", { name: "Unsaved changes" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(screen.queryByTestId("effect-editor")).toBeNull());
  });
});
