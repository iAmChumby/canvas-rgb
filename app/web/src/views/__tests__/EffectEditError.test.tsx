import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

// A source that cannot be read (D2.2): the panel says so, another effect drops it, and Try
// again asks for the effect that failed rather than whatever is selected later.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=html,stock,source-fail");
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

const setState = (tokens: string) =>
  window.history.replaceState({}, "", `/effects/?state=${tokens}`);

async function mount() {
  await refreshAll();
  render(
    <ToastProvider>
      <ToastHost />
      <EffectsView />
    </ToastProvider>,
  );
  await screen.findByRole("button", { name: "Open Solid" });
}

async function open(name: "Sample HTML" | "Ink Drop") {
  fireEvent.click(screen.getByRole("button", { name: `Open ${name}` }));
  await screen.findByRole("heading", { name });
}

describe("a source that cannot be read", () => {
  it("says so, and picking another effect drops the panel", async () => {
    setState("html,stock,source-fail");
    await mount();
    await open("Sample HTML");
    fireEvent.click(screen.getByRole("button", { name: "Edit code" }));
    expect(await screen.findByText("The code could not be read")).toBeTruthy();
    // The gallery is not inert here: this is the error pane, not the editor.
    fireEvent.click(screen.getByRole("button", { name: "Open Ink Drop" }));
    await screen.findByRole("heading", { name: "Ink Drop" });
    await waitFor(() => expect(screen.queryByText("The code could not be read")).toBeNull());
    // And the effect it was about opens when the file can be read again.
    setState("html,stock");
    fireEvent.click(screen.getByRole("button", { name: "Edit code" }));
    await screen.findByTestId("effect-code");
    expect(screen.getByRole("heading", { name: "Ink Drop" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(screen.queryByTestId("effect-editor")).toBeNull());
  });

  it("retries the effect that failed, not the one selected later", async () => {
    setState("html,stock,source-fail");
    await mount();
    await open("Sample HTML");
    fireEvent.click(screen.getByRole("button", { name: "Edit code" }));
    expect(await screen.findByText("The code could not be read")).toBeTruthy();
    // The file can be read again: Try again asks for the failed effect itself.
    setState("html,stock");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    });
    await screen.findByTestId("effect-code");
    expect(screen.getByRole("heading", { name: "Sample HTML" })).toBeTruthy();
  });
});
