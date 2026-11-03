import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { useState } from "react";
import { AngleDial } from "@/components/AngleDial";

// React needs this flag to run effects inside act() under a test runner.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(cleanup);

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

/**
 * A dial 100 px square at x=0, y=0, so its centre is (50, 50): a press at (100, 50) is 0°,
 * (50, 100) is 90°, (0, 50) is 180° and (50, 0) is 270°. A parent holds the value, as
 * PropertyControls does, so each key step lands on the last one.
 */
function setup(onChange = vi.fn(), onCommit = vi.fn(), start = 90, disabled = false) {
  function Host() {
    const [v, setValue] = useState(start);
    return <AngleDial value={v} label="Angle" disabled={disabled} onChange={(n) => { onChange(n); setValue(n); }} onCommit={onCommit} />;
  }
  const { container } = render(<Host />);
  const el = container.querySelector(".angle-dial") as HTMLDivElement;
  el.getBoundingClientRect = () => ({ left: 0, width: 100, top: 0, right: 100, bottom: 100, height: 100, x: 0, y: 0, toJSON() {} });
  return { el, onChange, onCommit };
}

describe("AngleDial (#106)", () => {
  it("maps the pointer to the canvas' angles: right 0, down 90, left 180, up 270", () => {
    const { el, onChange } = setup();
    fireEvent.pointerDown(el, { clientX: 100, clientY: 50, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 50, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 0, clientY: 50, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 50, clientY: 0, pointerId: 1 });
    expect(onChange.mock.calls).toEqual([[0], [90], [180], [270]]);
  });

  it("commits the angle under the pointer on release", () => {
    const { el, onCommit } = setup();
    fireEvent.pointerDown(el, { clientX: 50, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 0, clientY: 50, pointerId: 1 });
    fireEvent.pointerUp(el, { clientX: 0, clientY: 50, pointerId: 1 });
    expect(onCommit).toHaveBeenCalledWith(180);
    expect(el.dataset.dragging).toBe("false");
  });

  it("never starts a native drag: pointerdown is prevented (#105)", () => {
    const { el } = setup();
    const down = new MouseEvent("pointerdown", { bubbles: true, cancelable: true });
    Object.defineProperty(down, "pointerId", { value: 1 });
    el.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
  });

  it("lets only the first pointer move, end and commit the drag", () => {
    const { el, onCommit } = setup();
    fireEvent.pointerDown(el, { clientX: 100, clientY: 50, pointerId: 1 });
    fireEvent.pointerDown(el, { clientX: 0, clientY: 50, pointerId: 2 });
    fireEvent.pointerMove(el, { clientX: 0, clientY: 50, pointerId: 2 });
    fireEvent.pointerUp(el, { clientX: 0, clientY: 50, pointerId: 2 });
    expect(el.dataset.dragging).toBe("true");
    fireEvent.pointerUp(el, { clientX: 50, clientY: 100, pointerId: 1 });
    expect(onCommit.mock.calls).toEqual([[90]]);
  });

  it("steps 1° with the arrow keys and 15° with shift", () => {
    const { el, onChange, onCommit } = setup(undefined, undefined, 0);
    fireEvent.keyDown(el, { key: "ArrowRight" });
    expect(onChange).toHaveBeenLastCalledWith(1);
    fireEvent.keyDown(el, { key: "ArrowRight", shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith(16);
    fireEvent.keyDown(el, { key: "ArrowUp", shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith(31);
    fireEvent.keyDown(el, { key: "ArrowLeft" });
    expect(onChange).toHaveBeenLastCalledWith(30);
    fireEvent.keyDown(el, { key: "ArrowDown", shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith(15);
    expect(onCommit).toHaveBeenCalledTimes(5);
  });

  it("wraps through 0: one step down from 0 is 359", () => {
    const { el, onChange } = setup(undefined, undefined, 0);
    fireEvent.keyDown(el, { key: "ArrowLeft" });
    expect(onChange).toHaveBeenLastCalledWith(359);
  });

  it("goes to 0 and 359 with Home and End", () => {
    const { el, onChange } = setup(undefined, undefined, 90);
    fireEvent.keyDown(el, { key: "Home" });
    expect(onChange).toHaveBeenLastCalledWith(0);
    fireEvent.keyDown(el, { key: "End" });
    expect(onChange).toHaveBeenLastCalledWith(359);
  });

  it("says the degrees and the direction for a screen reader", () => {
    const { el: down } = setup(undefined, undefined, 90);
    expect(down.getAttribute("role")).toBe("slider");
    expect(down.getAttribute("aria-valuemin")).toBe("0");
    expect(down.getAttribute("aria-valuemax")).toBe("359");
    expect(down.getAttribute("aria-valuenow")).toBe("90");
    expect(down.getAttribute("aria-valuetext")).toBe("90°, pointing down");
    const { el: right } = setup(undefined, undefined, 0);
    expect(right.getAttribute("aria-valuetext")).toBe("0°, pointing right");
    const { el: up } = setup(undefined, undefined, 270);
    expect(up.getAttribute("aria-valuetext")).toBe("270°, pointing up");
  });

  it("does nothing while disabled", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    const { el } = setup(onChange, onCommit, 90, true);
    fireEvent.pointerDown(el, { clientX: 0, clientY: 50, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 50, clientY: 100, pointerId: 1 });
    fireEvent.pointerUp(el, { clientX: 50, clientY: 100, pointerId: 1 });
    fireEvent.keyDown(el, { key: "ArrowRight" });
    expect(onChange).not.toHaveBeenCalled();
    expect(onCommit).not.toHaveBeenCalled();
    expect(el.getAttribute("aria-disabled")).toBe("true");
  });
});
