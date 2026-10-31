import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { Slider } from "@/components/Slider";

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

/** A slot 114 px wide at x=0: with the 14 px cap, 100 px of travel, so clientX 7 + n is n %. */
function setup(onChange = vi.fn(), onCommit = vi.fn()) {
  let setDisabled: (d: boolean) => void = () => {};
  function Host() {
    const [v, setV] = useState(0);
    const [disabled, set] = useState(false);
    setDisabled = set;
    return (
      <Slider value={v} min={0} max={100} step={25} label="Key response" disabled={disabled} onChange={(n) => { onChange(n); setV(n); }} onCommit={onCommit} />
    );
  }
  const { container } = render(<Host />);
  const el = container.querySelector(".slider") as HTMLDivElement;
  el.getBoundingClientRect = () => ({ left: 0, width: 114, top: 0, right: 114, bottom: 24, height: 24, x: 0, y: 0, toJSON() {} });
  const p = () => Number(el.style.getPropertyValue("--p"));
  return { el, p, onChange, onCommit, disable: () => act(() => setDisabled(true)) };
}

describe("Slider (#13)", () => {
  it("lets the cap follow the pointer between steps while dragging", () => {
    const { el, p } = setup();
    fireEvent.pointerDown(el, { clientX: 7 + 10, pointerId: 1 });
    expect(p()).toBeCloseTo(0.1);
    fireEvent.pointerMove(el, { clientX: 7 + 37, pointerId: 1 });
    expect(p()).toBeCloseTo(0.37);
    expect(el.getAttribute("aria-valuenow")).toBe("25");
  });

  it("sends only values on the step grid, once per change", () => {
    const { el, onChange } = setup();
    fireEvent.pointerDown(el, { clientX: 7 + 5, pointerId: 1 });
    for (const x of [8, 11, 14, 20, 30, 31, 33, 60]) fireEvent.pointerMove(el, { clientX: 7 + x, pointerId: 1 });
    expect(onChange.mock.calls.map((c) => c[0])).toEqual([25, 50]);
  });

  it("settles onto the committed step on release", () => {
    const { el, p, onCommit } = setup();
    fireEvent.pointerDown(el, { clientX: 7 + 10, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 7 + 40, pointerId: 1 });
    fireEvent.pointerUp(el, { clientX: 7 + 40, pointerId: 1 });
    expect(onCommit).toHaveBeenCalledWith(50);
    expect(p()).toBeCloseTo(0.5);
    expect(el.dataset.dragging).toBe("false");
  });

  it("ends the drag on cancel: the cap goes back to the value", () => {
    const { el, p } = setup();
    fireEvent.pointerDown(el, { clientX: 7 + 10, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 7 + 37, pointerId: 1 });
    fireEvent.pointerCancel(el, { pointerId: 1 });
    expect(el.dataset.dragging).toBe("false");
    expect(p()).toBeCloseTo(0.25);
  });

  it("ends the drag on release even when the slider was disabled meanwhile, and commits nothing", () => {
    const { el, p, onCommit, disable } = setup();
    fireEvent.pointerDown(el, { clientX: 7 + 10, pointerId: 1 });
    fireEvent.pointerMove(el, { clientX: 7 + 37, pointerId: 1 });
    disable();
    fireEvent.pointerUp(el, { clientX: 7 + 37, pointerId: 1 });
    expect(el.dataset.dragging).toBe("false");
    expect(p()).toBeCloseTo(0.25);
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("lets only the first pointer move, end and commit the drag", () => {
    const { el, onCommit } = setup();
    fireEvent.pointerDown(el, { clientX: 7 + 10, pointerId: 1 });
    fireEvent.pointerDown(el, { clientX: 7 + 90, pointerId: 2 });
    fireEvent.pointerMove(el, { clientX: 7 + 90, pointerId: 2 });
    fireEvent.pointerUp(el, { clientX: 7 + 90, pointerId: 2 });
    expect(el.dataset.dragging).toBe("true");
    fireEvent.pointerUp(el, { clientX: 7 + 50, pointerId: 1 });
    expect(onCommit.mock.calls).toEqual([[50]]);
  });
});
