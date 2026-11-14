// The drag session's loose ends (#257 phase 2, fix pass 1): a finger that scrolls is not a lift,
// a session never outlives its pointer, the page's unmount leaves nothing behind, a click after a
// drag or a key is not also an add, a library drop is one undo step, and the drop boxes are read
// at the lift rather than on every move.

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { dismissNotifications, NotifyHost } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { refreshAll, getSnap } from "@/lib/store";
import { wholeLightsCanvas } from "@/lib/groupCanvas";
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

/** The stage is mocked at 640 x 400 css pixels at (0, 100): two pixels per canvas unit. */
const K = 2;
const px = (u: number) => u * K;
const atY = (u: number) => 100 + u * K;

async function openBuilder() {
  await refreshAll();
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
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
  await screen.findByRole("heading", { name: "Sunset glow" });
  await waitFor(() => expect(document.querySelectorAll(".layer-row")).toHaveLength(3));
  const stage = document.querySelector<HTMLElement>(".rd-canvas")!;
  const rect = vi.spyOn(stage, "getBoundingClientRect").mockReturnValue({
    x: 0, y: 0, left: 0, top: 100, right: 640, bottom: 500, width: 640, height: 400, toJSON: () => ({}),
  } as DOMRect);
  return { stage, rect, unmount: view.unmount };
}

const rows = () => document.querySelectorAll(".layer-row");
const live = () => document.querySelector(".rd-live")?.textContent ?? "";
const ghost = () => document.querySelector(".rd-drag-ghost");
const tile = (name: string) => screen.getByRole("button", { name: `Add ${name}` });

const mouse = { pointerId: 7, pointerType: "mouse", buttons: 1 };
const finger = { pointerId: 9, pointerType: "touch", buttons: 1 };

/** A mouse press on Solid's tile that has travelled onto the picture: the drag is lifted. */
function liftOnStage() {
  const solid = tile("Solid");
  act(() => void fireEvent.pointerDown(solid, { ...mouse, clientX: 10, clientY: 700 }));
  act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: 16, clientY: 700 }));
  act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: px(200), clientY: atY(100) }));
  expect(ghost()).not.toBeNull();
  return solid;
}

/** A whole drag from the tile onto empty picture, dropped. */
function dragTileOntoStage() {
  const solid = tile("Solid");
  act(() => void fireEvent.pointerDown(solid, { ...mouse, clientX: 10, clientY: 700 }));
  act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: 16, clientY: 700 }));
  act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: px(200), clientY: atY(100) }));
  act(() => void fireEvent.pointerUp(window, { ...mouse, buttons: 0, clientX: px(200), clientY: atY(100) }));
}

describe("a finger that scrolls is not a lift", () => {
  it("cancels the hold when the finger moves past the slop before 300 ms", async () => {
    await openBuilder();
    const added = vi.spyOn(api, "effect");
    vi.useFakeTimers();
    const solid = tile("Solid");
    act(() => void fireEvent.pointerDown(solid, { ...finger, clientX: 10, clientY: 700 }));
    act(() => void vi.advanceTimersByTime(100));
    act(() => void fireEvent.pointerMove(window, { ...finger, clientX: 10, clientY: 740 }));
    act(() => void vi.advanceTimersByTime(300));
    expect(ghost()).toBeNull();
    expect(live()).toBe("");
    expect(added).not.toHaveBeenCalled();
  });

  it("still lifts on a hold that stays put", async () => {
    await openBuilder();
    vi.useFakeTimers();
    const solid = tile("Solid");
    act(() => void fireEvent.pointerDown(solid, { ...finger, clientX: 10, clientY: 700 }));
    act(() => void vi.advanceTimersByTime(100));
    act(() => void fireEvent.pointerMove(window, { ...finger, clientX: 11, clientY: 701 }));
    act(() => void vi.advanceTimersByTime(300));
    expect(ghost()).not.toBeNull();
  });
});

describe("Escape and the page scroll under a pointer drag (#257 review)", () => {
  it("ends a lifted mouse drag on Escape, and the release that follows adds nothing", async () => {
    await openBuilder();
    const added = vi.spyOn(api, "effect");
    liftOnStage();
    act(() => void fireEvent.keyDown(window, { key: "Escape" }));
    expect(ghost()).toBeNull();
    act(() => void fireEvent.pointerUp(window, { ...mouse, buttons: 0, clientX: px(200), clientY: atY(100) }));
    expect(rows()).toHaveLength(3);
    expect(added).not.toHaveBeenCalled();
  });

  it("lets a finger scroll the page until the hold lifts, then keeps the move for the drag", async () => {
    await openBuilder();
    vi.useFakeTimers();
    const solid = tile("Solid");
    const touchMove = () => {
      const e = new Event("touchmove", { bubbles: true, cancelable: true });
      window.dispatchEvent(e);
      return e.defaultPrevented;
    };
    act(() => void fireEvent.pointerDown(solid, { ...finger, clientX: 10, clientY: 700 }));
    expect(touchMove()).toBe(false);
    act(() => void vi.advanceTimersByTime(300));
    expect(ghost()).not.toBeNull();
    expect(touchMove()).toBe(true);
    act(() => void fireEvent.pointerUp(window, { ...finger, buttons: 0, clientX: 10, clientY: 700 }));
    expect(touchMove()).toBe(false);
  });
});

describe("a session never outlives its pointer", () => {
  const ends: [string, () => void][] = [
    ["a lost pointer capture", () => document.querySelector<HTMLElement>('[data-effect="solid"]')!.dispatchEvent(new Event("lostpointercapture"))],
    ["the window losing focus", () => void window.dispatchEvent(new Event("blur"))],
    [
      "the tab going to the background",
      () => {
        Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
        document.dispatchEvent(new Event("visibilitychange"));
      },
    ],
  ];

  for (const [name, end] of ends) {
    it(`ends on ${name}, and a new press, an undo and a keyboard lift all work again`, async () => {
      await openBuilder();
      liftOnStage();
      act(() => end());
      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
      expect(ghost()).toBeNull();
      expect(live()).toBe("Cancelled; nothing changed.");
      expect(rows()).toHaveLength(3);

      // A new press: the drag lands, and the layer is added.
      dragTileOntoStage();
      await waitFor(() => expect(rows()).toHaveLength(4));

      // Undo: the add goes in one step.
      act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
      await waitFor(() => expect(rows()).toHaveLength(3));

      // A keyboard lift: the session takes the key again.
      const solid = tile("Solid");
      act(() => void fireEvent.keyDown(solid, { key: " " }));
      expect(live()).toMatch(/^Lifted Solid\./);
    });
  }
});

describe("leaving the page", () => {
  it("ends a lifted press: a release after the unmount adds nothing and throws nothing", async () => {
    const { unmount } = await openBuilder();
    const added = vi.spyOn(api, "effect");
    liftOnStage();
    unmount();
    expect(() => {
      fireEvent.pointerUp(window, { ...mouse, buttons: 0, clientX: px(200), clientY: atY(100) });
    }).not.toThrow();
    // An add runs a turn later: wait for it before saying it did not come.
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(added).not.toHaveBeenCalled();
  });

  it("stops a finger's pending hold: it never lifts on a page that is gone", async () => {
    const { unmount } = await openBuilder();
    const capture = vi.fn();
    Element.prototype.setPointerCapture = capture as unknown as typeof Element.prototype.setPointerCapture;
    vi.useFakeTimers();
    const solid = tile("Solid");
    act(() => void fireEvent.pointerDown(solid, { ...finger, clientX: 10, clientY: 700 }));
    unmount();
    expect(() => {
      vi.advanceTimersByTime(500);
      fireEvent.pointerUp(window, { ...finger, buttons: 0, clientX: 10, clientY: 700 });
    }).not.toThrow();
    expect(capture).not.toHaveBeenCalled();
    delete (Element.prototype as { setPointerCapture?: unknown }).setPointerCapture;
  });
});

describe("a click is not also an add", () => {
  it("adds one layer for one drag, however late its click arrives", async () => {
    await openBuilder();
    const added = vi.spyOn(api, "effect");
    dragTileOntoStage();
    await waitFor(() => expect(rows()).toHaveLength(4));
    // The browser's click after the release; a touch screen sends it after the task has ended.
    await new Promise((resolve) => setTimeout(resolve, 30));
    fireEvent.click(tile("Solid"));
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(rows()).toHaveLength(4);
    expect(added).toHaveBeenCalledTimes(1);
  });

  it("does not add when Space lifts a tile: the keyup's click is swallowed, then Escape cancels", async () => {
    await openBuilder();
    const added = vi.spyOn(api, "effect");
    const solid = tile("Solid");
    solid.focus();
    act(() => void fireEvent.keyDown(solid, { key: " " }));
    expect(live()).toMatch(/^Lifted Solid\./);
    // A browser clicks a button on the keyup of Space.
    act(() => void fireEvent.keyUp(solid, { key: " " }));
    fireEvent.click(solid);
    act(() => void fireEvent.keyDown(solid, { key: "Escape" }));
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(live()).toBe("Cancelled; nothing changed.");
    expect(rows()).toHaveLength(3);
    expect(added).not.toHaveBeenCalled();
  });

  it("drops once on Enter, and the same Enter does not lift the tile again", async () => {
    await openBuilder();
    const solid = tile("Solid");
    solid.focus();
    act(() => void fireEvent.keyDown(solid, { key: " " }));
    act(() => void fireEvent.keyUp(solid, { key: " " }));
    fireEvent.click(solid);
    act(() => void fireEvent.keyDown(solid, { key: "ArrowDown" }));
    act(() => void fireEvent.keyDown(solid, { key: "Enter" }));
    await waitFor(() => expect(rows()).toHaveLength(4));
    // The drop is the last word: the Enter that dropped did not reach the tile and lift it again.
    expect(live()).toMatch(/^Solid added /);
    act(() => void fireEvent.keyUp(solid, { key: "Enter" }));
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(rows()).toHaveLength(4);
  });

  it("drops once on Space, and the click of its keyup adds nothing", async () => {
    await openBuilder();
    const solid = tile("Solid");
    solid.focus();
    act(() => void fireEvent.keyDown(solid, { key: " " }));
    act(() => void fireEvent.keyUp(solid, { key: " " }));
    fireEvent.click(solid);
    act(() => void fireEvent.keyDown(solid, { key: " " }));
    await waitFor(() => expect(rows()).toHaveLength(4));
    act(() => void fireEvent.keyUp(solid, { key: " " }));
    fireEvent.click(solid);
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(rows()).toHaveLength(4);
    expect(live()).toMatch(/^Solid added /);
  });
});

describe("a drop is one undo step", () => {
  it("puts the prior layer count back with one Ctrl+Z", async () => {
    await openBuilder();
    dragTileOntoStage();
    await waitFor(() => expect(rows()).toHaveLength(4));
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(rows()).toHaveLength(3));
  });
});

describe("the drop boxes", () => {
  it("are read at the lift and when the page scrolls or resizes, not on every move", async () => {
    const { rect } = await openBuilder();
    liftOnStage();
    const afterLift = rect.mock.calls.length;
    for (let step = 0; step < 12; step++) {
      act(() => void fireEvent.pointerMove(window, { ...mouse, clientX: px(100 + step), clientY: atY(60) }));
    }
    expect(rect.mock.calls.length).toBe(afterLift);
    act(() => void window.dispatchEvent(new Event("scroll")));
    expect(rect.mock.calls.length).toBeGreaterThan(afterLift);
    const afterScroll = rect.mock.calls.length;
    act(() => void window.dispatchEvent(new Event("resize")));
    expect(rect.mock.calls.length).toBeGreaterThan(afterScroll);
  });
});

describe("the stage while a drag runs", () => {
  it("outlines the target device and names it in a pill inside the picture", async () => {
    const { stage } = await openBuilder();
    const solid = tile("Solid");
    solid.focus();
    act(() => void fireEvent.keyDown(solid, { key: " " }));
    act(() => void fireEvent.keyDown(window, { key: "ArrowRight" }));
    const outline = stage.querySelector('.rd-drop-rect[data-target="device"]');
    expect(outline).not.toBeNull();
    const pill = stage.querySelector(".rd-drop-pill");
    const snap = getSnap();
    const first = wholeLightsCanvas(snap.lights ?? [], snap.devices ?? []).deviceIds[0];
    const name = snap.devices?.find((device) => device.id === first)?.name ?? "";
    expect(pill?.textContent).toBe(name);
    expect(stage.querySelector(".rd-drop-strip")).not.toBeNull();
    expect(stage.dataset.dropping).toBe("true");
    act(() => void fireEvent.keyDown(window, { key: "Escape" }));
    expect(stage.querySelector(".rd-drop-pill")).toBeNull();
    expect(stage.dataset.dropping).toBeUndefined();
  });
});
