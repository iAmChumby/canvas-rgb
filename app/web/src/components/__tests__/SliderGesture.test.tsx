import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { Slider } from "@/components/Slider";

// The gesture contract (#491): `onBegin` runs when the first input (the pointer or a key) goes down
// and `onEnd` runs once, when the last of them has ended, the focus left a key, or the slider went
// away. A caller that keeps a history makes one entry per `onBegin`/`onEnd` pair.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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
  HTMLElement.prototype.setPointerCapture = (id: number) => void captured.add(id);
  HTMLElement.prototype.releasePointerCapture = (id: number) => void captured.delete(id);
  HTMLElement.prototype.hasPointerCapture = (id: number) => captured.has(id);
});

afterEach(cleanup);

function setup() {
  const log: string[] = [];
  const onBegin = vi.fn(() => void log.push("begin"));
  const onEnd = vi.fn(() => void log.push("end"));
  const view = render(
    <Slider value={50} min={0} max={100} step={5} label="Gesture" onChange={() => {}} onBegin={onBegin} onEnd={onEnd} />,
  );
  const el = view.container.querySelector(".slider") as HTMLDivElement;
  el.getBoundingClientRect = () => ({ left: 0, width: 114, top: 0, right: 114, bottom: 24, height: 24, x: 0, y: 0, toJSON() {} });
  return { el, log, onBegin, onEnd, view };
}

describe("the slider's gesture (#491)", () => {
  it("a held key, however long, is one begin and one end", () => {
    const { el, log } = setup();
    fireEvent.keyDown(el, { key: "ArrowUp" });
    for (let k = 0; k < 20; k++) fireEvent.keyDown(el, { key: "ArrowUp", repeat: true });
    fireEvent.keyUp(el, { key: "ArrowUp" });
    expect(log).toEqual(["begin", "end"]);
  });

  it("another key's release does not end the hold", () => {
    const { el, log } = setup();
    fireEvent.keyDown(el, { key: "ArrowUp" });
    fireEvent.keyUp(el, { key: "Shift" });
    fireEvent.keyUp(el, { key: "Tab" });
    expect(log).toEqual(["begin"]);
    fireEvent.keyUp(el, { key: "ArrowUp" });
    expect(log).toEqual(["begin", "end"]);
  });

  it("a key the slider ignores begins nothing", () => {
    const { el, log } = setup();
    fireEvent.keyDown(el, { key: "Shift" });
    fireEvent.keyUp(el, { key: "Shift" });
    expect(log).toEqual([]);
  });

  it("two keys held at once are one gesture that ends with the last release", () => {
    const { el, log } = setup();
    fireEvent.keyDown(el, { key: "ArrowUp" });
    fireEvent.keyDown(el, { key: "PageUp" });
    fireEvent.keyUp(el, { key: "ArrowUp" });
    expect(log).toEqual(["begin"]);
    fireEvent.keyUp(el, { key: "PageUp" });
    expect(log).toEqual(["begin", "end"]);
  });

  it("a pointer drag that ends while a key is held leaves the gesture open until the key is up", () => {
    const { el, log } = setup();
    fireEvent.keyDown(el, { key: "ArrowUp" });
    fireEvent.pointerDown(el, { pointerId: 1, clientX: 20 });
    fireEvent.pointerUp(el, { pointerId: 1, clientX: 40 });
    expect(log).toEqual(["begin"]);
    fireEvent.keyUp(el, { key: "ArrowUp" });
    expect(log).toEqual(["begin", "end"]);
  });

  it("a key released while the pointer drags leaves the gesture open until the pointer ends", () => {
    const { el, log } = setup();
    fireEvent.pointerDown(el, { pointerId: 1, clientX: 20 });
    fireEvent.keyDown(el, { key: "ArrowUp" });
    fireEvent.keyUp(el, { key: "ArrowUp" });
    expect(log).toEqual(["begin"]);
    fireEvent.pointerCancel(el, { pointerId: 1 });
    expect(log).toEqual(["begin", "end"]);
  });

  it("losing the focus with a key held ends the gesture, and a late keyup adds nothing", () => {
    const { el, log } = setup();
    fireEvent.keyDown(el, { key: "ArrowUp" });
    fireEvent.blur(el);
    expect(log).toEqual(["begin", "end"]);
    fireEvent.keyUp(el, { key: "ArrowUp" });
    expect(log).toEqual(["begin", "end"]);
  });

  it("a blur with nothing held ends nothing", () => {
    const { el, log } = setup();
    fireEvent.blur(el);
    expect(log).toEqual([]);
  });

  it("a blur during a pointer drag leaves the drag open until the pointer ends", () => {
    const { el, log } = setup();
    fireEvent.pointerDown(el, { pointerId: 1, clientX: 20 });
    fireEvent.blur(el);
    expect(log).toEqual(["begin"]);
    fireEvent.pointerUp(el, { pointerId: 1, clientX: 30 });
    expect(log).toEqual(["begin", "end"]);
  });

  it("a lost pointer capture ends the drag once", () => {
    const { el, log } = setup();
    fireEvent.pointerDown(el, { pointerId: 1, clientX: 20 });
    fireEvent.lostPointerCapture(el, { pointerId: 1 });
    fireEvent.pointerUp(el, { pointerId: 1, clientX: 30 });
    expect(log).toEqual(["begin", "end"]);
  });

  it("unmounting mid-gesture ends it, and an idle unmount ends nothing", () => {
    const held = setup();
    fireEvent.keyDown(held.el, { key: "ArrowUp" });
    held.view.unmount();
    expect(held.log).toEqual(["begin", "end"]);

    const idle = setup();
    idle.view.unmount();
    expect(idle.log).toEqual([]);
  });

  it("a gesture can begin again after it ended", () => {
    const { el, log } = setup();
    fireEvent.keyDown(el, { key: "ArrowUp" });
    fireEvent.keyUp(el, { key: "ArrowUp" });
    fireEvent.pointerDown(el, { pointerId: 1, clientX: 20 });
    fireEvent.pointerUp(el, { pointerId: 1, clientX: 20 });
    fireEvent.keyDown(el, { key: "ArrowDown" });
    fireEvent.blur(el);
    expect(log).toEqual(["begin", "end", "begin", "end", "begin", "end"]);
  });
});
