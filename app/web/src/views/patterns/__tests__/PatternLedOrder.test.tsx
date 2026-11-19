import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { LightView } from "@/proto/LightView";
import type { Pattern } from "@/proto/Pattern";

// #468 phase 3: the notch and direction arrow in the pattern editor's ring view. They write the
// light's LED order through the lights PATCH and identify the physical LED on the real device.

const { api } = vi.hoisted(() => ({
  api: {
    putPatternDraft: vi.fn(),
    deleteDraft: vi.fn(),
    lights: { patch: vi.fn(), list: vi.fn() },
    identify: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { patchSnap } from "@/lib/store";
import { layoutDots } from "@/views/patterns/dotLayout";
import { PatternEditor } from "@/views/patterns/PatternEditor";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
const BOX = 400;

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
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
  HTMLCanvasElement.prototype.getContext = (() => ({
    setTransform() {}, clearRect() {}, beginPath() {}, arc() {}, stroke() {}, fill() {},
    fillStyle: "", strokeStyle: "", lineWidth: 1,
  })) as unknown as typeof HTMLCanvasElement.prototype.getContext;
  Element.prototype.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: BOX, height: BOX, right: BOX, bottom: BOX, x: 0, y: 0, toJSON() {} }) as DOMRect;
  window.matchMedia = (() => ({
    matches: false, media: "", addEventListener() {}, removeEventListener() {},
  })) as unknown as typeof window.matchMedia;
  vi.stubGlobal("requestAnimationFrame", () => 1);
  vi.stubGlobal("cancelAnimationFrame", () => undefined);
});

beforeEach(() => {
  api.putPatternDraft.mockReset().mockResolvedValue({
    active: true, props: {}, lease_remaining_ms: 15_000, heartbeat_ms: 5_000, lease_ms: 15_000,
  } as DraftStatus);
  api.deleteDraft.mockReset().mockResolvedValue({ active: false });
  api.lights.patch.mockReset();
  api.lights.list.mockReset();
  api.identify.mockReset().mockResolvedValue({ ok: true });
  patchSnap({ lights: null, devices: null });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  patchSnap({ lights: null, devices: null });
});

function blank(leds = 8): Pattern {
  return {
    id: "pattern:Fan",
    built_on: { led_count: leds, arrangement: "ring" },
    frames: [{ colors: Array.from({ length: leds }, () => OFF), hold: 0 }],
    motion: { type: "none" },
  };
}

function light(first_led?: number, reversed = false, leds?: { start: number; count: number }): LightView {
  return {
    light: {
      id: "fake:a#z0", device_id: "fake:a", zone_index: 0, arrangement: "ring", reversed, first_led, leds,
    } as LightView["light"],
    led_count: 8,
    connected: true,
    label: "Fan 1",
  };
}

function device(): DeviceInfo {
  return {
    id: "fake:a",
    name: "Fan",
    zones: [{
      index: 0, name: "zone0", kind: { type: "linear" }, led_count: 8,
      min_leds: 8, max_leds: 8, resizable: false,
    }],
    connected: true,
  } as unknown as DeviceInfo;
}

function show(first_led?: number, reversed = false, leds?: { start: number; count: number }) {
  const view = light(first_led, reversed, leds);
  patchSnap({ lights: [view], devices: [device()] });
  api.lights.patch.mockResolvedValue(view);
  api.lights.list.mockResolvedValue({ lights: [view] });
  render(<PatternEditor pattern={blank()} onSave={vi.fn()} onCancel={vi.fn()} />);
}

const settle = () => act(async () => {});

describe("the LED-order notch in the pattern editor", () => {
  it("moves the first LED with the arrow keys and identifies it on the real device", async () => {
    show(0);
    await settle();
    const notch = screen.getByTestId("first-led-notch");
    expect(notch.getAttribute("aria-valuenow")).toBe("1");

    await act(async () => { fireEvent.keyDown(notch, { key: "ArrowRight" }); });
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { first_led: 1 });
    expect(api.identify).toHaveBeenCalledWith("fake:a", {
      zone: 0, led: 1, count: 1, stop: false,
    });

    await act(async () => { fireEvent.keyDown(notch, { key: "Home" }); });
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { first_led: 0 });
    expect(api.identify).toHaveBeenLastCalledWith("fake:a", {
      zone: 0, led: 0, count: 1, stop: false,
    });
  });

  it("drags the notch to the LED under the pointer", async () => {
    show(0);
    await settle();
    const notch = screen.getByTestId("first-led-notch");
    const target = layoutDots(blank().built_on, BOX, BOX)[5]!;
    await act(async () => {
      fireEvent.pointerDown(notch, { pointerId: 1 });
      fireEvent.pointerMove(notch, { pointerId: 1, clientX: target.x, clientY: target.y });
      fireEvent.pointerUp(notch, { pointerId: 1 });
    });
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { first_led: 5 });
  });

  it("toggles the run direction through the lights PATCH", async () => {
    show(0, false);
    await settle();
    await act(async () => { fireEvent.click(screen.getByTestId("led-order-direction")); });
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { reversed: true, first_led: 0 });
  });

  it("shows the marked LED and the current direction on the view", async () => {
    show(3, true);
    await settle();
    const view = screen.getByTestId("led-order");
    expect(view.getAttribute("data-first-led")).toBe("3");
    expect(view.getAttribute("data-reversed")).toBe("true");
    expect(screen.getByTestId("first-led-notch").getAttribute("aria-valuenow")).toBe("4");
  });

  it("names the two keys in words, and the direction key says whether the run is reversed (#468)", async () => {
    // The keys used to be two bare icons in the stage's corners; a reader of the screen (or of the
    // page) now finds them by what they do, with the LED number in the first one.
    show(2, false);
    await settle();
    expect(screen.getByRole("button", { name: "Find LED 3" })).toBe(screen.getByTestId("led-order-identify"));
    const reverse = screen.getByRole("button", { name: "Reverse" });
    expect(reverse.getAttribute("aria-pressed")).toBe("false");
    cleanup();
    show(2, true);
    await settle();
    expect(screen.getByRole("button", { name: "Reverse" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("sends the first LED with the direction, so the first LED stays first as it turns", async () => {
    // A reversed light with no first LED starts on its last LED (the old full flip); turning it
    // forward keeps that LED first instead of jumping to LED 0.
    show(undefined, true);
    await settle();
    expect(screen.getByTestId("first-led-notch").getAttribute("aria-valuenow")).toBe("8");
    await act(async () => { fireEvent.click(screen.getByTestId("led-order-direction")); });
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { reversed: false, first_led: 7 });
  });

  it("identifies the zone's LED, not the slice's, on a part that starts past the zone's first LED", async () => {
    // A chain part: its LEDs 0..7 are the zone's LEDs 12..19, and `first_led` counts inside the part.
    show(0, false, { start: 12, count: 8 });
    await settle();
    await act(async () => { fireEvent.keyDown(screen.getByTestId("first-led-notch"), { key: "ArrowRight" }); });
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { first_led: 1 });
    expect(api.identify).toHaveBeenLastCalledWith("fake:a", {
      zone: 0, led: 13, count: 1, stop: false,
    });
    await act(async () => { fireEvent.click(screen.getByTestId("led-order-identify")); });
    expect(api.identify).toHaveBeenLastCalledWith("fake:a", {
      zone: 0, led: 12, count: 1, stop: false,
    });
  });

  it("writes and identifies once for a drag across several LEDs, at the LED it settles on", async () => {
    show(0);
    await settle();
    const notch = screen.getByTestId("first-led-notch");
    const dots = layoutDots(blank().built_on, BOX, BOX);
    await act(async () => {
      fireEvent.pointerDown(notch, { pointerId: 1 });
      for (const i of [1, 2, 3, 4, 5]) {
        fireEvent.pointerMove(notch, { pointerId: 1, clientX: dots[i]!.x, clientY: dots[i]!.y });
      }
    });
    // The notch follows the pointer while it is down, and nothing is written yet.
    expect(screen.getByTestId("led-order").getAttribute("data-first-led")).toBe("5");
    expect(api.lights.patch).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.pointerUp(notch, { pointerId: 1 });
    });
    expect(api.lights.patch).toHaveBeenCalledTimes(1);
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { first_led: 5 });
    expect(api.identify).toHaveBeenCalledTimes(1);
  });
});
