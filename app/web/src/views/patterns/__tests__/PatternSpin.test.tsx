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
import { getSnap, patchSnap } from "@/lib/store";
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
  // jsdom has no scrollIntoView; the header's light chooser asks its active option to scroll in.
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

  it("lets the owner choose which light of the same size the shape writes to (#481)", async () => {
    const first = fanLight("strip");
    const second = { ...fanLight("strip"), light: { ...first.light, id: "fake:b#z0" }, label: "Fan 2" };
    patchSnap({ lights: [first, second], devices: [fanDevice()] });
    api.lights.patch.mockResolvedValue({ ...first, light: { ...first.light, arrangement: "ring" } });
    api.lights.list.mockResolvedValue({
      lights: [
        { ...first, light: { ...first.light, arrangement: "ring" } },
        { ...second, light: { ...second.light, arrangement: "ring" } },
      ],
    });
    show(blank(8, "strip"));
    await settle();
    const chooser = screen.getByRole("combobox", { name: "Which light the shape change writes to" });
    fireEvent.click(chooser);
    await act(async () => {
      fireEvent.click(screen.getByRole("option", { name: "Fan 2" }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    expect(api.lights.patch).toHaveBeenCalledWith("fake:b#z0", { arrangement: "ring" });
    expect(api.lights.patch).not.toHaveBeenCalledWith("fake:a#z0", { arrangement: "ring" });
  });

  it("does not offer a light of another arrangement, since the chooser is not a second shape change (#481)", async () => {
    const line = fanLight("strip");
    const ring = { ...fanLight("ring"), light: { ...line.light, id: "fake:b#z0", arrangement: "ring" as const }, label: "Fan 2" };
    patchSnap({ lights: [line, ring], devices: [fanDevice()] });
    show(blank(8, "strip"));
    await settle();
    expect(screen.queryByRole("combobox", { name: "Which light the shape change writes to" })).toBeNull();
    // The light of the pattern's shape is the one a shape change writes to.
    api.lights.patch.mockResolvedValue({ ...line, light: { ...line.light, arrangement: "ring" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { arrangement: "ring" });
    expect(api.lights.patch).not.toHaveBeenCalledWith("fake:b#z0", { arrangement: "ring" });
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

describe("undoing a shape change", () => {
  /** The arrangement the store holds for the light these tests write to. */
  const stored = (): string | undefined => getSnap().lights?.find((l) => l.light.id === "fake:a#z0")?.light.arrangement;
  /** Clicks a key and lets the light write, the refresh and the draft's frame settle. */
  const click = async (role: "radio" | "button", name: string) => {
    await act(async () => {
      fireEvent.click(screen.getByRole(role, { name }));
    });
    await frames();
  };
  /** The light the daemon answers with on the next PATCH, and the list it then reports. */
  const lightAs = (arrangement: "strip" | "ring") => {
    api.lights.patch.mockResolvedValue({ ...fanLight(arrangement) });
    api.lights.list.mockResolvedValue({ lights: [fanLight(arrangement)] });
  };

  it("writes the light back with the pattern, and writes it again on redo (#481)", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    lightAs("ring");
    show(blank(8, "strip"));
    await settle();

    await click("radio", "Ring");
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { arrangement: "ring" });
    expect(stored()).toBe("ring");

    // Undo: the light goes back to the line it was, and the pattern's shape with it.
    api.lights.patch.mockClear();
    lightAs("strip");
    await click("button", "Undo");
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { arrangement: "strip" });
    expect(stored()).toBe("strip");
    expect(screen.getByText("strip of 8")).toBeTruthy();

    // Redo: the ring is written to the same light again.
    api.lights.patch.mockClear();
    lightAs("ring");
    await click("button", "Redo");
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { arrangement: "ring" });
    expect(stored()).toBe("ring");
    expect(screen.getByText("ring of 8")).toBeTruthy();
  });

  it("leaves the shape and the history where they were when the light will not change (#481)", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    lightAs("ring");
    show(blank(8, "strip"));
    await settle();
    await click("radio", "Ring");
    expect(screen.getByText("ring of 8")).toBeTruthy();

    api.lights.patch.mockRejectedValue(new Error("the light file could not be saved"));
    await click("button", "Undo");
    expect(api.lights.patch).toHaveBeenCalledWith("fake:a#z0", { arrangement: "strip" });
    // The editor keeps the ring it shows, because the light was not written; the step is still
    // there to undo, and the row says why.
    expect(screen.getByText("ring of 8")).toBeTruthy();
    expect(screen.getByText("the light file could not be saved")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByRole("button", { name: "Redo" }) as HTMLButtonElement).disabled).toBe(true);
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

  it("makes every arrow nudge undoable, not just the first (#481)", async () => {
    show(blank(12));
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Gradient" }));
    await frames();
    const before = draft().frames[0]!.colors;
    const stop = screen.getByRole("button", { name: /^Stop 1, #/ });
    fireEvent.keyDown(stop, { key: "ArrowRight" });
    await frames();
    const afterOne = draft().frames[0]!.colors;
    fireEvent.keyDown(stop, { key: "ArrowRight" });
    await frames();
    const afterTwo = draft().frames[0]!.colors;
    expect(afterTwo).not.toEqual(afterOne);

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(draft().frames[0]!.colors).toEqual(afterOne);
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(draft().frames[0]!.colors).toEqual(before);
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

  it("shows Off, not Spin, at 0 turns a second: the daemon plays no motion (#481)", async () => {
    show(blank(12, "ring", { type: "spin", turns_per_s: 0 } as Pattern["motion"]));
    await settle();
    expect(screen.getByRole("radio", { name: "Off" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("radio", { name: "Spin" }).getAttribute("aria-checked")).toBe("false");
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

  it("the Off after a Spin is its own history entry: Spin, Off, Undo -> Spin (#491)", async () => {
    show(blank(12, "ring"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Spin" }));
    });
    await frames();
    expect(draft().motion).toEqual({ type: "spin", turns_per_s: 0.25 });
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false);

    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Off" }));
    });
    await frames();
    expect(draft().motion).toEqual({ type: "none" });

    // One press puts the spin back; the next press takes it away again.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual({ type: "spin", turns_per_s: 0.25 });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual({ type: "none" });
  });

  it("closing the editor during a pending undo write sends no draft after the DELETE (#491)", async () => {
    patchSnap({ lights: [fanLight("strip")], devices: [fanDevice()] });
    api.lights.patch.mockResolvedValue({ ...fanLight("ring") });
    api.lights.list.mockResolvedValue({ lights: [fanLight("ring")] });
    const view = show(blank(8, "strip"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Ring" }));
    });
    await frames();
    expect(screen.getByText("ring of 8")).toBeTruthy();

    // Hold the undo's light PATCH. The owner then closes the editor, which deletes the draft.
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
    view.unmount();
    await settle();
    expect(api.deleteDraft).toHaveBeenCalled();
    api.putPatternDraft.mockClear();

    // The write lands after the editor is gone: nothing may go back on the lights.
    await act(async () => {
      land();
      for (let i = 0; i < 20; i++) await Promise.resolve();
    });
    await frames();
    expect(api.putPatternDraft).not.toHaveBeenCalled();
  });

  it("a held arrow key is one history entry, not one per repeat (#491)", async () => {
    const started = { type: "spin", turns_per_s: 0.25 } as Pattern["motion"];
    show(blank(12, "ring", started));
    await settle();
    const slider = screen.getByRole("slider", { name: "Spin speed, turns per second" });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fill all" }));
    });
    await frames();
    const filled = draft().frames[0]!.colors;

    await act(async () => {
      fireEvent.keyDown(slider, { key: "ArrowUp" });
    });
    for (let k = 0; k < 95; k++) {
      await act(async () => {
        fireEvent.keyDown(slider, { key: "ArrowUp", repeat: true });
      });
    }
    await act(async () => {
      fireEvent.keyUp(slider, { key: "ArrowUp" });
    });
    await frames();

    // One press undoes the whole held key, and the fill is still undoable behind it.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual(started);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().frames[0]!.colors).not.toEqual(filled);
  });

  it("undoes one drag of the spin slider in one press, not one tick at a time (#491)", async () => {
    const started = { type: "spin", turns_per_s: 0.25 } as Pattern["motion"];
    show(blank(12, "ring", started));
    await settle();
    const slider = screen.getByRole("slider", { name: "Spin speed, turns per second" });

    // A press on the cap and one drag across many 0.05 steps: the slider sends one onChange per
    // step crossed, but the editor must open only one history entry for the whole gesture.
    fireEvent.pointerDown(slider, { pointerId: 1, clientX: 210 });
    for (let x = 220; x <= 320; x += 10) {
      fireEvent.pointerMove(slider, { pointerId: 1, clientX: x });
    }
    fireEvent.pointerUp(slider, { pointerId: 1, clientX: 320 });
    await frames();
    const dragged = draft().motion as { type: string; turns_per_s: number };
    expect(dragged.type).toBe("spin");
    expect(dragged.turns_per_s).toBeGreaterThan(0.25);

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    // One press puts the whole drag back, and no entry of the drag is left to undo.
    expect(draft().motion).toEqual(started);
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(true);
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
