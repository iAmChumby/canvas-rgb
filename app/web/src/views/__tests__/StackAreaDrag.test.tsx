import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { rectFromDrag } from "@/views/effects/stackDraft";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The stack builder against the demo backend (D95.5): the seeded Sunset glow, its layers, the
// preview PUTs, Try on the lights and Save. No daemon, no socket: the demo answers every route.

// The preview feed's own problem (a second stack socket, refused by the daemon) arrives through
// this subscription; the test drives it directly.
const feed = vi.hoisted(() => ({ listener: null as ((problem: string | null) => void) | null }));

vi.mock("@/lib/stackPreview", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/stackPreview")>();
  return {
    ...actual,
    onStackPreviewFeed: (listener: (problem: string | null) => void) => {
      feed.listener = listener;
      return () => {
        feed.listener = null;
      };
    },
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

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

// Dragging a rectangle area on the stack preview (#104). The stage is mocked at 640 x 400 css
// pixels, so one canvas unit is two pixels and a pixel delta halves into units.
const K = 2;

async function openBuilderWith(layerName: string) {
  window.history.replaceState({}, "", "/effects/?stack=Sunset%20glow&state=complete,stack");
  await refreshAll();
  render(
    <ToastProvider>
      <ToastHost />
      <StripSlotsProvider>
        <div className="strip-slots">
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </div>
        <EffectsView />
      </StripSlotsProvider>
    </ToastProvider>,
  );
  await screen.findByTestId("stack-builder");
  await screen.findByRole("heading", { name: "Sunset glow" });
  await waitFor(() => expect(screen.getAllByText(layerName).length).toBeGreaterThan(0));
  fireEvent.click(screen.getByText(layerName).closest("li")!);
  const stage = document.querySelector<HTMLElement>(".rd-canvas")!;
  vi.spyOn(stage, "getBoundingClientRect").mockReturnValue({
    x: 0, y: 0, left: 0, top: 0, right: 640, bottom: 400, width: 640, height: 400, toJSON: () => ({}),
  } as DOMRect);
  return stage;
}

const frame = () => document.querySelector<HTMLElement>(".rd-area-frame")!;
const readout = () => document.querySelector(".layer-area-text")?.textContent ?? "";
/** Canvas units to the stage's css pixels. */
const px = (u: number) => u * K;

const mouse = { pointerId: 1, pointerType: "mouse", buttons: 1 };

function drag(el: HTMLElement, from: [number, number], to: [number, number], extra: object = {}) {
  act(() => {
    fireEvent.pointerDown(el, { ...mouse, clientX: from[0], clientY: from[1] });
    fireEvent.pointerMove(el, { ...mouse, clientX: to[0], clientY: to[1], ...extra });
    fireEvent.pointerUp(el, { ...mouse, buttons: 0, clientX: to[0], clientY: to[1] });
  });
}

type Saved = { areas?: Array<{ kind: string; x?: number; y?: number; width?: number; height?: number }> };
const saves = () => ((globalThis as { __stackSaves?: Saved[] }).__stackSaves ??= []);

describe("dragging a rectangle area on the preview", () => {
  it("moves the area by the pointer delta, in canvas units, and marks the draft unsaved", async () => {
    await openBuilderWith("Heat");
    expect(readout()).toContain("x 16, y 150");
    expect(screen.queryByText("Unsaved")).toBeNull();
    // Inside the rectangle (110 x 46 at 16, 150): press its middle, drag (+40, -20) px.
    drag(frame(), [px(71), px(173)], [px(71) + 40, px(173) - 20]);
    await waitFor(() => expect(readout()).toContain("x 36, y 140"));
    expect(readout()).toContain("110 × 46");
    expect(screen.getByText("Unsaved")).toBeTruthy();
  });

  it("clamps a move at the canvas edge", async () => {
    await openBuilderWith("Heat");
    drag(frame(), [px(71), px(173)], [px(71) + 5000, px(173) + 5000]);
    await waitFor(() => expect(readout()).toContain("x 210, y 154"));
  });

  it("resizes from the opposite corner", async () => {
    await openBuilderWith("Heat");
    // The south-east corner is at (126, 196); drag it (-40, -20) px, so the box shrinks by (20, 10).
    drag(frame(), [px(126), px(196)], [px(126) - 40, px(196) - 20]);
    await waitFor(() => expect(readout()).toContain("90 × 36"));
    expect(readout()).toContain("x 16, y 150");
    // The north-west corner pulls the other way, with the south-east corner held.
    drag(frame(), [px(16), px(150)], [px(16) + 20, px(150) + 10]);
    await waitFor(() => expect(readout()).toContain("x 26, y 155"));
    expect(readout()).toContain("80 × 31");
  });

  it("clamps a resize at the canvas edge and at the minimum size", async () => {
    await openBuilderWith("Heat");
    drag(frame(), [px(126), px(196)], [px(126) + 5000, px(196) + 5000]);
    await waitFor(() => expect(readout()).toContain("304 × 50"));
    drag(frame(), [px(320), px(200)], [0, 0]);
    await waitFor(() => expect(readout()).toContain("4 × 4"));
    expect(readout()).toContain("x 16, y 150");
  });

  it("keeps the aspect ratio on a corner with Shift", async () => {
    await openBuilderWith("Heat");
    drag(frame(), [px(126), px(196)], [px(126) + 22, px(196) + 2], { shiftKey: true });
    // 110 x 46 would grow by 1.1 (width 121), but the canvas floor is 50 below the top edge, so
    // the ratio holds at the largest fit: 119.6 x 50 -> 120 x 50.
    await waitFor(() => expect(readout()).toContain("120 × 50"));
  });

  it("nudges one unit with an arrow key and ten with Shift", async () => {
    await openBuilderWith("Heat");
    fireEvent.keyDown(frame(), { key: "ArrowRight" });
    await waitFor(() => expect(readout()).toContain("x 17, y 150"));
    fireEvent.keyDown(frame(), { key: "ArrowUp", shiftKey: true });
    await waitFor(() => expect(readout()).toContain("x 17, y 140"));
    expect(screen.getByText("Unsaved")).toBeTruthy();
  });

  it("ignores a press outside the rectangle", async () => {
    await openBuilderWith("Heat");
    drag(frame(), [px(2), px(2)], [px(40), px(40)]);
    expect(readout()).toContain("x 16, y 150");
  });

  it("draws no handles and takes no drag for an area that is not a rectangle", async () => {
    await openBuilderWith("Sunset");
    expect(frame().dataset.editable).toBe("false");
    expect(frame().querySelector(".handle")).toBeNull();
    const before = readout();
    drag(frame(), [px(100), px(100)], [px(150), px(150)]);
    fireEvent.keyDown(frame(), { key: "ArrowRight" });
    expect(readout()).toBe(before);
    expect(screen.queryByText("Unsaved")).toBeNull();
  });

  it("puts the area back when the pointer is cancelled", async () => {
    await openBuilderWith("Heat");
    act(() => {
      fireEvent.pointerDown(frame(), { ...mouse, clientX: px(71), clientY: px(173) });
      fireEvent.pointerMove(frame(), { ...mouse, clientX: px(71) + 60, clientY: px(173) - 40 });
    });
    await waitFor(() => expect(readout()).toContain("x 46, y 130"));
    act(() => {
      fireEvent.pointerCancel(frame(), { ...mouse, buttons: 0 });
    });
    await waitFor(() => expect(readout()).toContain("x 16, y 150"));
  });

  it("grabs a corner from a press in the 12 px skirt", async () => {
    await openBuilderWith("Heat");
    // Ten css pixels outside the south-east corner (126, 196), still inside the 12 px reach.
    drag(frame(), [px(126) + 8, px(196) + 8], [px(126) + 8 - 40, px(196) + 8 - 20]);
    await waitFor(() => expect(readout()).toContain("90 × 36"));
    expect(readout()).toContain("x 16, y 150");
  });

  it("ends the drag when a mouse moves with no button down", async () => {
    await openBuilderWith("Heat");
    act(() => {
      fireEvent.pointerDown(frame(), { ...mouse, clientX: px(71), clientY: px(173) });
      fireEvent.pointerMove(frame(), { ...mouse, clientX: px(71) + 40, clientY: px(173) - 20 });
    });
    await waitFor(() => expect(readout()).toContain("x 36, y 140"));
    // The pointerup was lost: the next move has no button, and the area stays where it is.
    act(() => {
      fireEvent.pointerMove(frame(), { ...mouse, buttons: 0, clientX: px(71) + 200, clientY: px(173) - 20 });
    });
    expect(readout()).toContain("x 36, y 140");
  });

  it("ends the drag when the pointer capture is lost", async () => {
    await openBuilderWith("Heat");
    act(() => {
      fireEvent.pointerDown(frame(), { ...mouse, clientX: px(71), clientY: px(173) });
      fireEvent.pointerMove(frame(), { ...mouse, clientX: px(71) + 40, clientY: px(173) - 20 });
      fireEvent.lostPointerCapture(frame(), { ...mouse });
      fireEvent.pointerMove(frame(), { ...mouse, clientX: px(71) + 200, clientY: px(173) - 20 });
    });
    expect(readout()).toContain("x 36, y 140");
  });

  it("neither draws nor drags the area of a layer that is switched off", async () => {
    await openBuilderWith("Heat");
    expect(frame()).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Hide Heat" }));
    await waitFor(() => expect(document.querySelector(".rd-area-frame")).toBeNull());
  });

  // Last: the Save writes the demo store, and a later test would open the moved stack.
  it("saves exactly the rectangle the area picker's Rectangle mode would make", async () => {
    await openBuilderWith("Heat");
    saves().length = 0;
    drag(frame(), [px(71), px(173)], [px(71) + 40, px(173) - 20]);
    await waitFor(() => expect(readout()).toContain("x 36, y 140"));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saves()).toHaveLength(1));
    const picker = { kind: "rect", ...rectFromDrag({ x: 36, y: 140 }, { x: 146, y: 186 }) };
    expect(saves()[0]?.areas).toContainEqual(picker);
    await waitFor(() => expect(screen.queryByText("Unsaved")).toBeNull());
  });
});
