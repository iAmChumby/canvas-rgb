import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

// New from code (D2.3) against the demo backend: `?state=html,stock` puts the sample user effect
// and the two stock HTML effects in the library, so both starting points are real files.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=html,stock");
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

const source = () =>
  Array.from(document.querySelectorAll(".cm-line"))
    .map((line) => line.textContent ?? "")
    .join("\n");

async function mount() {
  await refreshAll();
  render(
    <ToastProvider>
      <ToastHost />
      {/* More is a page action: it lives in the status strip, so the strip is here. */}
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>
    </ToastProvider>,
  );
  await screen.findByRole("button", { name: "Open Solid" });
}

const setState = (tokens: string) =>
  window.history.replaceState({}, "", `/effects/?state=${tokens}`);

const newEffect = async () => {
  // The code editor's way in is advanced, behind the page's overflow menu.
  fireEvent.click(screen.getByRole("button", { name: "More effect actions" }));
  fireEvent.click(await screen.findByRole("button", { name: "New from code" }));
  return screen.findByTestId("new-effect");
};

describe("a new effect", () => {
  it("starts from a copy, keeps its licence line, and saves under a new name", async () => {
    await mount();
    const chooser = await newEffect();
    // Built-ins are not offered: only the HTML effects are.
    expect(chooser.textContent).toContain("Ink Drop");
    expect(chooser.textContent).toContain("Wet Smear");
    expect(chooser.textContent).not.toContain("Solid");
    fireEvent.click(within(chooser).getByRole("button", { name: "Copy Ink Drop" }));
    await screen.findByTestId("effect-code");
    const field = screen.getByLabelText("File name") as HTMLInputElement;
    expect(field.value).toBe("ink-drop copy.html");
    expect(source()).toContain("MIT licence");
    expect(source()).toContain("<title>Ink Drop copy</title>");
    // Saving reads the file name, imports it, and the library shows the copy.
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Ink Drop copy is running", {}, { timeout: 3000 })).toBeTruthy();
    expect(await screen.findByRole("button", { name: "Open Ink Drop copy" })).toBeTruthy();
  });

  it("starts blank from the minimal file and saves it as a new effect", async () => {
    await mount();
    await newEffect();
    fireEvent.click(screen.getByRole("button", { name: "Blank effect" }));
    await screen.findByTestId("effect-code");
    const field = screen.getByLabelText("File name") as HTMLInputElement;
    expect(field.value).toBe("my-effect.html");
    expect(source()).toContain("<title>My effect</title>");
    expect(source()).toContain("<canvas id=\"stage\"");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("My effect is running", {}, { timeout: 3000 })).toBeTruthy();
    expect(await screen.findByRole("button", { name: "Open My effect" })).toBeTruthy();
  });

  it("asks before replacing a file that exists", async () => {
    await mount();
    await newEffect();
    fireEvent.click(screen.getByRole("button", { name: "Blank effect" }));
    await screen.findByTestId("effect-code");
    const field = screen.getByLabelText("File name") as HTMLInputElement;
    fireEvent.change(field, { target: { value: "ink-drop.html" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    // The import route's replace prompt, labelled so it is not the preset conflict.
    expect(await screen.findByText("file exists")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Replace" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await act(async () => undefined);
    expect(screen.queryByText("file exists")).toBeNull();
    // Nothing was written: the blank is still the file it was.
    expect(screen.getByLabelText("File name")).toBeTruthy();
  });

  it("writes a new file without replace, asks on the route's clash, then replaces", async () => {
    await mount();
    // Another window saved the name after this one read the list: the list does not have it,
    // and the import route refuses a write that did not ask to replace.
    setState("html,stock,import-clash");
    const imports = ((globalThis as { __imports?: { file_name: string; replace: boolean }[] }).__imports ??= []);
    imports.length = 0;
    await newEffect();
    fireEvent.click(screen.getByRole("button", { name: "Blank effect" }));
    await screen.findByTestId("effect-code");
    const field = screen.getByLabelText("File name") as HTMLInputElement;
    fireEvent.change(field, { target: { value: "race.html" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("file exists")).toBeTruthy();
    expect(imports.map((call) => call.replace)).toEqual([false]);
    fireEvent.click(screen.getByRole("button", { name: "Replace" }));
    expect(await screen.findByText("My effect is running", {}, { timeout: 3000 })).toBeTruthy();
    expect(imports.map((call) => call.replace)).toEqual([false, true]);
    expect(imports.at(-1)?.file_name).toBe("race.html");
    setState("html,stock");
  });

  it("says why a copy could not be read and stays in the chooser", async () => {
    await mount();
    setState("html,stock,source-fail");
    const chooser = await newEffect();
    fireEvent.click(within(chooser).getByRole("button", { name: "Copy Ink Drop" }));
    expect(await screen.findByText(/could not read the effect file/)).toBeTruthy();
    expect(screen.getByTestId("new-effect")).toBeTruthy();
    setState("html,stock");
  });

  it("says it is reading a copy while the file is on its way", async () => {
    await mount();
    setState("html,stock,slow");
    const chooser = await newEffect();
    fireEvent.click(within(chooser).getByRole("button", { name: "Copy Wet Smear" }));
    expect(await screen.findByText("Reading…")).toBeTruthy();
    await screen.findByTestId("effect-code", {}, { timeout: 5000 });
    setState("html,stock");
  });
});
