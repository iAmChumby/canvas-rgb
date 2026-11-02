import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

// The live preview (D2.4) against the demo backend. `preview-late-error` draws the first draft
// and fails every later one the way a frame callback past its deadline does; `preview-busy`
// answers every preview 503 with Retry-After, as two full render slots do.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=html,preview-late-error");
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

async function openEditor() {
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
  await screen.findByTestId("effect-code");
}

/** Moves the preview's Speed control, which asks the daemon for a new render. */
async function moveSpeed(to: string) {
  const speed = screen.getByLabelText("Speed value");
  fireEvent.focus(speed);
  fireEvent.change(speed, { target: { value: to } });
  await act(async () => {
    fireEvent.blur(speed);
  });
}

describe("the editor's live preview", () => {
  it("keeps the last good loop and marks the draft stale when the next render fails", async () => {
    setState("html,preview-late-error");
    await openEditor();
    // The first draft draws: no error list, no stale label.
    await screen.findByText("Preview");
    expect(screen.queryByText("Stale: last good render")).toBeNull();

    await moveSpeed("55");
    // 400 ms after the last change the daemon answers with the runtime's error.
    expect(await screen.findByText("Stale: last good render", {}, { timeout: 3000 })).toBeTruthy();
    expect(screen.getByText(/ran past its deadline/)).toBeTruthy();
    // The loop that was drawn is the loop on screen: it is still labelled as the preview.
    expect(screen.getByRole("img", { name: "Preview of Sample HTML" })).toBeTruthy();
    // And the error has its gutter marker (the API's line, not the message parsed).
    expect(document.querySelector(".cm-lint-marker-error")).toBeTruthy();
  });

  it("waits out a busy daemon once, never spins, and offers Try again", async () => {
    setState("html,preview-busy");
    const calls: unknown[] = [];
    (globalThis as { __preview?: unknown[] }).__preview = calls;
    await openEditor();
    expect(await screen.findByText("Busy: the daemon is rendering other previews")).toBeTruthy();
    // One answer, then one retry after Retry-After (400 ms in the demo), and no more.
    await waitFor(() => expect(calls.length).toBe(2), { timeout: 3000 });
    await new Promise((resolve) => setTimeout(resolve, 900));
    expect(calls.length).toBe(2);
    // A busy daemon is never polled; the person asks again when they choose to.
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(calls.length).toBe(3), { timeout: 3000 });
    expect(await screen.findByText("Busy: the daemon is rendering other previews")).toBeTruthy();
  });
});
