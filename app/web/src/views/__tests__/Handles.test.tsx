// The stage's handles as a component (#257 review): the grip clamped inside the stage box, the
// undo run closed when a gesture is cut off by an unmount, and a period said as a speed.

import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EFFECTS } from "@/lib/fixtures";
import { FrameHandles } from "@/views/effects/Handles";
import type { StackLayer } from "@/proto/StackLayer";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(cleanup);

const schemaOf = (id: string) => {
  const found = EFFECTS.find((effect) => effect.schema.id === id);
  if (!found) throw new Error(`no demo schema for ${id}`);
  return found.schema;
};

/** A layer of `effect` with `props` over its schema's defaults. */
function layerOf(effect: string, props: StackLayer["props"]): StackLayer {
  return {
    id: "l1",
    name: "Test",
    effect,
    props,
    area: { kind: "canvas" },
    blend: "normal",
    opacity: 100,
    visible: true,
  };
}

/** The stage is 640 x 400 css pixels at (0, 100): two pixels per canvas unit. */
const STAGE = { left: 0, top: 100, right: 640, bottom: 500 };

/** The grip's centre, read from the arm's length and the handle's rotation. */
function gripCentre(container: HTMLElement, hub: { x: number; y: number }) {
  const arm = container.querySelector<HTMLElement>(".rd-angle-arm");
  const handle = container.querySelector<HTMLElement>(".rd-angle-handle");
  const length = parseFloat(arm?.style.width ?? "0");
  const match = /rotate\((-?[\d.]+)deg\)/.exec(handle?.style.transform ?? "");
  const dir = match ? Number(match[1]) : 0;
  const rad = (dir * Math.PI) / 180;
  return { x: hub.x + length * Math.cos(rad), y: hub.y + length * Math.sin(rad), length, dir };
}

describe("the grip's clamp inside the stage", () => {
  it("keeps the grip centre inside the stage at the left edge, with the angle true", () => {
    const hub = { x: 4, y: 446 };
    const view = render(
      <FrameHandles
        layer={layerOf("gradient", { angle: { type: "angle", value: 180 }, scroll_speed: { type: "number", value: 100 } })}
        schema={schemaOf("gradient")}
        centre={() => hub}
        stage={() => STAGE}
        onChange={() => {}}
      />,
    );
    const at = gripCentre(view.container, hub);
    expect(at.dir).toBe(180);
    expect(at.x).toBeGreaterThanOrEqual(STAGE.left);
    expect(at.x).toBeLessThanOrEqual(STAGE.right);
    expect(at.y).toBeGreaterThanOrEqual(STAGE.top);
    expect(at.y).toBeLessThanOrEqual(STAGE.bottom);
  });

  it("keeps the grip centre inside the stage at the right edge", () => {
    const hub = { x: 636, y: 446 };
    const view = render(
      <FrameHandles
        layer={layerOf("gradient", { angle: { type: "angle", value: 0 }, scroll_speed: { type: "number", value: 100 } })}
        schema={schemaOf("gradient")}
        centre={() => hub}
        stage={() => STAGE}
        onChange={() => {}}
      />,
    );
    const at = gripCentre(view.container, hub);
    expect(at.dir).toBe(0);
    expect(at.x).toBeGreaterThanOrEqual(STAGE.left);
    expect(at.x).toBeLessThanOrEqual(STAGE.right);
    expect(at.y).toBeGreaterThanOrEqual(STAGE.top);
    expect(at.y).toBeLessThanOrEqual(STAGE.bottom);
  });
});

describe("the direction a drag stores (#257 review)", () => {
  it("stores 0, not 360, when the grip is dragged just above the x-axis", () => {
    const onChange = vi.fn();
    const hub = { x: 320, y: 300 };
    const view = render(
      <FrameHandles
        layer={layerOf("gradient", { angle: { type: "angle", value: 0 } })}
        schema={schemaOf("gradient")}
        centre={() => hub}
        stage={() => STAGE}
        onChange={onChange}
      />,
    );
    const grip = view.container.querySelector<HTMLElement>(".rd-angle-grip");
    expect(grip).not.toBeNull();
    // A point 0.4° above the +x axis: atan2 gives -0.4°, which rounds to 0.
    const x = hub.x + 60 * Math.cos((-0.4 * Math.PI) / 180);
    const y = hub.y + 60 * Math.sin((-0.4 * Math.PI) / 180);
    fireEvent.pointerDown(grip!, { pointerId: 7, pointerType: "mouse", buttons: 1, clientX: hub.x + 16, clientY: hub.y });
    fireEvent.pointerMove(grip!, { pointerId: 7, pointerType: "mouse", buttons: 1, clientX: x, clientY: y });
    const stored = onChange.mock.calls.at(-1)?.[1];
    expect(stored?.type).toBe("angle");
    expect(stored?.value).toBe(0);
    expect(stored?.value).not.toBe(360);
  });
});

describe("a gesture cut off by an unmount", () => {
  it("ends the undo run once when the handle goes away mid-drag", () => {
    const onChangeEnd = vi.fn();
    const view = render(
      <FrameHandles
        layer={layerOf("gradient", { angle: { type: "angle", value: 0 } })}
        schema={schemaOf("gradient")}
        centre={() => ({ x: 100, y: 100 })}
        stage={() => STAGE}
        onChange={() => {}}
        onChangeEnd={onChangeEnd}
      />,
    );
    const grip = view.container.querySelector<HTMLElement>(".rd-angle-grip");
    expect(grip).not.toBeNull();
    fireEvent.pointerDown(grip!, { pointerId: 7, pointerType: "mouse", buttons: 1, clientX: 100, clientY: 100 });
    fireEvent.pointerMove(grip!, { pointerId: 7, pointerType: "mouse", buttons: 1, clientX: 120, clientY: 100 });
    expect(onChangeEnd).not.toHaveBeenCalled();
    view.unmount();
    expect(onChangeEnd).toHaveBeenCalledTimes(1);
  });
});

describe("the pill's value text", () => {
  it("says a period as a speed, so a short period reads as faster", () => {
    const fast = render(
      <FrameHandles
        layer={layerOf("breathing", { period: { type: "number", value: 1 } })}
        schema={schemaOf("breathing")}
        centre={() => null}
        onChange={() => {}}
      />,
    );
    expect(fast.container.querySelector(".rd-speed-pill")?.getAttribute("aria-valuetext")).toMatch(/^Speed: faster \(/);
    cleanup();
    const slow = render(
      <FrameHandles
        layer={layerOf("breathing", { period: { type: "number", value: 20 } })}
        schema={schemaOf("breathing")}
        centre={() => null}
        onChange={() => {}}
      />,
    );
    expect(slow.container.querySelector(".rd-speed-pill")?.getAttribute("aria-valuetext")).toMatch(/^Speed: slower \(/);
  });

  it("leaves a plain speed as it was", () => {
    const view = render(
      <FrameHandles
        layer={layerOf("rain", { speed: { type: "number", value: 120 } })}
        schema={schemaOf("rain")}
        centre={() => null}
        onChange={() => {}}
      />,
    );
    expect(view.container.querySelector(".rd-speed-pill")?.getAttribute("aria-valuetext")).toBe("Speed 120");
  });
});
