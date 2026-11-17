import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { LedOrderControl } from "@/components/LedOrderControl";

// #468: the ring drag in the Devices row. The row's box is 76 x 36 around a 132 x 132 view, so the
// picture is letterboxed with a band each side; the pointer must be read through it.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const BOX = { left: 40, top: 12, width: 76, height: 36 };

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
  const captured = new Set<number>();
  Element.prototype.setPointerCapture = (id: number) => void captured.add(id);
  Element.prototype.releasePointerCapture = (id: number) => void captured.delete(id);
  Element.prototype.hasPointerCapture = (id: number) => captured.has(id);
  Element.prototype.getBoundingClientRect = () =>
    ({
      ...BOX,
      right: BOX.left + BOX.width,
      bottom: BOX.top + BOX.height,
      x: BOX.left,
      y: BOX.top,
      toJSON() {},
    }) as DOMRect;
});

afterEach(cleanup);

describe("dragging the notch in the Devices row's letterboxed box", () => {
  it("lands on the LED under the pointer, for every LED of a 16 LED ring", async () => {
    const picked: number[] = [];
    const onFirstLed = vi.fn((led: number) => void picked.push(led));
    render(<LedOrderControl ledCount={16} firstLed={0} reversed={false} ring label="Fan" onFirstLed={onFirstLed} />);
    const svg = screen.getByTestId("led-order");
    // The drawn LEDs: the circles with no role (the handle is the slider).
    const leds = Array.from(svg.querySelectorAll("circle")).filter((c) => !c.getAttribute("role") && !c.closest("g"));
    expect(leds).toHaveLength(16);
    // The view is 132 x 132 in 76 x 36: scale 36 / 132, a band of (76 - 36) / 2 each side.
    const scale = 36 / 132;
    const bandX = (76 - 132 * scale) / 2;
    const notch = screen.getByTestId("first-led-notch");

    const wrong: number[] = [];
    for (let i = 0; i < leds.length; i += 1) {
      onFirstLed.mockClear();
      const cx = Number(leds[i]!.getAttribute("cx"));
      const cy = Number(leds[i]!.getAttribute("cy"));
      await act(async () => {
        fireEvent.pointerDown(notch, { pointerId: 1 });
        fireEvent.pointerMove(notch, {
          pointerId: 1,
          clientX: BOX.left + bandX + cx * scale,
          clientY: BOX.top + cy * scale,
        });
        fireEvent.pointerUp(notch, { pointerId: 1 });
      });
      const got = onFirstLed.mock.calls[0]?.[0] ?? (i === 0 ? 0 : -1);
      if (got !== i) wrong.push(i);
    }
    expect(wrong).toEqual([]);
  });
});
