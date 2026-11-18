// Behaviours the reviews of PR #491 proved (review pass 4); kept so no later fix pass regresses them.
// The unmount guard: after the editor is gone, no draft PUT, no light PATCH and no state update
// on any path (undo, redo, shape choice, motion, paint, gradient).

const { api } = vi.hoisted(() => ({
  api: {
    putPatternDraft: vi.fn(),
    deleteDraft: vi.fn(),
    putPattern: vi.fn(),
    lights: { patch: vi.fn(), list: vi.fn() },
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { LightView } from "@/proto/LightView";
import type { Pattern } from "@/proto/Pattern";
import { patchSnap } from "@/lib/store";
import { PatternEditor } from "@/views/patterns/PatternEditor";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
const BOX = 400;

let pending = new Map<number, FrameRequestCallback>();
let nextFrame = 0;

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
});

beforeEach(() => {
  pending = new Map();
  nextFrame = 0;
  window.matchMedia = ((query: string) =>
    ({
      matches: false,
      media: query,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList);
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    nextFrame += 1;
    pending.set(nextFrame, cb);
    return nextFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => void pending.delete(id));
  HTMLCanvasElement.prototype.getContext = (() => ({
    setTransform() {},
    clearRect() {},
    beginPath() {},
    arc() {},
    stroke() {},
    fill() {},
    fillStyle: "",
    strokeStyle: "",
  })) as unknown as typeof HTMLCanvasElement.prototype.getContext;
  Element.prototype.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: BOX, height: BOX, right: BOX, bottom: BOX, x: 0, y: 0, toJSON() {} }) as DOMRect;
  api.putPatternDraft.mockReset().mockResolvedValue(status());
  api.deleteDraft.mockReset().mockResolvedValue(status({ active: false }));
  api.putPattern.mockReset();
  api.lights.patch.mockReset();
  api.lights.list.mockReset();
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
  patchSnap({ lights: null, devices: null });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  patchSnap({ lights: null, devices: null });
});

function status(over: Partial<DraftStatus> = {}): DraftStatus {
  return { active: true, props: {}, lease_remaining_ms: 15_000, heartbeat_ms: 5_000, lease_ms: 15_000, ...over };
}

function blank(leds: number, arrangement: "ring" | "strip" = "ring", motion: Pattern["motion"] = { type: "none" }): Pattern {
  return {
    id: "pattern:Fan",
    built_on: { led_count: leds, arrangement },
    frames: [{ colors: Array.from({ length: leds }, () => OFF), hold: 0 }],
    motion,
  };
}

function show(pattern: Pattern) {
  return render(<PatternEditor pattern={pattern} onSave={vi.fn()} onCancel={vi.fn()} />);
}

const settle = () => act(async () => {});

async function frames(now = 0): Promise<void> {
  const queued = [...pending.values()];
  pending.clear();
  await act(async () => {
    for (const cb of queued) cb(now);
  });
}

const fanLight = (arrangement: "strip" | "ring" | "grid" = "strip"): LightView => ({
  light: { id: "fake:a#z0", device_id: "fake:a", zone_index: 0, arrangement, reversed: false } as LightView["light"],
  led_count: 8,
  connected: true,
  label: "Fan 1",
});
const fanDevice = (kind: "linear" | "matrix" = "linear"): DeviceInfo =>
  ({
    id: "fake:a",
    name: "Fan",
    zones: [
      {
        index: 0,
        name: "zone0",
        kind: kind === "linear" ? { type: "linear" } : { type: "matrix", width: 4, height: 2, map: [] },
        led_count: 8,
        min_leds: 8,
        max_leds: 8,
        resizable: false,
      },
    ],
    connected: true,
  }) as unknown as DeviceInfo;

describe("#491 the unmount guard", () => {
  it("U1: the undo's write in flight: the editor closes, and nothing goes on the lights afterwards", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    api.lights.patch.mockResolvedValue({ ...fanLight("ring") });
    api.lights.list.mockResolvedValue({ lights: [fanLight("ring")] });
    const errors: string[] = [];
    const spy = vi.spyOn(console, "error").mockImplementation((...a: unknown[]) => {
      errors.push(String(a[0]));
    });
    const view = show(blank(8, "strip"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    await frames();
    expect(screen.getByText("ring of 8")).toBeTruthy();

    let land: () => void = () => {};
    api.lights.patch.mockReset().mockImplementation(
      () =>
        new Promise((resolve) => {
          land = () => resolve({ ...fanLight("strip") });
        }),
    );
    api.lights.list.mockResolvedValue({ lights: [fanLight("strip")] });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    const patchesBefore = api.lights.patch.mock.calls.length;
    view.unmount();
    await settle();
    expect(api.deleteDraft).toHaveBeenCalled();
    api.putPatternDraft.mockClear();

    await act(async () => {
      land();
      for (let i = 0; i < 40; i++) await Promise.resolve();
    });
    await frames();
    expect(api.putPatternDraft).not.toHaveBeenCalled();
    // No new light write either, and no React complaint about the dead editor.
    expect(api.lights.patch.mock.calls.length).toBe(patchesBefore);
    expect(errors).toEqual([]);
    spy.mockRestore();
  });

  it("U2: the redo's write in flight: the editor closes, and nothing goes on the lights afterwards", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    api.lights.patch.mockResolvedValue({ ...fanLight("ring") });
    api.lights.list.mockResolvedValue({ lights: [fanLight("ring")] });
    const view = show(blank(8, "strip"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    await frames();
    // Undo it, so the redo stack holds the shape step.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect((screen.getByRole("button", { name: "Redo" }) as HTMLButtonElement).disabled).toBe(false);

    let land: () => void = () => {};
    api.lights.patch.mockReset().mockImplementation(
      () =>
        new Promise((resolve) => {
          land = () => resolve({ ...fanLight("ring") });
        }),
    );
    api.lights.list.mockResolvedValue({ lights: [fanLight("ring")] });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Redo" }));
    });
    const patchesBefore = api.lights.patch.mock.calls.length;
    view.unmount();
    await settle();
    api.putPatternDraft.mockClear();

    await act(async () => {
      land();
      for (let i = 0; i < 40; i++) await Promise.resolve();
    });
    await frames();
    expect(api.putPatternDraft).not.toHaveBeenCalled();
    expect(api.lights.patch.mock.calls.length).toBe(patchesBefore);
  });

  it("U7: two queued shape undos: the editor closes during the first write, and the second never goes out (r5 B9)", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    api.lights.patch.mockResolvedValue({ ...fanLight("ring") });
    api.lights.list.mockResolvedValue({ lights: [fanLight("ring")] });
    const view = show(blank(8, "strip"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    await frames();
    api.lights.patch.mockResolvedValue({ ...fanLight("strip") });
    api.lights.list.mockResolvedValue({ lights: [fanLight("strip")] });
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Line" }));
    });
    await frames();

    let land: () => void = () => {};
    api.lights.patch.mockReset().mockImplementation(
      () =>
        new Promise((resolve) => {
          land = () => resolve({ ...fanLight("ring") });
        }),
    );
    api.lights.list.mockResolvedValue({ lights: [fanLight("ring")] });
    // Two presses inside one PATCH round trip: the first writes, the second waits in the lane.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    const patchesBefore = api.lights.patch.mock.calls.length;
    expect(patchesBefore).toBe(1);
    view.unmount();
    await settle();

    await act(async () => {
      land();
      for (let i = 0; i < 40; i++) await Promise.resolve();
    });
    await frames();
    // The queued second undo must not write the light after the editor is gone.
    expect(api.lights.patch.mock.calls.length).toBe(patchesBefore);
  });

  it("U3: the shape choice's PATCH in flight: the editor closes, and the pattern does not change", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    api.lights.patch.mockResolvedValue({ ...fanLight("strip") });
    api.lights.list.mockResolvedValue({ lights: [fanLight("strip")] });
    let land: () => void = () => {};
    api.lights.patch.mockImplementation(
      () =>
        new Promise((resolve) => {
          land = () => resolve({ ...fanLight("ring") });
        }),
    );
    const view = show(blank(8, "strip"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    const patchesBefore = api.lights.patch.mock.calls.length;
    view.unmount();
    await settle();
    api.putPatternDraft.mockClear();
    await act(async () => {
      land();
      for (let i = 0; i < 40; i++) await Promise.resolve();
    });
    await frames();
    // The shape's PATCH went out before the close; after it lands, nothing may follow.
    expect(api.lights.patch.mock.calls.length).toBe(patchesBefore);
    expect(api.putPatternDraft).not.toHaveBeenCalled();
  });

  it("U4: a motion change with the editor closed before the frame: no draft", async () => {
    const view = show(blank(12, "ring"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Spin" }));
    });
    view.unmount();
    await frames();
    api.putPatternDraft.mockClear();
    await settle();
    expect(api.putPatternDraft).not.toHaveBeenCalled();
  });

  it("U5: a paint with the editor closed before the frame: no draft", async () => {
    const view = show(blank(12, "ring"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fill all" }));
    });
    view.unmount();
    await frames();
    api.putPatternDraft.mockClear();
    await settle();
    expect(api.putPatternDraft).not.toHaveBeenCalled();
  });

  it("U6: the Gradient tool with the editor closed before the frame: no draft", async () => {
    const view = show(blank(12, "ring"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Gradient" }));
    });
    view.unmount();
    await frames();
    api.putPatternDraft.mockClear();
    await settle();
    expect(api.putPatternDraft).not.toHaveBeenCalled();
  });
});
