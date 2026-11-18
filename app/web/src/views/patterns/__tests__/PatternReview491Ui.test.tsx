// Behaviours the reviews of PR #491 proved (review pass 3); kept so no later fix pass regresses them.
// The app's own paths: the mid-write edit (B1) with the browser's own timing, the Devices-change
// rule, the slider gate (pointer cancel, lost capture, a drag that ends outside the window, key
// repeat) and the Motion segmented control's history entry.

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
import { getSnap, patchSnap } from "@/lib/store";
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

const draft = (): Pattern => api.putPatternDraft.mock.calls.at(-1)![0].pattern;

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

/** The light the daemon answers with on the next PATCH, and the list it then reports. */
const lightAs = (arrangement: "strip" | "ring") => {
  api.lights.patch.mockResolvedValue({ ...fanLight(arrangement) });
  api.lights.list.mockResolvedValue({ lights: [fanLight(arrangement)] });
};

const spin = (t: number): Pattern["motion"] => ({ type: "spin", turns_per_s: t }) as Pattern["motion"];

describe("#491 the undo's light write in the app", () => {
  it("B1: an edit that lands while the undo's light write is in flight leaves the light with the pattern", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    lightAs("ring");
    show(blank(8, "strip"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    await frames();
    expect(screen.getByText("ring of 8")).toBeTruthy();
    expect(getSnap().lights?.[0]?.light.arrangement).toBe("ring");

    // The undo's PATCH hangs; the light comes back as a strip once it lands.
    let land: () => void = () => {};
    api.lights.patch.mockImplementation(
      () =>
        new Promise((resolve) => {
          land = () => resolve({ ...fanLight("strip") });
        }),
    );
    api.lights.list.mockResolvedValue({ lights: [fanLight("strip")] });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    // An edit lands while the write is in flight: "Fill all" is one history entry.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fill all" }));
    });
    await act(async () => {
      land();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    // The pattern kept its shape, so the light must be in that shape too.
    expect(screen.getByText("ring of 8")).toBeTruthy();
    const patches = api.lights.patch.mock.calls.map((c) => (c[1] as { arrangement: string }).arrangement);
    // The undo wrote "strip"; the forward write must put the light back to "ring".
    expect(patches).toContain("strip");
    expect(patches.at(-1)).toBe("ring");
  });

  it("B1-browser: the same race with the browser's own timing leaves the light out of step", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    // The daemon's own truth, and the store the editor reads.
    let daemon: "strip" | "ring" = "strip";
    let hold = false;
    let land: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      land = resolve;
    });
    api.lights.patch.mockImplementation(async (_id: string, patch: { arrangement: string }) => {
      if (hold && patch.arrangement === "strip") await held;
      daemon = patch.arrangement as "strip" | "ring";
      return { ...fanLight(daemon) };
    });
    api.lights.list.mockImplementation(async () => ({ lights: [fanLight(daemon)] }));
    show(blank(8, "strip"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    await frames();
    expect(getSnap().lights?.[0]?.light.arrangement).toBe("ring");

    hold = true;
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fill all" }));
    });

    // The browser: the daemon's answer arrives in a microtask, and the passive effect that would
    // refresh the editor's copy of the light is a macrotask (React schedules it through the
    // Scheduler's MessageChannel), so it has not run when the undo's promise settles.
    land();
    for (let i = 0; i < 60; i++) await Promise.resolve();

    const patches = api.lights.patch.mock.calls.map((c) => (c[1] as { arrangement: string }).arrangement);
    // The undo wrote "strip"; the forward write must put the light back to the pattern's "ring".
    expect(patches).toContain("strip");
    expect(patches.at(-1)).toBe("ring");
  });

  it("Devices: a light changed elsewhere is left alone, with the reason in the note row", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    lightAs("ring");
    show(blank(8, "strip"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    await frames();
    expect(screen.getByText("ring of 8")).toBeTruthy();

    // The owner changes the light in Devices: the store now says it is a strip.
    patchSnap({ lights: [fanLight("strip")] });
    await settle();
    api.lights.patch.mockClear();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();

    expect(api.lights.patch).not.toHaveBeenCalled();
    expect(screen.getByText(/changed elsewhere/)).toBeTruthy();
    expect(screen.getByText("ring of 8")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false);
  });
});

describe("#491 the spin slider's history gate", () => {
  const sliderOf = () => screen.getByRole("slider", { name: "Spin speed, turns per second" }) as HTMLDivElement;

  /** A press on the cap and a drag across many 0.05 steps. */
  const drag = (el: HTMLDivElement, from = 210, to = 320) => {
    fireEvent.pointerDown(el, { pointerId: 1, clientX: from });
    for (let x = from + 10; x <= to; x += 10) fireEvent.pointerMove(el, { pointerId: 1, clientX: x });
  };

  it("G: one drag is one entry, so a drag cannot push the paint out of the history", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fill all" }));
    });
    await frames();
    const filled = draft().frames[0]!.colors;

    await act(async () => {
      drag(slider);
      fireEvent.pointerUp(slider, { pointerId: 1, clientX: 320 });
    });
    await frames();
    expect((draft().motion as unknown as { turns_per_s: number }).turns_per_s).toBeGreaterThan(0.25);

    // One press puts the whole drag back, and the fill is still undoable behind it.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.25));
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().frames[0]!.colors).not.toEqual(filled);
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("a pointer cancel ends the drag and leaves exactly one entry", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      drag(slider);
      fireEvent.pointerCancel(slider, { pointerId: 1 });
    });
    await frames();
    expect((draft().motion as unknown as { turns_per_s: number }).turns_per_s).toBeGreaterThan(0.25);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.25));
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("a lost pointer capture ends the drag and leaves exactly one entry", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      drag(slider);
      slider.dispatchEvent(new PointerEvent("lostpointercapture", { bubbles: true, pointerId: 1 }));
    });
    await frames();
    expect((draft().motion as unknown as { turns_per_s: number }).turns_per_s).toBeGreaterThan(0.25);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.25));
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("a drag that ends outside the window leaves exactly one entry", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      drag(slider);
      // The pointer is released far outside the editor's box.
      fireEvent.pointerUp(slider, { pointerId: 1, clientX: 9999 });
    });
    await frames();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.25));
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("key repeat: 95 presses up and 95 down are one entry, so a held arrow key cannot push the paint out of the history", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fill all" }));
    });
    await frames();
    const filled = draft().frames[0]!.colors;

    // A held arrow key: the browser repeats the key, and each repeat is its own task, so the
    // editor re-renders between them.
    for (let k = 0; k < 95; k++) {
      await act(async () => {
        fireEvent.keyDown(slider, { key: "ArrowUp" });
      });
    }
    for (let k = 0; k < 95; k++) {
      await act(async () => {
        fireEvent.keyDown(slider, { key: "ArrowDown" });
      });
    }
    await act(async () => {
      fireEvent.keyUp(slider, { key: "ArrowUp" });
      fireEvent.keyUp(slider, { key: "ArrowDown" });
    });
    await frames();

    // The keys are one entry: one Undo puts the motion back, and the fill is still on the frame,
    // not pushed out of the 100-deep history.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.25));
    expect(draft().frames[0]!.colors).toEqual(filled);
  });

  it("the Motion segmented control: the Off that follows a Spin is not in the history", async () => {
    show(blank(12, "ring"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Spin" }));
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.25));
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false);

    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Off" }));
    });
    await frames();
    expect(draft().motion).toEqual({ type: "none" });

    // One press must put the spin back: the Off is a change of its own.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.25));
    // And the history holds nothing else.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
