// The builder's stage (#257 phase 1) against the demo backend: every layer's frame is drawn, a
// press picks the smallest one under it, Tab and Escape walk the pick, Delete removes the picked
// layer, Ctrl+D copies it, the Make-rectangle key converts an area by name, and undo and redo
// take a whole drag back in one press.

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

vi.mock("@/lib/stackPreview", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/stackPreview")>();
  return {
    ...actual,
    onStackPreviewFeed: () => () => {},
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=stack");
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

afterEach(async () => {
  cleanup();
  vi.restoreAllMocks();
  // A change undone back to the saved stack drops the builder's guard entry with
  // `history.back()`, which lands a turn later: let it land before the next test sets its URL.
  await new Promise((resolve) => setTimeout(resolve, 0));
});

/** The stage is mocked at 640 x 400 css pixels: one canvas unit is two pixels. */
const K = 2;
const px = (u: number) => u * K;

async function openBuilder() {
  await refreshAll();
  // The URL is set after the read and just before the mount: a guard entry popped by the
  // previous test lands during the read, and it would otherwise win over this one.
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
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
  await screen.findByRole("heading", { name: "Sunset glow" });
  await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(3));
  const stage = document.querySelector<HTMLElement>(".rd-canvas")!;
  vi.spyOn(stage, "getBoundingClientRect").mockReturnValue({
    x: 0, y: 0, left: 0, top: 0, right: 640, bottom: 400, width: 640, height: 400, toJSON: () => ({}),
  } as DOMRect);
  return stage;
}

const row = (name: string) => screen.getByText(name).closest("li") as HTMLElement;
const pickedName = () => {
  const panel = document.querySelector(".rd-layer-edit");
  if (!panel || panel.classList.contains("rd-layer-none")) return null;
  return panel.querySelector("h2")?.textContent ?? null;
};
const readout = () => document.querySelector(".layer-area-text")?.textContent ?? "";
const frameOf = (id: string) => document.querySelector<HTMLElement>(`.rd-layer-frame[data-layer-id="${id}"]`);

const press = (stage: HTMLElement, x: number, y: number) => {
  act(() => {
    fireEvent.pointerDown(stage, { pointerId: 1, pointerType: "mouse", button: 0, clientX: x, clientY: y });
    fireEvent.pointerUp(stage, { pointerId: 1, pointerType: "mouse", buttons: 0, clientX: x, clientY: y });
  });
};

const mouse = { pointerId: 1, pointerType: "mouse", buttons: 1 };

function dragOn(el: HTMLElement, from: [number, number], to: [number, number]) {
  act(() => {
    fireEvent.pointerDown(el, { ...mouse, clientX: from[0], clientY: from[1] });
    fireEvent.pointerMove(el, { ...mouse, clientX: (from[0] + to[0]) / 2, clientY: (from[1] + to[1]) / 2 });
    fireEvent.pointerMove(el, { ...mouse, clientX: to[0], clientY: to[1] });
    fireEvent.pointerUp(el, { ...mouse, buttons: 0, clientX: to[0], clientY: to[1] });
  });
}

describe("the stage", () => {
  it("draws a frame for every visible layer, and only the picked one is the solid frame", async () => {
    await openBuilder();
    expect(frameOf("l1")).toBeTruthy();
    expect(frameOf("l2")).toBeTruthy();
    expect(frameOf("l3")).toBeTruthy();
    // The builder opens on the top layer (Sparkles); only its frame is the solid one.
    expect(frameOf("l3")?.classList.contains("rd-area-frame")).toBe(true);
    expect(frameOf("l1")?.classList.contains("rd-area-frame")).toBe(false);
    expect(document.querySelectorAll(".rd-area-frame")).toHaveLength(1);
  });

  it("picks the smallest frame under the press", async () => {
    const stage = await openBuilder();
    // The middle of Heat's 110 x 46 rectangle: inside it and inside both canvas layers.
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    // A point outside Heat's box: the whole-canvas layers tie, and the top one wins.
    press(stage, px(200), px(40));
    await waitFor(() => expect(pickedName()).toBe("Sparkles"));
  });

  it("walks the layers with Tab and Shift+Tab, top first", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    fireEvent.keyDown(stage, { key: "Tab" });
    await waitFor(() => expect(pickedName()).toBe("Sunset"));
    fireEvent.keyDown(stage, { key: "Tab", shiftKey: true });
    await waitFor(() => expect(pickedName()).toBe("Heat"));
  });

  it("clears the pick with Escape", async () => {
    const stage = await openBuilder();
    fireEvent.keyDown(stage, { key: "Escape" });
    await waitFor(() => expect(pickedName()).toBeNull());
    // With nothing picked, Tab is the browser's again: the stage does not cycle.
    fireEvent.keyDown(stage, { key: "Tab" });
    expect(pickedName()).toBeNull();
  });

  it("removes the picked layer with Delete, and brings it back with undo", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    fireEvent.keyDown(stage, { key: "Delete" });
    await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(2));
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(3));
    expect(screen.getByText("Heat")).toBeTruthy();
  });

  it("copies the picked layer with Ctrl+D", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(pickedName()).toBe("Heat"));
    fireEvent.keyDown(stage, { key: "d", ctrlKey: true });
    await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(4));
    expect(pickedName()).toBe("Heat copy");
  });

  it("takes a whole drag back with one undo, and forward with redo", async () => {
    const stage = await openBuilder();
    press(stage, px(71), px(173));
    await waitFor(() => expect(readout()).toContain("x 16, y 150"));
    const frame = document.querySelector<HTMLElement>(".rd-area-frame")!;
    dragOn(frame, [px(71), px(173)], [px(71) + 40, px(173) - 20]);
    await waitFor(() => expect(readout()).toContain("x 36, y 140"));
    // One undo puts the whole drag back, not one frame of it.
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    await waitFor(() => expect(readout()).toContain("x 16, y 150"));
    expect(screen.queryByText("Unsaved")).toBeNull();
    // Redo takes it forward again.
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(readout()).toContain("x 36, y 140"));
    // Ctrl+Y is the other redo; with nothing to redo it changes nothing.
    fireEvent.keyDown(document.body, { key: "y", ctrlKey: true });
    expect(readout()).toContain("x 36, y 140");
  });

  it("keeps Ctrl+Z for the text field it was typed in", async () => {
    await openBuilder();
    // The handler is on the window; any field on the page keeps its own undo.
    const input = document.createElement("input");
    document.body.append(input);
    try {
      expect(fireEvent.keyDown(input, { key: "z", ctrlKey: true })).toBe(true);
    } finally {
      input.remove();
    }
    expect(document.querySelectorAll(".layer-row")).toHaveLength(3);
  });

  it("offers Make rectangle on a whole-canvas area and converts it by name", async () => {
    const stage = await openBuilder();
    press(stage, px(200), px(40));
    await waitFor(() => expect(pickedName()).toBe("Sparkles"));
    const chip = screen.getByRole("button", { name: "Make Sparkles a rectangle" });
    fireEvent.click(chip);
    await waitFor(() => expect(readout()).toContain("Rectangle 320 × 200"));
    // The area is a rectangle now, so the key is gone and the frame carries handles.
    expect(screen.queryByRole("button", { name: "Make Sparkles a rectangle" })).toBeNull();
    expect(document.querySelector(".rd-area-frame")?.getAttribute("data-editable")).toBe("true");
    // Undo puts the whole-canvas area back.
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    await waitFor(() => expect(readout()).toContain("Whole canvas"));
    expect(screen.getByRole("button", { name: "Make Sparkles a rectangle" })).toBeTruthy();
  });

  it("shows and hides the desk's outlines with Parts", async () => {
    await openBuilder();
    const toggle = screen.getByRole("switch", { name: "Show the desk's device and zone outlines" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(document.querySelectorAll(".rd-part-outline").length).toBeGreaterThan(0);
    fireEvent.click(toggle);
    await waitFor(() => expect(document.querySelectorAll(".rd-part-outline")).toHaveLength(0));
  });

  it("removes a layer from its row's Delete key too", async () => {
    await openBuilder();
    fireEvent.keyDown(row("Heat"), { key: "Delete" });
    await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(2));
  });
});
