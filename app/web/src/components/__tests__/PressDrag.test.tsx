import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi, type Mock } from "vitest";
import { act, useState } from "react";
import { AngleDial } from "@/components/AngleDial";
import { ColorPicker } from "@/components/ColorPicker";
import { HueRing } from "@/components/HueRing";
import { NumberField } from "@/components/NumberField";
import { RailResizeHandle } from "@/components/RailResizeHandle";
import { Slider } from "@/components/Slider";
import type { CurvePoint } from "@/proto/CurvePoint";
import type { StackLayer } from "@/proto/StackLayer";
import { FanCurveEditor } from "@/views/FanCurveEditor";
import { PreviewStage } from "@/views/PreviewStage";
import { Stage } from "@/views/effects/Stage";
import { StackAreaPicker } from "@/views/effects/StackAreaPicker";

// The class test for `cause:ui-interaction` (#105): one rule over every control the pointer
// drags. On each drag surface a press is defaultPrevented (so Chromium's compatibility
// mousedown never starts a native drag or a text selection), and it still moves focus the way
// the native press did — a focused field outside the surface loses focus (a NumberField's
// draft commits on blur), because canceling the press canceled the mousedown that carried it.
// fireEvent returns false when the default was prevented.

// The stack preview's area frame draws a live picture; the test needs only the frame.
vi.mock("@/lib/stackPreview", () => ({
  onStackFrame: () => () => {},
  setStackPreviewStack: () => {},
  startStackPreview: () => {},
  stopStackPreview: () => {},
}));

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
  // On Element, not HTMLElement: the SVG surfaces (curve points, the area picker's desk) take
  // pointer capture too.
  Element.prototype.setPointerCapture = (id: number) => void captured.add(id);
  Element.prototype.releasePointerCapture = (id: number) => void captured.delete(id);
  Element.prototype.hasPointerCapture = (id: number) => captured.has(id);
  // jsdom has none; the motion hooks and every drag surface measure themselves with these.
  window.matchMedia ??= ((query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
});

afterEach(cleanup);

/** A slot 114 px wide at x=0: with the 14 px cap, 100 px of travel, so clientX 7 + n is n %. */
function measureSlider(el: HTMLDivElement): void {
  el.getBoundingClientRect = () => ({ left: 0, width: 114, top: 0, right: 114, bottom: 24, height: 24, x: 0, y: 0, toJSON() {} });
}

function renderSlider(disabled = false): HTMLDivElement {
  const { container } = render(
    <Slider value={0} min={0} max={100} step={25} label="Key response" disabled={disabled} onChange={vi.fn()} />,
  );
  const el = container.querySelector(".slider") as HTMLDivElement;
  measureSlider(el);
  return el;
}

function renderHueRing(): HTMLDivElement {
  const { container } = render(<HueRing value={0} label="Angle" onChange={vi.fn()} />);
  return container.querySelector(".hue-ring") as HTMLDivElement;
}

/** A dial 100 px square at x=0, y=0, like the AngleDial test's own measure. */
function renderAngleDial(): HTMLDivElement {
  const { container } = render(<AngleDial value={0} label="Angle" onChange={vi.fn()} />);
  const el = container.querySelector(".angle-dial") as HTMLDivElement;
  el.getBoundingClientRect = () => ({ left: 0, width: 100, top: 0, right: 100, bottom: 100, height: 100, x: 0, y: 0, toJSON() {} });
  return el;
}

function renderColorPicker(): { sv: HTMLDivElement; hue: HTMLDivElement } {
  const { container } = render(<ColorPicker label="Colour" value="#ff0000" onChange={vi.fn()} />);
  return {
    sv: container.querySelector(".picker-sv") as HTMLDivElement,
    hue: container.querySelector(".picker-hue") as HTMLDivElement,
  };
}

describe("PressDrag: a press never starts a native drag or a text selection (#105)", () => {
  it("the slider's press is prevented, its dragstart is refused, and it is not draggable", () => {
    const el = renderSlider();
    expect(fireEvent.pointerDown(el, { clientX: 7 + 20, pointerId: 1 })).toBe(false);
    fireEvent.pointerUp(el, { pointerId: 1 });
    expect(fireEvent.dragStart(el)).toBe(false);
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

  it("the angle dial's press is prevented, its dragstart is refused, and it is not draggable", () => {
    const el = renderAngleDial();
    expect(fireEvent.pointerDown(el, { clientX: 100, clientY: 50, pointerId: 1 })).toBe(false);
    fireEvent.pointerUp(el, { pointerId: 1 });
    expect(fireEvent.dragStart(el)).toBe(false);
    expect(el.getAttribute("draggable")).toBe("false");
  });

  it("the angle dial's press is prevented and still blurs a focused field", () => {
    const onBlur: Mock = vi.fn();
    const { container } = render(
      <>
        <input aria-label="Field" onBlur={onBlur} />
        <AngleDial value={0} label="Angle" onChange={vi.fn()} />
      </>,
    );
    const input = container.querySelector("input") as HTMLInputElement;
    act(() => input.focus());
    const el = container.querySelector(".angle-dial") as HTMLDivElement;
    el.getBoundingClientRect = () => ({ left: 0, width: 100, top: 0, right: 100, bottom: 100, height: 100, x: 0, y: 0, toJSON() {} });
    expect(fireEvent.pointerDown(el, { clientX: 100, clientY: 50, pointerId: 1 })).toBe(false);
    expect(onBlur).toHaveBeenCalled();
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

  it("pressing the slider still blurs a focused field: the draft commits", () => {
    const commit = vi.fn();
    function Host() {
      const [v, setV] = useState(30);
      return (
        <>
          <NumberField value={v} min={0} max={100} label="Opacity value" onCommit={(n) => { commit(n); setV(n); }} />
          <Slider value={v} min={0} max={100} step={25} label="Key response" onChange={() => {}} />
        </>
      );
    }
    const { container } = render(<Host />);
    const input = container.querySelector("input") as HTMLInputElement;
    act(() => input.focus());
    // A field with no draft commits nothing on blur; a typed draft commits.
    fireEvent.change(input, { target: { value: "45" } });
    const el = container.querySelector(".slider") as HTMLDivElement;
    measureSlider(el);
    fireEvent.pointerDown(el, { clientX: 7 + 20, pointerId: 1 });
    fireEvent.pointerUp(el, { pointerId: 1 });
    expect(commit).toHaveBeenCalledWith(45);
    expect(document.activeElement).toBe(el);
  });

  it("the layout stage's empty-desk press is prevented and still blurs a focused field", () => {
    const onBlur: Mock = vi.fn();
    const { container } = render(
      <>
        <input aria-label="Field" onBlur={onBlur} />
        <PreviewStage
          zones={[]}
          landmarks={[]}
          devices={null}
          selection={[]}
          onSelect={vi.fn()}
          onChangeZones={vi.fn()}
          onChangeLandmark={vi.fn()}
          onChangeParts={vi.fn()}
          onDrop={vi.fn()}
          dropPayloadType="layout-zone"
          layoutId="t1"
          mode="lights"
          onMode={vi.fn()}
          unit="cm"
        />
      </>,
    );
    const input = container.querySelector("input") as HTMLInputElement;
    act(() => input.focus());
    const el = container.querySelector(".stage") as HTMLDivElement;
    expect(fireEvent.pointerDown(el, { clientX: 10, clientY: 10, button: 0, pointerId: 1 })).toBe(false);
    expect(onBlur).toHaveBeenCalled();
  });

  it("the area picker's desk press is prevented and still blurs a focused field", () => {
    const onBlur: Mock = vi.fn();
    const layer: StackLayer = {
      id: "l1",
      name: "One",
      effect: "gradient",
      props: {},
      area: { kind: "canvas" },
      blend: "normal",
      opacity: 100,
      visible: true,
    };
    const { container } = render(
      <>
        <input aria-label="Field" onBlur={onBlur} />
        <StackAreaPicker layer={layer} layout={null} devices={null} onUse={vi.fn()} onClose={vi.fn()} />
      </>,
    );
    const input = container.querySelector("input") as HTMLInputElement;
    act(() => input.focus());
    const el = container.querySelector(".rd-desk") as SVGSVGElement;
    expect(fireEvent.pointerDown(el, { clientX: 10, clientY: 10, button: 0, pointerId: 1 })).toBe(false);
    expect(onBlur).toHaveBeenCalled();
  });

  it("the fan curve point's press is prevented and still blurs a focused field", () => {
    const onBlur: Mock = vi.fn();
    const points: CurvePoint[] = [
      { temp_c: 40, percent: 30 },
      { temp_c: 60, percent: 50 },
    ];
    const { container } = render(
      <>
        <input aria-label="Field" onBlur={onBlur} />
        <FanCurveEditor header="CPU" points={points} floor={0} currentTemp={null} currentDuty={null} onChange={vi.fn()} onCommit={vi.fn()} />
      </>,
    );
    const input = container.querySelector("input") as HTMLInputElement;
    act(() => input.focus());
    const el = container.querySelector(".curve-point") as SVGCircleElement;
    expect(fireEvent.pointerDown(el, { clientX: 10, clientY: 10, pointerId: 1 })).toBe(false);
    expect(onBlur).toHaveBeenCalled();
  });

  it("the rail's resize handle is prevented, refuses a dragstart and is not draggable", () => {
    // The handle is drawn where the rail has room to move: a window wider than the fitted page.
    Object.defineProperty(window, "innerWidth", { value: 1440, configurable: true, writable: true });
    const { container } = render(<RailResizeHandle />);
    const el = container.querySelector(".rail-resize") as HTMLDivElement;
    expect(fireEvent.pointerDown(el, { clientX: 200, pointerId: 1 })).toBe(false);
    fireEvent.pointerUp(el, { clientX: 200, pointerId: 1 });
    expect(fireEvent.dragStart(el)).toBe(false);
    expect(el.getAttribute("draggable")).toBe("false");
  });

  it("the stack stage's area frame is prevented, refuses a dragstart and still blurs a focused field", () => {
    const onBlur: Mock = vi.fn();
    const layer = {
      id: "l1",
      name: "Heat",
      effect: "fire",
      props: {},
      blend: "add",
      opacity: 70,
      visible: true,
      area: { kind: "rect", x: 16, y: 150, width: 110, height: 46 },
    } as unknown as StackLayer;
    const { container } = render(
      <>
        <input aria-label="Field" onBlur={onBlur} />
        <Stage
          stack={{ id: "stack:T", layers: [layer] }}
          layout={null}
          devices={null}
          picked={layer}
          parts
          onSelect={() => undefined}
          onAreaChange={vi.fn()}
        />
      </>,
    );
    const stage = container.querySelector(".rd-canvas") as HTMLElement;
    stage.getBoundingClientRect = () => ({ left: 0, top: 0, width: 640, height: 400, right: 640, bottom: 400, x: 0, y: 0, toJSON() {} });
    const input = container.querySelector("input") as HTMLInputElement;
    act(() => input.focus());
    const el = container.querySelector(".rd-area-frame") as HTMLElement;
    // Inside the rectangle: a body press.
    expect(fireEvent.pointerDown(el, { clientX: 142, clientY: 346, button: 0, pointerId: 1 })).toBe(false);
    expect(onBlur).toHaveBeenCalled();
    fireEvent.pointerUp(el, { pointerId: 1 });
    expect(fireEvent.dragStart(el)).toBe(false);
    expect(el.getAttribute("draggable")).toBe("false");
  });
});
