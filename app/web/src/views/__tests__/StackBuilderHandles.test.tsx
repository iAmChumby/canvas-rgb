// The picked frame's handles on the stage (#257 phase 3): the direction arrow dragged around the
// frame's centre with Shift snapping to 15°, the speed pill scrubbed and stepped, the inspector's
// dial and slider mirroring both, and one gesture being one undo step.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { dismissNotifications, NotifyHost } from "@/components/Notify";
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
  cleanup();
  vi.restoreAllMocks();
  act(() => dismissNotifications());
  await new Promise((resolve) => setTimeout(resolve, 0));
});

/** The stage is 640 x 400 css pixels at (0, 100): two pixels per canvas unit. */
const K = 2;
const px = (u: number) => u * K;
const atY = (u: number) => 100 + u * K;
/** The whole-canvas layers' frame centre, where the direction handle's hub sits. */
const CENTRE = { x: px(160), y: atY(100) };

const mouse = { pointerId: 7, pointerType: "mouse", buttons: 1 };

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
  vi.spyOn(stage, "getBoundingClientRect").mockReturnValue({
    x: 0, y: 0, left: 0, top: 100, right: 640, bottom: 500, width: 640, height: 400, toJSON: () => ({}),
  } as DOMRect);
  return { stage, view };
}

/** Picks a row by its layer id, as a press on it does. */
function pickRow(id: string) {
  const row = document.querySelector<HTMLElement>(`.layer-row[data-layer-id="${id}"]`)!;
  act(() => void fireEvent.click(row));
  return row;
}

/** The inspector's own controls for the picked layer (the dial and the sliders). */
const inspector = () => within(document.querySelector<HTMLElement>(".rd-layer-edit")!);
const dial = () => inspector().getByRole("slider", { name: "Angle" });
const slider = (name: string) => inspector().getByRole("slider", { name });

describe("the direction handle", () => {
  it("turns the effect around the frame's centre, and the inspector's dial follows", async () => {
    await openBuilder();
    pickRow("l1"); // Sunset, the Gradient
    const grip = await screen.findByRole("slider", { name: "Direction of Sunset" });
    expect(dial().getAttribute("aria-valuenow")).toBe("112");
    // A gradient at rest has scroll_speed 0, so the arrow is at its shortest: 16 px.
    expect(document.querySelector<HTMLElement>(".rd-angle-arm")?.style.width).toBe("16px");

    act(() => void fireEvent.pointerDown(grip, { ...mouse, clientX: CENTRE.x + 16, clientY: CENTRE.y }));
    act(() => void fireEvent.pointerMove(grip, { ...mouse, clientX: CENTRE.x + 60, clientY: CENTRE.y }));
    act(() => void fireEvent.pointerUp(grip, { ...mouse, buttons: 0, clientX: CENTRE.x + 60, clientY: CENTRE.y }));

    expect(dial().getAttribute("aria-valuenow")).toBe("0");
    expect(grip.getAttribute("aria-valuenow")).toBe("0");
    // One drag is one step: a single undo puts the 112 back.
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(dial().getAttribute("aria-valuenow")).toBe("112"));
  });

  it("snaps to 15° while Shift is held", async () => {
    await openBuilder();
    pickRow("l1");
    const grip = await screen.findByRole("slider", { name: "Direction of Sunset" });
    const x = CENTRE.x + 60 * Math.cos((20 * Math.PI) / 180);
    const y = CENTRE.y + 60 * Math.sin((20 * Math.PI) / 180);
    act(() => void fireEvent.pointerDown(grip, { ...mouse, clientX: CENTRE.x + 16, clientY: CENTRE.y }));
    act(() => void fireEvent.pointerMove(grip, { ...mouse, clientX: x, clientY: y, shiftKey: true }));
    act(() => void fireEvent.pointerUp(grip, { ...mouse, buttons: 0, clientX: x, clientY: y, shiftKey: true }));
    expect(dial().getAttribute("aria-valuenow")).toBe("15");
  });

  it("nudges 1° with the arrows and 15° with Shift, and one nudge is one step", async () => {
    await openBuilder();
    pickRow("l1");
    const grip = await screen.findByRole("slider", { name: "Direction of Sunset" });
    act(() => void fireEvent.keyDown(grip, { key: "ArrowRight" }));
    act(() => void fireEvent.keyUp(grip, { key: "ArrowRight" }));
    expect(dial().getAttribute("aria-valuenow")).toBe("113");
    act(() => void fireEvent.keyDown(grip, { key: "ArrowLeft", shiftKey: true }));
    act(() => void fireEvent.keyUp(grip, { key: "ArrowLeft", shiftKey: true }));
    expect(dial().getAttribute("aria-valuenow")).toBe("98");
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(dial().getAttribute("aria-valuenow")).toBe("113"));
  });
});

describe("the speed pill", () => {
  it("scrubs left and right, and the inspector's slider follows", async () => {
    await openBuilder();
    pickRow("l3"); // Sparkles, Rain: speed only, so a pill
    const pill = await screen.findByRole("slider", { name: "Speed of Sparkles" });
    expect(pill.getAttribute("aria-valuenow")).toBe("120");
    expect(slider("Speed").getAttribute("aria-valuenow")).toBe("120");

    // A scrub of half the 160 px sweep on Rain's 10..400 range: 120 climbs half of it, to 315.
    act(() => void fireEvent.pointerDown(pill, { ...mouse, clientX: 400, clientY: 500 }));
    act(() => void fireEvent.pointerMove(pill, { ...mouse, clientX: 480, clientY: 500 }));
    act(() => void fireEvent.pointerUp(pill, { ...mouse, buttons: 0, clientX: 480, clientY: 500 }));

    expect(pill.getAttribute("aria-valuenow")).toBe("315");
    expect(slider("Speed").getAttribute("aria-valuenow")).toBe("315");
    // One scrub is one step.
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(pill.getAttribute("aria-valuenow")).toBe("120"));
  });

  it("steps with Left and Right, and one press is one step", async () => {
    await openBuilder();
    pickRow("l3");
    const pill = await screen.findByRole("slider", { name: "Speed of Sparkles" });
    act(() => void fireEvent.keyDown(pill, { key: "ArrowRight" }));
    act(() => void fireEvent.keyUp(pill, { key: "ArrowRight" }));
    expect(pill.getAttribute("aria-valuenow")).toBe("121");
    act(() => void fireEvent.keyDown(pill, { key: "ArrowLeft" }));
    act(() => void fireEvent.keyUp(pill, { key: "ArrowLeft" }));
    expect(pill.getAttribute("aria-valuenow")).toBe("120");
    act(() => void fireEvent.keyDown(pill, { key: "ArrowLeft" }));
    act(() => void fireEvent.keyUp(pill, { key: "ArrowLeft" }));
    expect(pill.getAttribute("aria-valuenow")).toBe("119");
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(pill.getAttribute("aria-valuenow")).toBe("120"));
  });

  it("draws no handle for an effect with neither a direction nor a speed", async () => {
    await openBuilder();
    // The Gradient's handle is there first, so the absence below is a real absence.
    pickRow("l1");
    await screen.findByRole("slider", { name: "Direction of Sunset" });
    pickRow("l2"); // Heat, Fire: no angle and no speed
    await waitFor(() => expect(document.querySelector(".layer-row[data-layer-id='l2']")?.getAttribute("data-selected")).toBe("true"));
    expect(screen.queryByRole("slider", { name: /^Direction of Heat$/ })).toBeNull();
    expect(screen.queryByRole("slider", { name: /^Speed of Heat$/ })).toBeNull();
    expect(document.querySelector(".rd-angle-handle")).toBeNull();
    expect(document.querySelector(".rd-speed-pill")).toBeNull();
  });
});

describe("a gesture cut off by an unmount (#257 review)", () => {
  it("closes the undo run, so the next edit after the handle comes back is its own step", async () => {
    await openBuilder();
    pickRow("l1");
    const grip = await screen.findByRole("slider", { name: "Direction of Sunset" });
    // A drag left down: the angle goes to 0.
    act(() => void fireEvent.pointerDown(grip, { ...mouse, clientX: CENTRE.x + 16, clientY: CENTRE.y }));
    act(() => void fireEvent.pointerMove(grip, { ...mouse, clientX: CENTRE.x + 60, clientY: CENTRE.y }));
    expect(dial().getAttribute("aria-valuenow")).toBe("0");

    // Tab away (the handle unmounts) and back (a fresh handle, its run counter at 0).
    act(() => void fireEvent.keyDown(grip, { key: "Tab" }));
    await waitFor(() => expect(document.querySelector('.layer-row[data-layer-id="l3"]')?.getAttribute("data-selected")).toBe("true"));
    const stage = document.querySelector<HTMLElement>(".rd-canvas")!;
    act(() => void fireEvent.keyDown(stage, { key: "Tab", shiftKey: true }));
    await waitFor(() => expect(document.querySelector('.layer-row[data-layer-id="l1"]')?.getAttribute("data-selected")).toBe("true"));

    const fresh = await screen.findByRole("slider", { name: "Direction of Sunset" });
    act(() => void fireEvent.keyDown(fresh, { key: "ArrowRight" }));
    act(() => void fireEvent.keyUp(fresh, { key: "ArrowRight" }));
    expect(dial().getAttribute("aria-valuenow")).toBe("1");

    // One undo takes back only the nudge, to the dragged angle (0), not to the pre-drag 112.
    act(() => void fireEvent.keyDown(document.body, { key: "z", ctrlKey: true }));
    await waitFor(() => expect(dial().getAttribute("aria-valuenow")).toBe("0"));
  });
});
