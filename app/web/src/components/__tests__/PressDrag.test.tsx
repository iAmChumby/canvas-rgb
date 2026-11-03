import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ColorPicker } from "@/components/ColorPicker";
import { HueRing } from "@/components/HueRing";
import { Slider } from "@/components/Slider";

// The class test for `cause:ui-interaction` (#105): one rule over every control the pointer
// drags. A press is defaultPrevented (so Chromium's compatibility mousedown never starts a
// native drag or a text selection), a dragstart from the surface is refused, and the surface
// says it is not draggable. fireEvent returns false when the default was prevented.
//
// The four surfaces here are the ones the helper guards with `draggable={false}`; the SVG
// surfaces (the stack area picker's desk, the fan curve points, the layout stage) have no
// draggable attribute and are covered by their own views' tests.

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
});

afterEach(cleanup);

function renderSlider(disabled = false): HTMLDivElement {
  const { container } = render(
    <Slider value={0} min={0} max={100} step={25} label="Key response" disabled={disabled} onChange={vi.fn()} />,
  );
  const el = container.querySelector(".slider") as HTMLDivElement;
  el.getBoundingClientRect = () => ({ left: 0, width: 114, top: 0, right: 114, bottom: 24, height: 24, x: 0, y: 0, toJSON() {} });
  return el;
}

function renderHueRing(): HTMLDivElement {
  const { container } = render(<HueRing value={0} label="Angle" onChange={vi.fn()} />);
  return container.querySelector(".hue-ring") as HTMLDivElement;
}

function renderColorPicker(): { sv: HTMLDivElement; hue: HTMLDivElement } {
  const { container } = render(<ColorPicker label="Colour" value="#ff0000" onChange={vi.fn()} />);
  return {
    sv: container.querySelector(".picker-sv") as HTMLDivElement,
    hue: container.querySelector(".picker-hue") as HTMLDivElement,
  };
}

describe("PressDrag: a press never starts a native drag or a text selection (#105)", () => {
  const surfaces = (el: HTMLDivElement) => el;

  it("the slider's press is prevented, its dragstart is refused, and it is not draggable", () => {
    const el = renderSlider();
    expect(fireEvent.pointerDown(el, { clientX: 7 + 20, pointerId: 1 })).toBe(false);
    fireEvent.pointerUp(el, { pointerId: 1 });
    expect(fireEvent.dragStart(surfaces(el))).toBe(false);
    expect(el.getAttribute("draggable")).toBe("false");
  });

  it("a second pointer's press is prevented too", () => {
    const el = renderSlider();
    expect(fireEvent.pointerDown(el, { clientX: 7 + 20, pointerId: 1 })).toBe(false);
    expect(fireEvent.pointerDown(el, { clientX: 7 + 90, pointerId: 2 })).toBe(false);
    fireEvent.pointerUp(el, { pointerId: 1 });
  });

  it("a disabled slider keeps the old press: a click on it can still blur a focused field", () => {
    const el = renderSlider(true);
    expect(fireEvent.pointerDown(el, { clientX: 7 + 20, pointerId: 1 })).toBe(true);
  });

  it("the hue ring's press is prevented, its dragstart is refused, and it is not draggable", () => {
    const el = renderHueRing();
    expect(fireEvent.pointerDown(el, { clientX: 60, clientY: 10, pointerId: 1 })).toBe(false);
    fireEvent.pointerUp(el, { pointerId: 1 });
    expect(fireEvent.dragStart(el)).toBe(false);
    expect(el.getAttribute("draggable")).toBe("false");
  });

  it("the colour picker's square and strip are prevented and refuse a dragstart", () => {
    const { sv, hue } = renderColorPicker();
    expect(fireEvent.pointerDown(sv, { clientX: 20, clientY: 20, pointerId: 1 })).toBe(false);
    fireEvent.pointerUp(sv, { pointerId: 1 });
    expect(fireEvent.pointerDown(hue, { clientX: 20, pointerId: 1 })).toBe(false);
    fireEvent.pointerUp(hue, { pointerId: 1 });
    expect(fireEvent.dragStart(sv)).toBe(false);
    expect(fireEvent.dragStart(hue)).toBe(false);
    expect(sv.getAttribute("draggable")).toBe("false");
    expect(hue.getAttribute("draggable")).toBe("false");
  });

  it("a press still takes focus, so the keyboard keeps working after a mouse press", () => {
    const el = renderSlider();
    fireEvent.pointerDown(el, { clientX: 7 + 20, pointerId: 1 });
    fireEvent.pointerUp(el, { pointerId: 1 });
    expect(document.activeElement).toBe(el);
  });
});
