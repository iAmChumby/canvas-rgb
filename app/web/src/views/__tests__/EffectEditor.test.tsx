import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { refreshAll } from "@/lib/store";
import { editorContent, editorSource, lintMarker } from "@/test/editor";
import { EffectsView } from "@/views/EffectsView";

// Edit mode of the Effects inspector (issue #2, D2.2) against the demo backend: `?state=html`
// puts the sample user effect in the library, and `source-error` makes its source throw where
// the runtime would stop, so the preview's error and its gutter are real.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=html,source-error");
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
  // jsdom has no layout: CodeMirror measures text with a Range, and a Range without client
  // rects throws on the first measure. Zero rects are enough for the DOM to render.
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
  // The pane keeps the previous effect's detail until the new one lands; its Edit code is
  // disabled while it does. Wait for the loaded effect's own heading.
  await screen.findByRole("heading", { name: "Sample HTML" });
  const edit = screen.getByRole("button", { name: "Edit code" }) as HTMLButtonElement;
  expect(edit.disabled).toBe(false);
  fireEvent.click(edit);
  await screen.findByTestId("effect-editor");
  await screen.findByTestId("effect-code");
}

// The source as the editor holds it (awaited: the editor mounts after the host div).
async function source() {
  return editorSource();
}

/** Types by replacing the whole document: select all, then paste over it. */
async function replaceSource(text: string) {
  const content = await editorContent();
  content.focus();
  fireEvent.keyDown(content, { key: "a", code: "KeyA", keyCode: 65, ctrlKey: true });
  await act(async () => {
    fireEvent.paste(content, { clipboardData: { getData: () => text } });
  });
}

describe("the effect editor", () => {
  it("opens the effect's source in the code pane and closes with Done", async () => {
    await mount();
    expect(await source()).toContain("<title>Sample HTML</title>");
    expect(screen.getAllByText("fixture-sample.html").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByTestId("effect-editor")).toBeNull();
    expect(await screen.findByRole("button", { name: "Edit code" })).toBeTruthy();
  });

  it("asks Save or Discard before leaving unsaved edits", async () => {
    await mount();
    await replaceSource(`${await source()}\n<!-- an edit that was not saved -->\n`);
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(await screen.findByRole("alertdialog", { name: "Unsaved changes" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Keep editing" }));
    expect(screen.getByTestId("effect-editor")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    fireEvent.click(await screen.findByRole("button", { name: "Discard" }));
    expect(screen.queryByTestId("effect-editor")).toBeNull();
  });

  it("marks the daemon's error line in the gutter and its list", async () => {
    await mount();
    // The demo's source-error throws inside the draw callback; the preview answers with the
    // runtime's script-relative line, and the editor adds the script tag's line.
    const row = await screen.findByRole("button", { name: /line \d+/ });
    const shown = Number(/line (\d+)/.exec(row.textContent ?? "")?.[1]);
    const throwLine = (await source())
      .split("\n")
      .findIndex((line) => line.includes("throw new Error")) + 1;
    expect(shown).toBe(throwLine);
    expect(await lintMarker()).toBeTruthy();
    expect(row.textContent).toContain("threw where the runtime stopped");
  });
});
