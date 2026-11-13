import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll, refreshStatus } from "@/lib/store";
import { waitForStackGuard } from "@/test/historyGuard";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The stack builder on a phone, 390 x 844 (D95.5.7, PR D): one column, the picked layer as a
// one-line summary with Tune, the layer sheet over the builder, its focus trap, and the keys
// that close it. No daemon and no socket: the demo backend answers every route.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** The window the tests can widen: the phone query answers from one flag, and the app is told. */
let phoneWidth = true;
const phoneListeners = new Set<() => void>();
function setPhoneWidth(on: boolean) {
  phoneWidth = on;
  for (const listener of phoneListeners) listener();
}

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=stack");
  window.matchMedia = ((query: string) => ({
    get matches() {
      return query.includes("max-width: 720px") ? phoneWidth : false;
    },
    media: query,
    addEventListener: (_event: string, listener: () => void) => {
      if (query.includes("max-width: 720px")) phoneListeners.add(listener);
    },
    removeEventListener: (_event: string, listener: () => void) => {
      if (query.includes("max-width: 720px")) phoneListeners.delete(listener);
    },
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
  Element.prototype.setPointerCapture = (() => {}) as typeof Element.prototype.setPointerCapture;
  Element.prototype.releasePointerCapture = (() => {}) as typeof Element.prototype.releasePointerCapture;
  Element.prototype.hasPointerCapture = (() => false) as typeof Element.prototype.hasPointerCapture;
  Range.prototype.getClientRects = (() =>
    ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} }) as unknown as DOMRectList) as typeof Range.prototype.getClientRects;
  Range.prototype.getBoundingClientRect = (() =>
    ({ x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) }) as DOMRect) as typeof Range.prototype.getBoundingClientRect;
});

afterEach(() => {
  phoneWidth = true;
  cleanup();
});

type PreviewCall = { at: number; stack?: { layers: Array<{ id: string; props: Record<string, { value: unknown }> }> } };

const previewCalls = () => ((globalThis as { __stackPreview?: PreviewCall[] }).__stackPreview ??= []);

async function renderEffects(query: string) {
  window.history.replaceState({}, "", `/effects/${query}`);
  await refreshAll();
  return render(
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
}

function layerNames(): string[] {
  return screen
    .queryAllByRole("listitem")
    .map((row) => row.querySelector(".layer-name")?.textContent ?? "")
    .filter(Boolean);
}

async function openBuilder(query = "?stack=Sunset%20glow&state=complete,stack") {
  await renderEffects(query);
  await screen.findByTestId("stack-builder");
  await waitFor(() => expect(layerNames().length).toBeGreaterThan(0));
}

function pick(name: string) {
  const row = screen.getByText(name).closest("li");
  expect(row).not.toBeNull();
  if (row) fireEvent.click(row);
}

/** Adds from the library: on a phone it is the Library tab under the stage (spec §3). */
async function addFromLibrary(name: string) {
  const before = layerNames().length;
  fireEvent.click(screen.getByRole("radio", { name: "Library" }));
  fireEvent.click(await screen.findByRole("button", { name: `Add ${name}` }));
  fireEvent.click(screen.getByRole("radio", { name: "Layers" }));
  await waitFor(() => expect(layerNames()).toHaveLength(before + 1));
}

/** Focuses a control the way a person does before clicking it. */
function focusAndClick(control: HTMLElement) {
  control.focus();
  fireEvent.click(control);
}

describe("the stack builder on a phone", () => {
  it("is one column: the strip's keys, the picture, the list and the picked layer's summary", async () => {
    await openBuilder();
    // PR C's wider-window notice is gone.
    expect(screen.queryByText("Open on a wider window")).toBeNull();
    // The strip keeps one Save key, and its accessible name is its label.
    expect(screen.getByRole("button", { name: "Save" })).toBeTruthy();
    expect(screen.getByText("3 / 8")).toBeTruthy();
    // The picked layer is a one-line summary with Tune; the effect's name comes from the library
    // list, never from the raw id while its schema loads.
    expect(screen.getByRole("button", { name: "Tune Sparkles" })).toBeTruthy();
    expect(await screen.findByText("Rain · Whole canvas")).toBeTruthy();
    // The desktop column's Presets panel is not in the phone's column.
    expect(screen.queryByText("Presets")).toBeNull();
  });

  it("opens the layer sheet from Tune, holding the effect, blend, opacity, area and properties", async () => {
    await openBuilder();
    pick("Heat");
    focusAndClick(screen.getByRole("button", { name: "Tune Heat" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
    expect(within(sheet).getByText("Effect")).toBeTruthy();
    expect(within(sheet).getByText("Blend")).toBeTruthy();
    expect(within(sheet).getByRole("slider", { name: "Opacity" })).toBeTruthy();
    expect(within(sheet).getByRole("button", { name: "Change area" })).toBeTruthy();
    // Heat is Fire: its own controls are in the sheet, under the area row.
    expect(within(sheet).getByText("Intensity")).toBeTruthy();
    // Focus starts inside the sheet.
    expect(sheet.contains(document.activeElement)).toBe(true);
  });

  it("traps Tab inside the sheet while it is open", async () => {
    await openBuilder();
    pick("Heat");
    focusAndClick(screen.getByRole("button", { name: "Tune Heat" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
    const items = Array.from(
      sheet.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    );
    const first = items[0]!;
    const last = items[items.length - 1]!;
    expect(first.contains(document.activeElement) || first === document.activeElement).toBe(true);
    last.focus();
    fireEvent.keyDown(document.body, { key: "Tab" });
    expect(document.activeElement).toBe(first);
    first.focus();
    fireEvent.keyDown(document.body, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it("closes the sheet on Escape and puts focus back on Tune", async () => {
    await openBuilder();
    pick("Heat");
    const tune = screen.getByRole("button", { name: "Tune Heat" });
    focusAndClick(tune);
    await screen.findByRole("dialog", { name: "Tune Heat" });
    fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Tune Heat" })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(tune));
  });

  it("opens the area picker over the sheet and returns focus to Change area on Escape", async () => {
    await openBuilder();
    pick("Heat");
    focusAndClick(screen.getByRole("button", { name: "Tune Heat" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
    const change = within(sheet).getByRole("button", { name: "Change area" });
    focusAndClick(change);
    await screen.findByRole("dialog", { name: "Area for Heat" });
    fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Area for Heat" })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(change));
    // The sheet is still open behind it.
    expect(screen.getByRole("dialog", { name: "Tune Heat" })).toBeTruthy();
  });

  it("shows a held layer at 390, with the rest of the stack keeping its rate", async () => {
    await openBuilder("?stack=Sunset%20glow&state=complete,stack,stack-held");
    expect(screen.getByText(/Held: too slow/)).toBeTruthy();
    pick("Heat");
    focusAndClick(screen.getByRole("button", { name: "Tune Heat" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
    expect(within(sheet).getByText(/the rest of the stack keeps its rate/)).toBeTruthy();
  });

  it("shows eight layers as 8 / 8 with every tile disabled at 390", async () => {
    await openBuilder("?stack=Eight%20layers&state=complete,stack");
    expect(screen.getByText("8 / 8")).toBeTruthy();
    fireEvent.click(screen.getByRole("radio", { name: "Library" }));
    const add = screen.getByRole("button", { name: "Add Solid" });
    expect(add).toHaveProperty("disabled", true);
    expect(add.getAttribute("title")).toContain("A stack holds 8 layers");
  });

  it("runs a draft from the stage foot and stops it", async () => {
    await openBuilder();
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    expect(await screen.findByText("Draft on the lights")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(screen.queryByText("Draft on the lights")).toBeNull());
  });

  it("asks before leaving with unsaved changes at 390", async () => {
    await openBuilder();
    await addFromLibrary("Solid");
    // The in-app leave goes through the guard the dirty effect registers.
    await waitForStackGuard();
    fireEvent.click(screen.getByRole("button", { name: "Back to the effects" }));
    const prompt = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    expect(prompt).toBeTruthy();
    fireEvent.click(within(prompt).getByRole("button", { name: "Keep editing" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull());
  });

  it("reorders a layer with a finger held on its grip", async () => {
    await openBuilder();
    const rows = screen.getAllByRole("listitem");
    // jsdom has no layout: give each row the band it would have, top first.
    rows.forEach((row, i) => {
      row.getBoundingClientRect = () =>
        ({ x: 0, y: i * 50, top: i * 50, bottom: i * 50 + 50, left: 0, right: 300, width: 300, height: 50, toJSON: () => ({}) }) as DOMRect;
    });
    const sunset = screen.getByText("Sunset").closest("li")!;
    const grip = sunset.querySelector<HTMLButtonElement>(".layer-grip")!;
    act(() => {
      fireEvent.pointerDown(grip, { pointerType: "touch", pointerId: 1, clientX: 10, clientY: 120 });
    });
    // A finger lifts on the hold, not on the first move: before the hold the list still scrolls.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 320));
    });
    act(() => {
      fireEvent.pointerMove(window, { pointerType: "touch", pointerId: 1, clientX: 10, clientY: 10 });
      fireEvent.pointerUp(window, { pointerType: "touch", pointerId: 1, clientX: 10, clientY: 10 });
    });
    // Sunset moved past Sparkles and Heat to the top, in one drop.
    await waitFor(() => expect(layerNames()).toEqual(["Sunset", "Sparkles", "Heat"]));
    await waitFor(() =>
      expect(previewCalls().at(-1)?.stack?.layers.map((layer) => layer.id)).toEqual(["l2", "l3", "l1"]),
    );
  });

  it("keeps focus inside the area picker across the builder's status poll", async () => {
    await openBuilder();
    pick("Heat");
    focusAndClick(screen.getByRole("button", { name: "Tune Heat" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
    focusAndClick(within(sheet).getByRole("button", { name: "Change area" }));
    const picker = await screen.findByRole("dialog", { name: "Area for Heat" });
    const device = within(picker).getByRole("radio", { name: "Device" });
    device.focus();
    // The builder re-renders on each poll; the picker must not take the focus back.
    await act(async () => {
      await refreshStatus();
    });
    await act(async () => {
      await refreshStatus();
    });
    expect(document.activeElement).toBe(device);
  });

  it("returns focus to Tune on Escape after a tap that focused nothing", async () => {
    await openBuilder();
    pick("Heat");
    const tune = screen.getByRole("button", { name: "Tune Heat" });
    // A tap: the body is still the active element, and a click does not focus the key.
    (document.activeElement as HTMLElement | null)?.blur();
    fireEvent.click(tune);
    await screen.findByRole("dialog", { name: "Tune Heat" });
    fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Tune Heat" })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(tune));
  });

  it("pulls Tab into the sheet when the active element is outside it", async () => {
    await openBuilder();
    pick("Heat");
    focusAndClick(screen.getByRole("button", { name: "Tune Heat" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
    const save = screen.getByRole("button", { name: "Save" });
    save.focus();
    expect(document.activeElement).toBe(save);
    fireEvent.keyDown(save, { key: "Tab" });
    expect(sheet.contains(document.activeElement)).toBe(true);
  });

  it("opens the library tab when the last layer is deleted with its sheet open", async () => {
    await renderEffects("?stack=new&state=complete,stack");
    await screen.findByTestId("stack-builder");
    await addFromLibrary("Solid");
    focusAndClick(await screen.findByRole("button", { name: "Tune Solid" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Solid" });
    fireEvent.click(within(sheet).getByRole("button", { name: "Delete Solid" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Tune Solid" })).toBeNull());
    // Tune is unmounted with the layer: the library opens and its first tile takes the keyboard,
    // so the focus is never stranded on the body.
    await waitFor(() => expect(document.activeElement?.hasAttribute("data-lib-tile")).toBe(true));
  });

  it("traps Tab inside the area picker, at its last control and its first", async () => {
    await openBuilder();
    pick("Heat");
    focusAndClick(screen.getByRole("button", { name: "Tune Heat" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
    focusAndClick(within(sheet).getByRole("button", { name: "Change area" }));
    const picker = await screen.findByRole("dialog", { name: "Area for Heat" });
    const items = Array.from(picker.querySelectorAll<HTMLElement>("button:not([disabled]), [href]"));
    const first = items[0]!;
    const last = items[items.length - 1]!;
    last.focus();
    fireEvent.keyDown(document.body, { key: "Tab" });
    expect(document.activeElement).toBe(first);
    first.focus();
    fireEvent.keyDown(document.body, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it("focuses Keep editing, traps Tab in the prompt, and Escape goes back where focus was", async () => {
    await openBuilder();
    await addFromLibrary("Solid");
    const back = screen.getByRole("button", { name: "Back to the effects" });
    back.focus();
    fireEvent.click(back);
    const prompt = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    const keep = within(prompt).getByRole("button", { name: "Keep editing" });
    await waitFor(() => expect(document.activeElement).toBe(keep));
    // Tab stays inside the prompt: the last key wraps to the first.
    const items = Array.from(prompt.querySelectorAll<HTMLElement>("button:not([disabled])"));
    const first = items[0]!;
    const last = items[items.length - 1]!;
    last.focus();
    fireEvent.keyDown(document.body, { key: "Tab" });
    expect(document.activeElement).toBe(first);
    // Escape is Keep editing, and focus goes back to the key that opened the prompt.
    fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(back));
  });

  it("opens the leave prompt over the open sheet, and the sheet's trap yields", async () => {
    await openBuilder();
    await addFromLibrary("Solid");
    pick("Heat");
    focusAndClick(screen.getByRole("button", { name: "Tune Heat" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
    // A held page change asks via the same listener Back would hit; it must be installed first.
    await waitForStackGuard();
    // A held page change asks, with the sheet still under the prompt.
    act(() => {
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    const prompt = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    expect(sheet.getAttribute("data-open")).toBe("true");
    const keep = within(prompt).getByRole("button", { name: "Keep editing" });
    await waitFor(() => expect(document.activeElement).toBe(keep));
    // Tab from a control of the sheet comes into the prompt, not to the page behind it.
    const inSheet = sheet.querySelector<HTMLElement>("button")!;
    inSheet.focus();
    fireEvent.keyDown(inSheet, { key: "Tab" });
    expect(prompt.contains(document.activeElement)).toBe(true);
    fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("alertdialog", { name: "Unsaved changes" })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(inSheet));
    expect(screen.getByRole("dialog", { name: "Tune Heat" })).toBeTruthy();
  });

  it("moves focus to the picked row when the layout widens past the phone", async () => {
    await openBuilder();
    pick("Heat");
    focusAndClick(screen.getByRole("button", { name: "Tune Heat" }));
    const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
    const back = within(sheet).getByRole("button", { name: "Back to the stack" });
    back.focus();
    expect(document.activeElement).toBe(back);
    // The desktop layout has no Back key inside the sheet: focus goes to the picked row.
    act(() => setPhoneWidth(false));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Back to the stack" })).toBeNull());
    const row = document.querySelector<HTMLElement>('.rd-layers .layer-row[data-layer-id="l2"]');
    expect(document.activeElement).toBe(row);
  });
});
