import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { CaretLeft } from "@phosphor-icons/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import type { StackDetail } from "@/proto/StackDetail";

// Six things the "pure move" of StackBuilder.tsx (#257 phase 4, commit 9f3678d) changed. Each
// test names what the file did before the move; the move was to change nothing.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let phone = false;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: query.includes("max-width: 720px") ? phone : false,
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

afterEach(async () => {
  cleanup();
  phone = false;
  vi.restoreAllMocks();
  // A guard entry dropped with `history.back()` lands a turn later: let it land here.
  await new Promise((resolve) => setTimeout(resolve, 0));
});

async function open(query: string) {
  window.history.replaceState({}, "", `/effects/${query}`);
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
}

function layerNames(): string[] {
  return screen
    .getAllByRole("listitem")
    .map((row) => row.querySelector(".layer-name")?.textContent ?? "")
    .filter(Boolean);
}

function pick(name: string) {
  const label = [...document.querySelectorAll(".layer-name")].find((el) => el.textContent === name);
  const row = label?.closest("li");
  expect(row).not.toBeNull();
  if (row) fireEvent.click(row);
}

describe("Choose effect keeps its anchor (A1)", () => {
  it("places the popover under the button, links it, and Escape hands focus back to the button", async () => {
    await open("?stack=Sunset%20glow&state=complete,stack,stack-missing");
    await waitFor(() => expect(layerNames()).toContain("Sparkles"));
    pick("Sparkles");
    const button = await screen.findByRole("button", { name: "Choose effect for Sparkles" });
    fireEvent.click(button);
    const panel = await waitFor(() => {
      const found = document.querySelector<HTMLElement>(".rd-choose-effect");
      expect(found).not.toBeNull();
      return found as HTMLElement;
    });
    // The popover was placed against its anchor (a panel without an anchor never gets a left).
    await waitFor(() => expect(panel.style.left).not.toBe(""));
    // The anchor (the span around the key, as before the move) names the panel it opened.
    expect(button.parentElement?.getAttribute("aria-controls")).toBe(panel.id);
    // Escape from inside the list returns focus to the key that opened it.
    const row = panel.querySelector<HTMLElement>(".rd-add-row");
    expect(row).not.toBeNull();
    row?.focus();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(document.activeElement).toBe(button));
  });
});

describe("the daemon never gets a rect past the canvas (A2)", () => {
  it("sends a rect at x=315, width 10 clamped, on the preview and on Try on the lights", async () => {
    window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
    await refreshAll();
    const stored: StackDetail = await api.stack("Sunset glow");
    const past: StackDetail = {
      ...stored,
      stack: {
        ...stored.stack,
        layers: stored.stack.layers.map((layer) =>
          layer.name === "Heat" ? { ...layer, area: { kind: "rect", x: 315, y: 150, width: 10, height: 46 } } : layer,
        ),
      },
    };
    vi.spyOn(api, "stack").mockResolvedValue(past);
    const preview = vi.spyOn(api, "putStackPreview");
    const draft = vi.spyOn(api, "putDraft");
    await open("?stack=Sunset%20glow&state=complete,stack");
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    const heat = (layers: Array<{ name: string; area: unknown }> | undefined) =>
      layers?.find((layer) => layer.name === "Heat")?.area;
    const clamped = { kind: "rect", x: 315, y: 150, width: 5, height: 46 };
    await waitFor(() => expect(preview).toHaveBeenCalled());
    for (const [body] of preview.mock.calls) expect(heat(body.stack?.layers)).toEqual(clamped);

    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    await waitFor(() => expect(draft).toHaveBeenCalled());
    for (const [body] of draft.mock.calls) expect(heat(body.stack?.layers)).toEqual(clamped);
  });
});

describe("the mask amount's typed value is a step of its own (A4)", () => {
  it("two typed commits are two undo steps, as the number field's were", async () => {
    await open("?stack=Warm%20flame&state=complete,stack,html");
    const amount = () => screen.getByRole("slider", { name: "Mask amount" }).getAttribute("aria-valuenow");
    await waitFor(() => expect(amount()).toBe("100"));
    const type = (value: string) => {
      fireEvent.click(screen.getByRole("button", { name: /^Edit Mask amount/ }));
      const field = screen.getByRole("textbox", { name: "Mask amount value" });
      fireEvent.change(field, { target: { value } });
      fireEvent.keyDown(field, { key: "Enter" });
    };
    type("60");
    await waitFor(() => expect(amount()).toBe("60"));
    type("30");
    await waitFor(() => expect(amount()).toBe("30"));
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    await waitFor(() => expect(amount()).toBe("60"));
  });
});

describe("the phone sheet's back key (A5)", () => {
  it("draws a CaretLeft at 18, as it did before the move", async () => {
    phone = true;
    await open("?stack=Sunset%20glow&state=complete,stack");
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    const sheet = document.querySelector<HTMLElement>(".rd-layer-sheet");
    expect(sheet).not.toBeNull();
    const icon = within(sheet as HTMLElement).getByRole("button", { hidden: true, name: "Back to the stack" }).querySelector("svg");
    expect(icon).not.toBeNull();
    const caret = render(<CaretLeft size={18} />).container.querySelector("svg");
    expect(icon?.innerHTML).toBe(caret?.innerHTML);
    expect(icon?.getAttribute("width")).toBe("18");
  });
});

describe("the grip's key reads the live lift (sweep of the move)", () => {
  it("ignores Alt+Arrow in the turn a Space lifted the row, as the ref-reading handler did", async () => {
    await open("?stack=Sunset%20glow&state=complete,stack");
    await waitFor(() => expect(layerNames()).toHaveLength(3));
    const before = layerNames();
    const grip = screen.getByRole("button", { name: "Reorder Heat" });
    // One turn, no render between the two keys: only the ref knows the row is lifted.
    act(() => {
      fireEvent.keyDown(grip, { key: " " });
      fireEvent.keyDown(grip, { key: "ArrowUp", altKey: true });
    });
    expect(layerNames()).toEqual(before);
  });
});

describe("Escape in the name dialog (A3)", () => {
  /** A new stack with one layer, asked to leave: Save opens the name dialog, owing a leave. */
  async function nameDialogOwingALeave() {
    await open("?stack=new&state=complete,stack");
    fireEvent.click(await screen.findByRole("button", { name: "Add Solid" }));
    await waitFor(() => expect(layerNames()).toHaveLength(1));
    fireEvent.click(await screen.findByRole("button", { name: "Back to the effects" }));
    const prompt = await screen.findByRole("alertdialog", { name: "Unsaved changes" });
    fireEvent.click(within(prompt).getByRole("button", { name: "Save" }));
    return await screen.findByRole("dialog", { name: "Name the stack" });
  }

  /** Name the stack from the strip's Save, and say what became of the builder. */
  async function saveNamedFromTheStrip(name: string) {
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    const dialog = await screen.findByRole("dialog", { name: "Name the stack" });
    const field = within(dialog).getByRole("textbox", { name: "Stack name" });
    fireEvent.change(field, { target: { value: name } });
    fireEvent.keyDown(field, { key: "Enter" });
  }

  it("Escape closes the dialog and leaves the owed leave as it was, as before the move", async () => {
    const dialog = await nameDialogOwingALeave();
    fireEvent.keyDown(within(dialog).getByRole("textbox", { name: "Stack name" }), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Name the stack" })).toBeNull());
    // Before the move only Close and Cancel dropped the owed leave. Escape kept it, so the next
    // name that is saved carries the leave out.
    await saveNamedFromTheStrip("Zq test stack");
    await waitFor(() => expect(screen.queryByTestId("stack-builder")).toBeNull());
  });

  it("the Close key still drops the owed leave", async () => {
    const dialog = await nameDialogOwingALeave();
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Name the stack" })).toBeNull());
    await saveNamedFromTheStrip("Zq other stack");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Name the stack" })).toBeNull());
    expect(screen.getByTestId("stack-builder")).toBeTruthy();
  });
});
