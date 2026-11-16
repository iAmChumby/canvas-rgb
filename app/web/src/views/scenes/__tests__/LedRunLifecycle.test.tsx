import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LedRun, runSignature } from "@/views/scenes/LedRun";
import type { LightView } from "@/proto/LightView";

// A run keeps its canvas, its observers and its loop while the page hands it a new array of lights
// that says the same thing (the lights are re-read every few seconds), and restarts when the
// lights really change.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const lights = (): LightView[] => [
  { light: { id: "a#z0", device_id: "a", zone_index: 0, arrangement: "strip", reversed: false }, led_count: 8, connected: true, label: "A" },
  { light: { id: "b#z0", device_id: "b", zone_index: 0, arrangement: "ring", reversed: false }, led_count: 16, connected: true, label: "B" },
];

let resizeObservers = 0;
let intersectionObservers = 0;
let rafCalls = 0;
let paints = 0;
let onScreen: (entries: Array<{ isIntersecting: boolean }>) => void = () => {};
let frame: ((t: number) => void) | null = null;

beforeEach(() => {
  resizeObservers = 0;
  intersectionObservers = 0;
  rafCalls = 0;
  paints = 0;
  frame = null;
  globalThis.ResizeObserver = class {
    constructor() {
      resizeObservers += 1;
    }
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  globalThis.IntersectionObserver = class {
    constructor(cb: (entries: Array<{ isIntersecting: boolean }>) => void) {
      intersectionObservers += 1;
      onScreen = cb;
    }
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof IntersectionObserver;
  const ctx = new Proxy(
    {},
    {
      get: (_t, name) => {
        if (name === "clearRect") return () => void (paints += 1);
        return () => {};
      },
      set: () => true,
    },
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
  vi.spyOn(HTMLCanvasElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 400, height: 60, top: 0, left: 0, right: 400, bottom: 60, x: 0, y: 0, toJSON: () => ({}),
  });
  vi.stubGlobal("requestAnimationFrame", (cb: (t: number) => void) => {
    rafCalls += 1;
    frame = cb;
    return rafCalls;
  });
  vi.stubGlobal("cancelAnimationFrame", () => {});
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("runSignature", () => {
  it("is equal for equal content and different when a light changes", () => {
    expect(runSignature(lights())).toBe(runSignature(lights()));
    const changed = lights();
    (changed[0] as LightView).led_count = 9;
    expect(runSignature(changed)).not.toBe(runSignature(lights()));
    const away = lights();
    (away[1] as LightView).connected = false;
    expect(runSignature(away)).not.toBe(runSignature(lights()));
  });
});

describe("a LED run", () => {
  it("keeps its observers when the lights are replaced with equal content", () => {
    const { rerender } = render(<LedRun lights={lights()} />);
    expect(resizeObservers).toBe(1);
    rerender(<LedRun lights={lights()} />);
    rerender(<LedRun lights={lights()} />);
    expect(resizeObservers).toBe(1);
    expect(intersectionObservers).toBe(1);
  });

  it("restarts when a light really changes", () => {
    const { rerender } = render(<LedRun lights={lights()} />);
    const changed = lights();
    (changed[0] as LightView).led_count = 12;
    rerender(<LedRun lights={changed} />);
    expect(resizeObservers).toBe(2);
  });

  it("does not paint while it is scrolled out of view, and paints again when it returns", () => {
    render(<LedRun lights={lights()} />);
    const first = paints;
    expect(first).toBeGreaterThan(0);
    onScreen([{ isIntersecting: false }]);
    const run = frame as ((t: number) => void) | null;
    run?.(1000);
    run?.(2000);
    expect(paints).toBe(first);
    onScreen([{ isIntersecting: true }]);
    (frame as ((t: number) => void) | null)?.(3000);
    expect(paints).toBeGreaterThan(first);
  });
});
