// Issue #369: the colour swatch must not pulse while a colour is dragged.
//
// The box was already one size (the #367 measurement). What Luke saw - "it looks like it shrinks
// and grows every time the hex code changes" - is the mask reveal from issue #187 replaying on
// every value of a stream: the field swatch's overlay, and the picker's preview, which was keyed
// by the colour and so was replaced about every 180 ms during a drag.
//
// The rule, locked in here: a reveal belongs to a change that lands once. A drag on the square or
// the strip, a held arrow key, a run of values from the daemon: the new colour shows at once and
// nothing replays. A quick swatch clicked or a hex typed and committed: exactly one reveal.
// Reduced motion: none at all. The assertions are on the class and the node that carry the
// reveal, not on how long it takes.
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { ColorField } from "@/components/ColorField";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const captured = new Set<number>();

beforeAll(() => {
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
  Element.prototype.setPointerCapture = (id: number) => void captured.add(id);
  Element.prototype.releasePointerCapture = (id: number) => void captured.delete(id);
  Element.prototype.hasPointerCapture = (id: number) => captured.has(id);
  window.matchMedia ??= ((query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
  Element.prototype.scrollIntoView = () => {};
});

afterEach(() => {
  cleanup();
  captured.clear();
  document.documentElement.removeAttribute("data-motion");
  vi.useRealTimers();
});

/** The field that holds a value, the way a property control does. */
function Host() {
  const [value, setValue] = useState("#ff0000");
  return (
    <ColorField
      label="Colour"
      value={value}
      swatches={[
        { hex: "#00ff00", label: "Ink" },
        { hex: "#0000ff", label: "Paint" },
      ]}
      onChange={setValue}
    />
  );
}

/** Opens the picker and gives the square a box, which jsdom does not lay out. */
function openPicker(container: HTMLElement): HTMLDivElement {
  fireEvent.click(container.querySelector(".color-swatch") as HTMLButtonElement);
  const square = document.querySelector(".picker-sv") as HTMLDivElement;
  square.getBoundingClientRect = () =>
    ({ left: 0, top: 0, right: 100, bottom: 100, width: 100, height: 100, x: 0, y: 0, toJSON() {} }) as DOMRect;
  return square;
}

/** Every reveal the field swatch starts: one overlay node per reveal. */
const fieldWipes = () => document.querySelectorAll(".color-wipe").length;
/** The picker's preview, and whether it carries the reveal. */
const preview = () => document.querySelector(".picker-preview") as HTMLElement;
const previewWipes = () => document.querySelectorAll(".picker-preview.wipe").length;

describe("#369: a stream of colour changes never replays the reveal", () => {
  it("a drag of eight values starts no wipe on the swatch and none on the preview", () => {
    vi.useFakeTimers();
    const { container } = render(<Host />);
    const square = openPicker(container);
    const first = preview();
    expect(first).toBeTruthy();
    expect(fieldWipes()).toBe(0);

    fireEvent.pointerDown(square, { clientX: 8, clientY: 8, pointerId: 1 });
    for (const [x, y] of [
      [20, 20],
      [32, 26],
      [44, 32],
      [56, 40],
      [64, 48],
      [72, 58],
      [84, 66],
      [92, 74],
    ]) {
      // A frame apart, the way a real drag streams its values.
      act(() => {
        vi.advanceTimersByTime(16);
      });
      fireEvent.pointerMove(square, { clientX: x, clientY: y, pointerId: 1 });
      expect(fieldWipes(), `no overlay after the drag reached ${x},${y}`).toBe(0);
      expect(previewWipes(), `the preview did not replay at ${x},${y}`).toBe(0);
      expect(preview(), "the preview was not replaced mid-drag").toBe(first);
    }
    fireEvent.pointerUp(square, { clientX: 92, clientY: 74, pointerId: 1 });
    expect(fieldWipes()).toBe(0);
  });

  it("a quick swatch clicked starts exactly one wipe", () => {
    vi.useFakeTimers();
    const { container } = render(<Host />);
    openPicker(container);
    fireEvent.click(document.querySelectorAll(".picker-swatch")[1] as HTMLButtonElement);
    expect(fieldWipes(), "one choice, one reveal").toBe(1);
    expect(previewWipes(), "the preview reveals the chosen colour").toBe(1);
  });

  it("a hex typed and committed starts exactly one wipe", () => {
    vi.useFakeTimers();
    const { container } = render(<Host />);
    openPicker(container);
    const field = document.querySelector(".picker-hex") as HTMLInputElement;
    fireEvent.change(field, { target: { value: "#12ab34" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(fieldWipes(), "one commit, one reveal").toBe(1);
    expect(previewWipes()).toBe(1);
  });

  it("with reduced motion a change starts no wipe at all", () => {
    vi.useFakeTimers();
    document.documentElement.setAttribute("data-motion", "reduced");
    const { container } = render(<Host />);
    openPicker(container);
    fireEvent.click(document.querySelectorAll(".picker-swatch")[1] as HTMLButtonElement);
    expect(fieldWipes(), "reduced motion draws no overlay").toBe(0);
    expect(previewWipes(), "reduced motion replays nothing").toBe(0);
  });
});
