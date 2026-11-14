// The library and the drag session (#257 phase 2) against the demo backend: a tile's click adds
// a layer, a Space lift and Enter drop places one, the live region says each step, the grip
// reorders by the same session, and a pointer drag from the tray onto the stage lands on the zone
// under the finger with Shift.

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { dismissNotifications, NotifyHost } from "@/components/Notify";
import { getSnap, refreshAll } from "@/lib/store";
import { lightsUnder, wholeLightsCanvas } from "@/lib/groupCanvas";
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
  cleanup();
  vi.restoreAllMocks();
  act(() => dismissNotifications());
  await new Promise((resolve) => setTimeout(resolve, 0));
});

/** The stage is mocked at 640 x 400 css pixels at (0, 100): two pixels per canvas unit. */
const K = 2;
const px = (u: number) => u * K;
const atY = (u: number) => 100 + u * K;

async function openBuilder() {
  await refreshAll();
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
    x: 0, y: 0, left: 0, top: 100, right: 640, bottom: 500, width: 640, height: 400, toJSON: () => ({}),
  } as DOMRect);
  return stage;
}

const rows = () => document.querySelectorAll(".layer-row");
const layerNames = () =>
  [...document.querySelectorAll(".layer-row")]
    .map((row) => row.querySelector(".layer-name")?.textContent ?? "")
    .filter(Boolean);
const live = () => document.querySelector(".rd-live")?.textContent ?? "";

/** The builder's canvas: every light, as one flow-across group. */
function builderCanvas() {
  const snap = getSnap();
  return wholeLightsCanvas(snap.lights ?? [], snap.devices ?? []);
}

/** The label of the light a canvas point belongs to. */
function labelAt(x: number, y: number): string {
  return lightsUnder(builderCanvas(), { x, y })[0]?.label ?? "";
}

/** The name of the first device in the group. */
function firstDeviceName(): string {
  const snap = getSnap();
  const id = builderCanvas().deviceIds[0];
  return snap.devices?.find((device) => device.id === id)?.name ?? "";
}
const tile = (name: string) => screen.getByRole("button", { name: `Add ${name}` });
const rowOf = (name: string) =>
  [...document.querySelectorAll<HTMLElement>(".layer-row")].find(
    (row) => row.querySelector(".layer-name")?.textContent === name,
  )!;

describe("the library", () => {
  it("adds a layer from a tile's click, on top and whole canvas", async () => {
    await openBuilder();
    fireEvent.click(tile("Solid"));
    await waitFor(() => expect(rows()).toHaveLength(4));
    expect(layerNames()[0]).toBe("Solid");
    expect(rowOf("Solid").querySelector(".layer-meta")?.textContent).toContain("Whole canvas");
  });

  it("is two tabs, and the masks list leads with the six that read best", async () => {
    await openBuilder();
    const first = () => document.querySelector<HTMLElement>(".rd-library [data-lib-tile]");
    expect(first()?.dataset.effect).toBeTruthy();
    fireEvent.click(screen.getByRole("radio", { name: "Masks" }));
    const order = [...document.querySelectorAll<HTMLElement>(".rd-library [data-lib-tile]")]
      .map((el) => el.dataset.effect)
      .filter((id) => id !== undefined)
      .slice(0, 6);
    expect(order).toEqual(["fire", "rain", "spiral", "aurora", "breathing", "gradient"]);
  });

  it("keeps the last tile for the code flow", async () => {
    await openBuilder();
    const tiles = [...document.querySelectorAll<HTMLElement>(".rd-library [data-lib-tile]")];
    expect(tiles.at(-1)?.textContent).toContain("New from code");
  });
});

describe("the keyboard sensor", () => {
  it("lifts a tile with Space, says each step, and adds on Enter at the device it names", async () => {
    await openBuilder();
    const solid = tile("Solid");
    solid.focus();
    act(() => void fireEvent.keyDown(solid, { key: " " }));
    expect(live()).toBe(
      "Lifted Solid. Left and Right pick where it goes, Up and Down pick a place in the list; Enter drops it, Escape cancels.",
    );
    // The first target is the whole canvas; the first device in the group is next.
    act(() => void fireEvent.keyDown(window, { key: "ArrowRight" }));
    expect(live()).toBe(`${firstDeviceName()}.`);
    act(() => void fireEvent.keyDown(window, { key: "Enter" }));
    await waitFor(() => expect(rows()).toHaveLength(4));
    expect(live()).toBe(`Solid added to ${firstDeviceName()}.`);
    expect(rowOf("Solid").querySelector(".layer-meta")?.textContent).toContain(firstDeviceName());
  });

  it("cancels on Escape and changes nothing", async () => {
    await openBuilder();
    const solid = tile("Solid");
    act(() => void fireEvent.keyDown(solid, { key: "Enter" }));
    act(() => void fireEvent.keyDown(window, { key: "Escape" }));
    expect(live()).toBe("Cancelled; nothing changed.");
    expect(rows()).toHaveLength(3);
  });

  it("reorders a row with the grip: Space, the arrows, Enter, and one undo", async () => {
    await openBuilder();
    const grip = rowOf("Sunset").querySelector<HTMLElement>(".layer-grip")!;
    act(() => void fireEvent.keyDown(grip, { key: " " }));
    expect(live()).toBe("Lifted Sunset. Up and Down pick a place in the list; Enter drops it, Escape cancels.");
    act(() => void fireEvent.keyDown(window, { key: "ArrowUp" }));
    expect(live()).toBe("Above Sunset.");
    act(() => void fireEvent.keyDown(window, { key: "ArrowUp" }));
    expect(live()).toBe("Above Heat.");
    act(() => void fireEvent.keyDown(window, { key: "ArrowUp" }));
    expect(live()).toBe("Top of the list.");
    act(() => void fireEvent.keyDown(window, { key: "Enter" }));
    await waitFor(() => expect(layerNames()).toEqual(["Sunset", "Sparkles", "Heat"]));
    expect(live()).toBe("Sunset moved to the top of the list.");
    // One drop is one undo step: one Ctrl+Z puts the whole move back.
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(layerNames()).toEqual(["Sparkles", "Heat", "Sunset"]));
  });
});

describe("the pointer sensor", () => {
  const mouse = { pointerId: 7, pointerType: "mouse", buttons: 1 };

  function dragTileTo(x: number, y: number, shiftKey = false) {
    const solid = tile("Solid");
    act(() => void fireEvent.pointerDown(solid, { ...mouse, clientX: 10, clientY: 700 }));
    // Past the slop first, as a real press is: the lift is the move, the target is the next one.
    act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: 16, clientY: 700 }));
    act(() =>
      void fireEvent.pointerMove(window, { ...mouse, clientX: x, clientY: y, shiftKey }),
    );
    act(() => void fireEvent.pointerUp(window, { ...mouse, buttons: 0, clientX: x, clientY: y, shiftKey }));
  }

  it("shows a ghost while the drag runs, and lands on the zone under it with Shift", async () => {
    await openBuilder();
    const solid = tile("Solid");
    act(() => void fireEvent.pointerDown(solid, { ...mouse, clientX: 10, clientY: 700 }));
    act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: px(71), clientY: atY(173), shiftKey: true }));
    // The ghost follows the pointer, and the live region names the zone.
    expect(document.querySelector(".rd-drag-ghost")?.textContent).toContain("Solid");
    await waitFor(() => expect(live()).toBe(`${labelAt(71, 173)}.`));
    act(() => void fireEvent.pointerUp(window, { ...mouse, buttons: 0, clientX: px(71), clientY: atY(173) }));
    await waitFor(() => expect(rows()).toHaveLength(4));
    expect(rowOf("Solid").querySelector(".layer-meta")?.textContent).toContain(labelAt(71, 173));
    expect(document.querySelector(".rd-drag-ghost")).toBeNull();
  });

  it("makes a new rectangle from the keyboard's own target", async () => {
    await openBuilder();
    const solid = tile("Solid");
    solid.focus();
    act(() => void fireEvent.keyDown(solid, { key: " " }));
    // Right cycles to the rectangle after the canvas and the group's devices; Left wraps to it.
    act(() => void fireEvent.keyDown(window, { key: "ArrowLeft" }));
    expect(live()).toBe("New rectangle.");
    act(() => void fireEvent.keyDown(window, { key: "Enter" }));
    await waitFor(() => expect(rows()).toHaveLength(4));
    const meta = rowOf("Solid").querySelector(".layer-meta")?.textContent ?? "";
    expect(meta).toContain("Rectangle 120 × 80");
  });

  it("adds nothing when a drag ends on the library", async () => {
    await openBuilder();
    dragTileTo(10, 700);
    await waitFor(() => expect(live()).toBe("Cancelled; nothing changed."));
    expect(rows()).toHaveLength(3);
  });
});
