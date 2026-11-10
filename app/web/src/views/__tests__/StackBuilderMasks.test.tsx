// Masks in the drag session (#257 phase 3): a mask tile dropped on a layer row, one dropped on
// the stage (the top-most layer under the pointer), one refused over bare picture with a hint
// beside the pointer, a replace that is one undo step, the chip's remove keys, and the keyboard
// lift that picks a row with Up and Down and drops on Enter.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { dismissNotifications, NotifyHost } from "@/components/Notify";
import { MASK_EMPTY_HINT } from "@/lib/dragSession";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

vi.mock("@/lib/stackPreview", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/stackPreview")>();
  return { ...actual, onStackPreviewFeed: () => () => {} };
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
  vi.useRealTimers();
  cleanup();
  vi.restoreAllMocks();
  act(() => dismissNotifications());
  await new Promise((resolve) => setTimeout(resolve, 0));
});

/** The stage is 640 x 400 css pixels at (0, 100): two pixels per canvas unit. */
const K = 2;
const px = (u: number) => u * K;
const atY = (u: number) => 100 + u * K;
const rowTop = (place: number) => 600 + place * 40;

async function openBuilder(stack = "Sunset glow") {
  await refreshAll();
  window.history.replaceState({}, "", `/effects/?stack=${encodeURIComponent(stack)}&state=complete,stack`);
  const view = render(
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
  await screen.findByRole("heading", { name: stack });
  await waitFor(() => expect(document.querySelectorAll(".layer-row").length).toBeGreaterThan(0));
  const stage = document.querySelector<HTMLElement>(".rd-canvas")!;
  vi.spyOn(stage, "getBoundingClientRect").mockReturnValue({
    x: 0, y: 0, left: 0, top: 100, right: 640, bottom: 500, width: 640, height: 400, toJSON: () => ({}),
  } as DOMRect);
  // The layer rows, as they sit on the page: drawn top first, 40 px each, starting at 600.
  [...document.querySelectorAll<HTMLElement>(".rd-layers .layer-row")].forEach((row, place) => {
    const top = rowTop(place);
    vi.spyOn(row, "getBoundingClientRect").mockReturnValue({
      x: 700, y: top, left: 700, top, right: 1000, bottom: top + 40, width: 300, height: 40, toJSON: () => ({}),
    } as DOMRect);
  });
  return { stage, view };
}

const rows = () => document.querySelectorAll(".layer-row");
const live = () => document.querySelector(".rd-live")?.textContent ?? "";
const chipIn = (id: string) => document.querySelector(`.layer-row[data-layer-id="${id}"] .rd-mask-chip`);
const chips = () => document.querySelectorAll(".rd-mask-chip");
const hint = () => document.querySelector(".rd-drag-hint")?.textContent ?? null;

const mouse = { pointerId: 7, pointerType: "mouse", buttons: 1 };

/** The Masks tab of the library: the six that read best as masks come first. */
function masksTab() {
  act(() => void fireEvent.click(screen.getByRole("radio", { name: "Masks" })));
  return screen.getByRole("button", { name: "Add Fire" });
}

/** A mouse press on a library tile that has travelled to `to`: the drag has lifted. */
function lift(tileName: string, to: { x: number; y: number }) {
  const tile = screen.getByRole("button", { name: `Add ${tileName}` });
  act(() => void fireEvent.pointerDown(tile, { ...mouse, clientX: 10, clientY: 700 }));
  act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: 16, clientY: 700 }));
  act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: to.x, clientY: to.y }));
  return tile;
}

function dropAt(to: { x: number; y: number }) {
  act(() => void fireEvent.pointerUp(window, { ...mouse, buttons: 0, clientX: to.x, clientY: to.y }));
}

/** A whole drag of a mask tile onto a point, dropped there. */
function dragMaskTo(to: { x: number; y: number }) {
  lift("Fire", to);
  dropAt(to);
}

describe("a mask dropped on a layer row", () => {
  it("puts the mask's thumb on that row, as one chip with a remove x", async () => {
    await openBuilder();
    masksTab();
    // Heat is the second row drawn; its canvas index is 1.
    lift("Fire", { x: 850, y: rowTop(1) + 20 });
    expect(document.querySelector<HTMLElement>('.layer-row[data-layer-id="l2"]')?.dataset.maskTarget).toBe("true");
    dropAt({ x: 850, y: rowTop(1) + 20 });
    await waitFor(() => expect(chipIn("l2")).not.toBeNull());
    expect(chips()).toHaveLength(1);
    const row = document.querySelector<HTMLElement>('.layer-row[data-layer-id="l2"]')!;
    expect(within(row).getByRole("button", { name: "Change the mask of Heat" })).toBeTruthy();
    expect(within(row).getByRole("button", { name: "Remove the mask from Heat" })).toBeTruthy();
    expect(live()).toBe("Fire is now the mask on Heat.");
    // Nothing else changed: still three layers, and the other rows have no chip.
    expect(rows()).toHaveLength(3);
  });
});

describe("a mask dropped on the stage", () => {
  it("goes to the top-most layer under the pointer", async () => {
    await openBuilder();
    masksTab();
    dragMaskTo({ x: px(160), y: atY(100) });
    // Sparkles is painted last, so it is the top-most layer under the picture.
    await waitFor(() => expect(chipIn("l3")).not.toBeNull());
    expect(chipIn("l1")).toBeNull();
    expect(chipIn("l2")).toBeNull();
    expect(live()).toBe("Fire is now the mask on Sparkles.");
  });

  it("outlines the layer the mask would land on while the pointer is over it", async () => {
    await openBuilder();
    masksTab();
    lift("Fire", { x: px(160), y: atY(100) });
    const target = document.querySelector<HTMLElement>('.rd-layer-frame[data-layer-id="l3"]');
    expect(target?.dataset.maskTarget).toBe("true");
    dropAt({ x: px(160), y: atY(100) });
    await waitFor(() => expect(chipIn("l3")).not.toBeNull());
  });
});

describe("a mask dropped on empty picture", () => {
  it("refuses, says why beside the pointer, and changes nothing", async () => {
    await openBuilder();
    // No visible layer means no frame under the pointer: every layer is hidden.
    for (const name of ["Sunset", "Heat", "Sparkles"]) {
      act(() => void fireEvent.click(screen.getByRole("button", { name: `Hide ${name}` })));
    }
    masksTab();
    lift("Fire", { x: px(160), y: atY(100) });
    expect(hint()).toBe(MASK_EMPTY_HINT);
    expect(live()).toBe(MASK_EMPTY_HINT);
    dropAt({ x: px(160), y: atY(100) });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(chips()).toHaveLength(0);
    expect(hint()).toBeNull();
    expect(rows()).toHaveLength(3);
  });
});

describe("a mask replace", () => {
  it("is one undo step: one Ctrl+Z brings the old mask back", async () => {
    await openBuilder("Warm flame");
    const chip = () => screen.getByRole("button", { name: "Change the mask of Warm white" });
    expect(chip().getAttribute("title")).toMatch(/^Mask: Fire\./);
    masksTab();
    const tile = screen.getByRole("button", { name: "Add Rain" });
    act(() => void fireEvent.pointerDown(tile, { ...mouse, clientX: 10, clientY: 700 }));
    act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: 16, clientY: 700 }));
    act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: 850, clientY: rowTop(0) + 20 }));
    dropAt({ x: 850, y: rowTop(0) + 20 });
    await waitFor(() => expect(chip().getAttribute("title")).toMatch(/^Mask: Rain\./));

    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(chip().getAttribute("title")).toMatch(/^Mask: Fire\./));
    expect(rows()).toHaveLength(1);
  });
});

describe("removing a mask", () => {
  it("removes it with the chip's x", async () => {
    await openBuilder("Warm flame");
    const remove = screen.getByRole("button", { name: "Remove the mask from Warm white" });
    // The x is the mask chip's own key, beside the mask's thumb on the row.
    expect(remove.closest(".rd-mask-chip")).not.toBeNull();
    act(() => void fireEvent.click(remove));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Change the mask of Warm white" })).toBeNull());
    expect(chips()).toHaveLength(0);
    // The row offers Add mask again.
    expect(screen.getByRole("button", { name: "Add a mask to Warm white" })).toBeTruthy();
  });

  it("removes it with Backspace on the focused chip", async () => {
    await openBuilder("Warm flame");
    const thumb = screen.getByRole("button", { name: "Change the mask of Warm white" });
    act(() => {
      thumb.focus();
      fireEvent.keyDown(thumb, { key: "Backspace" });
    });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Change the mask of Warm white" })).toBeNull());
    expect(chips()).toHaveLength(0);
  });
});

describe("a keyboard lift of a mask tile", () => {
  it("picks a row with Up and Down and drops it on Enter", async () => {
    await openBuilder();
    const tile = masksTab();
    act(() => {
      tile.focus();
      fireEvent.keyDown(tile, { key: " " });
    });
    expect(live()).toMatch(/^Lifted Fire as a mask\./);
    act(() => void fireEvent.keyUp(tile, { key: " " }));
    fireEvent.click(tile);
    // Up walks the drawn list from the top row, where the lift starts.
    act(() => void fireEvent.keyDown(window, { key: "ArrowUp" }));
    act(() => void fireEvent.keyDown(window, { key: "Enter" }));
    await waitFor(() => expect(chips()).toHaveLength(1));
    expect(live()).toMatch(/^Fire is now the mask on /);
    // The Enter that dropped did not also reach the focused tile and add a layer.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(rows()).toHaveLength(3);
  });
});
