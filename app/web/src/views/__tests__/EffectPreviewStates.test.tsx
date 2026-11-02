import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { demoHash } from "@/lib/demo";
import { SAMPLE_SOURCE } from "@/lib/demoEffects";
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

/** Replaces the whole document: select all, then paste over it. */
async function replaceSource(text: string) {
  const content = document.querySelector(".cm-content") as HTMLElement;
  content.focus();
  fireEvent.keyDown(content, { key: "a", code: "KeyA", keyCode: 65, ctrlKey: true });
  await act(async () => {
    fireEvent.paste(content, { clipboardData: { getData: () => text } });
  });
}

describe("the editor's live preview", () => {
  it("keeps the last good loop and marks the draft stale when the next render fails", async () => {
    setState("html,preview-late-error");
    await openEditor();
    // The first draft draws: no error list, no stale label.
    await screen.findByText("Preview");
    expect(screen.queryByText("Stale: last good render")).toBeNull();

    // A new draft fails the way a frame callback past its deadline does: its source is not
    // the one the loop on screen was drawn from.
    await replaceSource(`${SAMPLE_SOURCE}
<!-- a change that fails to draw -->
`);
    expect(await screen.findByText("Stale: last good render", {}, { timeout: 3000 })).toBeTruthy();
    expect(screen.getByText(/ran past its deadline/)).toBeTruthy();
    // The stale label says which render it is by the daemon's own source hash (D2.4).
    expect(screen.getByText("Stale: last good render").getAttribute("data-source")).toBe(
      demoHash(SAMPLE_SOURCE),
    );
    // The loop that was drawn is the loop on screen: it is still labelled as the preview.
    expect(screen.getByRole("img", { name: "Preview of Sample HTML" })).toBeTruthy();
    // And the error has its gutter marker (the API's line, not the message parsed).
    expect(document.querySelector(".cm-lint-marker-error")).toBeTruthy();
  });

  it("shows the check's warnings beside the preview with the code hidden", async () => {
    setState("html");
    await openEditor();
    // A draft with no frame request: the daemon's check says the picture never moves.
    await replaceSource(
      `<html><head><title>Still</title></head><body><canvas id="stage"></canvas></body>` +
        `<script>var ctx = document.getElementById("stage").getContext("2d");` +
        `ctx.fillStyle = "#f00"; ctx.fillRect(0, 0, 320, 200);</script></html>`,
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Hide code" }));
    });
    expect(await screen.findByText(/never moves/)).toBeTruthy();
    // The code pane is closed, so this is the copy beside the preview.
    expect(screen.queryByTestId("code-panel")).toBeNull();
    setState("html,preview-late-error");
  });

  it("waits Retry-After, retries exactly once, never spins, and offers Try again", async () => {
    setState("html,preview-busy");
    const calls: { at: number }[] = [];
    (globalThis as { __preview?: unknown[] }).__preview = calls;
    await openEditor();
    expect(await screen.findByText("Busy: the daemon is rendering other previews")).toBeTruthy();
    expect(calls.length).toBe(1);
    // The 503 said Retry-After: 1 s on the daemon (400 ms in the demo). Nothing may be sent
    // before that wait is over...
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(calls.length).toBe(1);
    // ...and exactly one retry follows it.
    await waitFor(() => expect(calls.length).toBe(2), { timeout: 2500 });
    const waited = (calls[1]?.at ?? 0) - (calls[0]?.at ?? 0);
    expect(waited).toBeGreaterThanOrEqual(380);
    expect(waited).toBeLessThan(1000);
    // A second busy answer stops there: a busy daemon is never polled.
    await new Promise((resolve) => setTimeout(resolve, 900));
    expect(calls.length).toBe(2);
    // The person asks again when they choose to.
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(calls.length).toBe(3), { timeout: 3000 });
    expect(await screen.findByText("Busy: the daemon is rendering other previews")).toBeTruthy();
  });
});
