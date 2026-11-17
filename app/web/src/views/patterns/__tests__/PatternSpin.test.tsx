import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { LightView } from "@/proto/LightView";
import type { Pattern } from "@/proto/Pattern";

// #468 phase 1 in the editor: the shape picker in the header (it writes the light's arrangement
// through the lights PATCH), the Gradient tool with Wrap, the Spin control saved as `Motion::spin`,
// and the preview turning with the daemon's own maths. The daemon is mocked; nothing waits for a
// real frame.

const { api } = vi.hoisted(() => ({
  api: {
    putPatternDraft: vi.fn(),
    deleteDraft: vi.fn(),
    putPattern: vi.fn(),
    lights: { patch: vi.fn(), list: vi.fn() },
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { gradientColours, seedStops } from "@/lib/gradient";
import { spinColours } from "@/lib/spin";
import { patchSnap } from "@/lib/store";
import { PatternEditor } from "@/views/patterns/PatternEditor";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
const BOX = 400;

let pending = new Map<number, FrameRequestCallback>();
let nextFrame = 0;
let reduced = false;

/** What the fake 2D context was asked to fill, one entry per dot, per frame. */
let fills: string[][] = [];

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
  reduced = false;
  fills = [];
  window.matchMedia = ((query: string) =>
    ({
      matches: reduced && query.includes("prefers-reduced-motion"),
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
  // A canvas that records the ink of every dot, one list per frame drawn.
  HTMLCanvasElement.prototype.getContext = (() => {
    let current: string[] = [];
    return {
      setTransform() {},
      clearRect() {
        current = [];
        fills.push(current);
      },
      beginPath() {},
      arc() {},
      stroke() {},
      fill() {
        current.push(String(this.fillStyle));
      },
      fillStyle: "",
      strokeStyle: "",
      lineWidth: 1,
    };
  }) as unknown as typeof HTMLCanvasElement.prototype.getContext;
  // Every element is measured at BOX x BOX before it renders, so the canvas and the handles'
  // overlay have a box to lay the dots in.
  Element.prototype.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: BOX, height: BOX, right: BOX, bottom: BOX, x: 0, y: 0, toJSON() {} }) as DOMRect;
  api.putPatternDraft.mockReset().mockResolvedValue(status());
  api.deleteDraft.mockReset().mockResolvedValue(status({ active: false }));
  api.putPattern.mockReset();
  api.lights.patch.mockReset();
  api.lights.list.mockReset();
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

/** A pattern of `leds` LEDs, all off, in `arrangement`. */
function blank(leds: number, arrangement: "ring" | "strip" = "ring", motion: Pattern["motion"] = { type: "none" }): Pattern {
  return {
    id: "pattern:Fan",
    built_on: { led_count: leds, arrangement },
    frames: [{ colors: Array.from({ length: leds }, () => OFF), hold: 0 }],
    motion,
  };
}

function show(pattern: Pattern, onSave: (p: Pattern) => void = vi.fn()) {
  return render(<PatternEditor pattern={pattern} onSave={onSave} onCancel={vi.fn()} />);
}

const settle = () => act(async () => {});

async function frames(now = 0): Promise<void> {
  const queued = [...pending.values()];
  pending.clear();
  await act(async () => {
    for (const cb of queued) cb(now);
  });
}

/** The newest document the editor put on the lights. */
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

describe("the shape picker in the editor's header", () => {
  it("writes the light's arrangement through the lights PATCH, then the pattern's shape", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    api.lights.patch.mockResolvedValue({ ...fanLight("ring") });
    api.lights.list.mockResolvedValue({ lights: [fanLight("ring")] });
    show(blank(8, "strip"));
    await settle();
    expect(screen.getByText("strip of 8")).toBeTruthy();

    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { arrangement: "ring" });
    await waitFor(() => expect(screen.getByText("ring of 8")).toBeTruthy());
    await frames();
    expect(draft().built_on.arrangement).toBe("ring");
  });

  it("leaves the pattern's shape alone, and says why, when the PATCH fails", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    api.lights.patch.mockRejectedValue(new Error("the light file could not be saved"));
    show(blank(8, "strip"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    expect(screen.getByText("strip of 8")).toBeTruthy();
    expect(screen.getByText(/could not be saved/)).toBeTruthy();
  });

  it("offers a grid only where the zone is one", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice("linear")] });
    const first = show(blank(8, "strip"));
    await settle();
    expect(screen.queryByRole("radio", { name: "Grid" })).toBeNull();
    expect(screen.getAllByRole("radio").map((r) => r.getAttribute("aria-label") ?? r.textContent)).toContain("Line");
    first.unmount();
    patchSnap({ lights: [fanLight("grid")], devices: [fanDevice("matrix")] });
    show(blank(8, "strip"));
    await settle();
    expect(screen.getByRole("radio", { name: "Grid" })).toBeTruthy();
  });
});

describe("the Gradient tool", () => {
  it("seeds three stops round the ring, wrapped, and writes their OKLab blend into the frame", async () => {
    show(blank(12));
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Gradient" }));
    await frames();
    const colours = draft().frames[0]!.colors;
    expect(colours).toEqual(gradientColours(seedStops(0, true), 12, true));
    expect(new Set(colours).size).toBeGreaterThan(6);
    expect(screen.getAllByRole("button", { name: /^Stop \d, #/ })).toHaveLength(3);
    expect(screen.getByRole("button", { name: "Wrap" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("changes the blend when Wrap is turned off, and one undo puts the frame back", async () => {
    show(blank(12));
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Gradient" }));
    await frames();
    const wrapped = draft().frames[0]!.colors;
    fireEvent.click(screen.getByRole("button", { name: "Wrap" }));
    await frames();
    expect(draft().frames[0]!.colors).not.toEqual(wrapped);
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(draft().frames[0]!.colors).toEqual(wrapped);
  });

  it("moves a stop with the arrow keys and writes the new blend", async () => {
    show(blank(12));
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Gradient" }));
    await frames();
    const before = draft().frames[0]!.colors;
    const second = screen.getByRole("button", { name: /^Stop 2, #/ });
    fireEvent.keyDown(second, { key: "ArrowRight" });
    await frames();
    expect(draft().frames[0]!.colors).not.toEqual(before);
  });

  it("is 4 clicks from an open editor to a saved, wrapped, spinning ring (Ring, Gradient, Spin, Save)", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    api.lights.patch.mockResolvedValue({ ...fanLight("ring") });
    api.lights.list.mockResolvedValue({ lights: [fanLight("ring")] });
    const onSave = vi.fn();
    show(blank(8, "strip"), onSave);
    await settle();
    let clicks = 0;
    const click = async (name: string, role: "radio" | "button" = "button") => {
      clicks += 1;
      await act(async () => {
        fireEvent.click(screen.getByRole(role, { name }));
      });
    };
    await click("Ring", "radio");
    await click("Gradient");
    await click("Spin", "radio");
    await click("Save");
    expect(clicks).toBe(4);
    expect(onSave).toHaveBeenCalledTimes(1);
    const saved = onSave.mock.calls[0]![0] as Pattern;
    expect(saved.built_on.arrangement).toBe("ring");
    expect(saved.frames[0]!.colors).toEqual(gradientColours(seedStops(0, true), 8, true));
    expect(saved.motion).toEqual({ type: "spin", turns_per_s: 0.25 });
  });
});

describe("the Spin control", () => {
  it("turns Spin on at the default speed and puts it on the lights and in the saved document", async () => {
    const onSave = vi.fn();
    show(blank(12), onSave);
    await settle();
    fireEvent.click(screen.getByRole("radio", { name: "Spin" }));
    await frames();
    expect(draft().motion).toEqual({ type: "spin", turns_per_s: 0.25 });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await settle();
    expect((onSave.mock.calls[0]![0] as Pattern).motion).toEqual({ type: "spin", turns_per_s: 0.25 });
  });

  it("turns Spin off to none, and marks the document unsaved", async () => {
    show(blank(12, "ring", { type: "spin", turns_per_s: -0.5 } as Pattern["motion"]));
    await settle();
    expect(screen.getByRole("radio", { name: "Spin" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText("-0.50")).toBeTruthy();
    fireEvent.click(screen.getByRole("radio", { name: "Off" }));
    await frames();
    expect(draft().motion).toEqual({ type: "none" });
    expect(screen.getByText("Unsaved changes")).toBeTruthy();
  });

  it("keeps a motion it does not know until the owner changes it", async () => {
    const onSave = vi.fn();
    const rotate = { type: "rotate", speed: 2 } as unknown as Pattern["motion"];
    show(blank(12, "ring", rotate), onSave);
    await settle();
    expect(screen.getByRole("radio", { name: "Off" }).getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await settle();
    expect((onSave.mock.calls[0]![0] as Pattern).motion).toEqual(rotate);
  });
});

describe("the preview", () => {
  const four = (): Pattern => ({
    ...blank(4),
    frames: [{ colors: ["#ff0000", "#00ff00", "#0000ff", "#ffffff"], hold: 0 }],
    motion: { type: "spin", turns_per_s: 0.25 } as Pattern["motion"],
  });

  it("turns the picture with the daemon's maths: a ring of 4 at 0.25 laps a second moves one LED a second", async () => {
    show(four());
    await settle();
    await frames(1000); // the clock starts here
    fills = [];
    await frames(2000); // one second on
    const colours = four().frames[0]!.colors;
    const want = spinColours(colours, 0.25, 1);
    const drawn = fills.at(-1)!;
    expect(drawn).toEqual(want);
    expect(want).toEqual([colours[3], colours[0], colours[1], colours[2]]);
  });

  it("stands still under reduced motion", async () => {
    reduced = true;
    show(four());
    await settle();
    expect(pending.size).toBe(0);
    expect(fills.at(-1)).toEqual(four().frames[0]!.colors);
  });
});
