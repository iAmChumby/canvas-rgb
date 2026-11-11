import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { CommandPalette, type Command } from "@/components/CommandPalette";

// The command palette over the phone's layer sheet (#232 review): both are dialogs with their own
// focus trap, and the sheet's trap must not pull the keyboard out of the palette, nor close
// itself when the palette does. No daemon and no socket: the demo backend answers every route,
// and the phone query answers from one flag like StackBuilderPhone.test.tsx.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let phoneWidth = true;
const phoneListeners = new Set<() => void>();

// The originals go back afterwards: this file does not leak a no-op scrollIntoView into the suite.
const originalMatchMedia = window.matchMedia;
const originalScrollIntoView = Element.prototype.scrollIntoView;

beforeAll(() => {
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

afterAll(() => {
  window.matchMedia = originalMatchMedia;
  Element.prototype.scrollIntoView = originalScrollIntoView;
});

afterEach(() => {
  phoneWidth = true;
  cleanup();
});

function paletteCommands(): Command[] {
  return [
    { id: "page-home", group: "Pages", label: "Go to Layout", hint: "/", run: () => {} },
    { id: "theme", group: "Appearance", label: "Switch the theme", run: () => {} },
  ];
}

/** The page and the palette as siblings, the palette after the page, as the shell renders them. */
function Harness() {
  const [paletteOpen, setPaletteOpen] = useState(true);
  return (
    <>
      <NotifyHost />
      <StripSlotsProvider>
        <div className="strip-slots">
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </div>
        <EffectsView />
        {paletteOpen && <CommandPalette commands={paletteCommands()} onClose={() => setPaletteOpen(false)} />}
      </StripSlotsProvider>
    </>
  );
}

async function renderWithOpenSheet() {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
  await refreshAll();
  render(<Harness />);
  await screen.findByTestId("stack-builder");
  // The demo backend answers after a first refresh: wait for the layers before picking one.
  await waitFor(() => expect(screen.queryAllByRole("listitem").length).toBeGreaterThan(0));
  // Pick a layer and open its sheet from Tune.
  const row = screen.getByText("Heat").closest("li");
  expect(row).not.toBeNull();
  if (row) fireEvent.click(row);
  const tune = screen.getByRole("button", { name: "Tune Heat" });
  tune.focus();
  fireEvent.click(tune);
  const sheet = await screen.findByRole("dialog", { name: "Tune Heat" });
  expect(sheet).toBeTruthy();
  return sheet;
}

describe("the command palette over the phone layer sheet", () => {
  it("keeps Tab and Shift+Tab inside the palette while the sheet is open behind it", async () => {
    const sheet = await renderWithOpenSheet();
    const palette = screen.getByRole("dialog", { name: "Command palette" });
    const field = screen.getByRole("combobox", { name: "Search commands" });
    field.focus();

    // Tab from the field: the palette owns the key, the sheet does not get to pull focus back.
    fireEvent.keyDown(field, { key: "Tab" });
    expect(palette.contains(document.activeElement)).toBe(true);
    expect(sheet.contains(document.activeElement)).toBe(false);

    // Shift+Tab from the first key: still inside the palette, not pulled into the sheet.
    field.focus();
    fireEvent.keyDown(field, { key: "Tab", shiftKey: true });
    expect(palette.contains(document.activeElement)).toBe(true);
    expect(sheet.contains(document.activeElement)).toBe(false);
  });

  it("closes only the palette on Escape, leaving the sheet open", async () => {
    const sheet = await renderWithOpenSheet();
    const field = screen.getByRole("combobox", { name: "Search commands" });
    field.focus();

    fireEvent.keyDown(field, { key: "Escape" });

    expect(screen.queryByRole("dialog", { name: "Command palette" })).toBeNull();
    // The sheet is the same dialog node it was before Escape, still open.
    expect(screen.getByRole("dialog", { name: "Tune Heat" })).toBe(sheet);
  });
});
