import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { RailResizeHandle } from "@/components/RailResizeHandle";
import { RAIL_KEY } from "@/lib/railWidth";

// The rail's resize handle (#126): the one place the rail's width can be changed, with the
// keyboard and the mouse.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  // jsdom has no PointerEvent and no pointer capture; a MouseEvent carries clientX just as well.
  if (!("PointerEvent" in window)) {
    class PE extends MouseEvent {
      pointerId: number;
      constructor(type: string, init: PointerEventInit = {}) {
        super(type, init);
        this.pointerId = init.pointerId ?? 1;
      }
    }
    (window as unknown as { PointerEvent: unknown }).PointerEvent = PE;
  }
  const captured = new Set<number>();
  HTMLElement.prototype.setPointerCapture = (id: number) => void captured.add(id);
  HTMLElement.prototype.releasePointerCapture = (id: number) => void captured.delete(id);
  HTMLElement.prototype.hasPointerCapture = (id: number) => captured.has(id);
  // A 1440 px window: a quarter of it is 360, so the rail's own 320 px cap is the bound.
  Object.defineProperty(window, "innerWidth", { value: 1440, configurable: true, writable: true });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.localStorage.clear();
  document.documentElement.style.removeProperty("--rail-w");
});

/** The handle, with a stored width (null for none). */
function setup(stored: string | null = null): HTMLElement {
  if (stored !== null) window.localStorage.setItem(RAIL_KEY, stored);
  const { container } = render(<RailResizeHandle />);
  return container.querySelector(".rail-resize") as HTMLElement;
}

describe("the rail's resize handle (#126)", () => {
  it("is a separator a screen reader can name, with the width it shows", () => {
    const el = setup();
    expect(el.getAttribute("role")).toBe("separator");
    expect(el.getAttribute("aria-orientation")).toBe("vertical");
    expect(el.getAttribute("aria-valuemin")).toBe("200");
    expect(el.getAttribute("aria-valuemax")).toBe("320");
    expect(el.getAttribute("aria-valuenow")).toBe("200");
    expect(el.getAttribute("aria-label")).toBeTruthy();
    expect(el.getAttribute("tabindex")).toBe("0");
  });

  it("reads the stored width back and shows it", () => {
    const el = setup("260");
    expect(el.getAttribute("aria-valuenow")).toBe("260");
    expect(document.documentElement.style.getPropertyValue("--rail-w")).toBe("260px");
  });

  it("clamps a stored width the window cannot hold", () => {
    // A 1000 px window: a quarter is 250, so the rail gives way before the page does.
    Object.defineProperty(window, "innerWidth", { value: 1000, configurable: true, writable: true });
    const el = setup("320");
    expect(el.getAttribute("aria-valuenow")).toBe("250");
    Object.defineProperty(window, "innerWidth", { value: 1440, configurable: true, writable: true });
  });

  it("steps 8 px with the arrow keys, and 32 px with shift", () => {
    const el = setup();
    fireEvent.keyDown(el, { key: "ArrowRight" });
    expect(el.getAttribute("aria-valuenow")).toBe("208");
    fireEvent.keyDown(el, { key: "ArrowRight" });
    expect(el.getAttribute("aria-valuenow")).toBe("216");
    fireEvent.keyDown(el, { key: "ArrowLeft" });
    expect(el.getAttribute("aria-valuenow")).toBe("208");
    fireEvent.keyDown(el, { key: "ArrowRight", shiftKey: true });
    expect(el.getAttribute("aria-valuenow")).toBe("240");
    fireEvent.keyDown(el, { key: "ArrowLeft", shiftKey: true });
    expect(el.getAttribute("aria-valuenow")).toBe("208");
  });

  it("goes to the bounds with home and end", () => {
    const el = setup();
    fireEvent.keyDown(el, { key: "End" });
    expect(el.getAttribute("aria-valuenow")).toBe("320");
    fireEvent.keyDown(el, { key: "Home" });
    expect(el.getAttribute("aria-valuenow")).toBe("200");
  });

  it("never steps past the bounds", () => {
    const el = setup();
    fireEvent.keyDown(el, { key: "ArrowLeft" });
    expect(el.getAttribute("aria-valuenow")).toBe("200");
    fireEvent.keyDown(el, { key: "End" });
    fireEvent.keyDown(el, { key: "ArrowRight" });
    expect(el.getAttribute("aria-valuenow")).toBe("320");
  });

  it("puts the rail back to its default with enter", () => {
    const el = setup("300");
    fireEvent.keyDown(el, { key: "Enter" });
    expect(el.getAttribute("aria-valuenow")).toBe("200");
    expect(window.localStorage.getItem(RAIL_KEY)).toBe("200");
  });

  it("puts the rail back to its default with a double-click", () => {
    const el = setup("300");
    let now = 1_000;
    vi.spyOn(Date, "now").mockImplementation(() => now);
    // A press that travels nothing is a click, not a drag.
    fireEvent.pointerDown(el, { clientX: 300, pointerId: 1 });
    fireEvent.pointerUp(el, { clientX: 300, pointerId: 1 });
    expect(el.getAttribute("aria-valuenow")).toBe("300");
    // The second press of the pair.
    now += 200;
    fireEvent.pointerDown(el, { clientX: 300, pointerId: 1 });
    expect(el.getAttribute("aria-valuenow")).toBe("200");
    fireEvent.pointerUp(el, { clientX: 300, pointerId: 1 });
    expect(window.localStorage.getItem(RAIL_KEY)).toBe("200");
  });

  it("drags the rail's edge and keeps the width", () => {
    const el = setup();
    fireEvent.pointerDown(el, { clientX: 200, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 240, pointerId: 1 });
    expect(el.getAttribute("aria-valuenow")).toBe("240");
    fireEvent.pointerUp(el, { clientX: 240, pointerId: 1 });
    expect(window.localStorage.getItem(RAIL_KEY)).toBe("240");
    expect(document.documentElement.style.getPropertyValue("--rail-w")).toBe("240px");
  });

  it("does not move the rail when the press travels only a little", () => {
    const el = setup();
    fireEvent.pointerDown(el, { clientX: 200, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 202, pointerId: 1 });
    expect(el.getAttribute("aria-valuenow")).toBe("200");
    fireEvent.pointerUp(el, { clientX: 202, pointerId: 1 });
    expect(window.localStorage.getItem(RAIL_KEY)).toBeNull();
  });

  it("never starts a native drag or a text selection (#105)", () => {
    const el = setup();
    expect(fireEvent.pointerDown(el, { clientX: 200, pointerId: 1 })).toBe(false);
    fireEvent.pointerUp(el, { clientX: 200, pointerId: 1 });
    expect(fireEvent.dragStart(el)).toBe(false);
    expect(el.getAttribute("draggable")).toBe("false");
  });
});
