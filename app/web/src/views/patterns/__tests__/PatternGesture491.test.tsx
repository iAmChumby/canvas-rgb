// Behaviours the reviews of PR #491 proved (review pass 4); kept so no later fix pass regresses them.
// The motion gesture gate: every motion control must make exactly one history entry per gesture
// and the gate must never stay open. Plus the StrictMode double-invoke of the new mount guard.

const { api } = vi.hoisted(() => ({
  api: {
    putPatternDraft: vi.fn(),
    deleteDraft: vi.fn(),
    putPattern: vi.fn(),
    lights: { patch: vi.fn(), list: vi.fn() },
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { Pattern } from "@/proto/Pattern";
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
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
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
const spin = (t: number): Pattern["motion"] => ({ type: "spin", turns_per_s: t }) as Pattern["motion"];

const sliderOf = () => screen.getByRole("slider", { name: "Spin speed, turns per second" }) as HTMLDivElement;

/** A press on the cap and a drag across many 0.05 steps. */
const drag = (el: HTMLDivElement, from = 210, to = 320) => {
  fireEvent.pointerDown(el, { pointerId: 1, clientX: from });
  for (let x = from + 10; x <= to; x += 10) fireEvent.pointerMove(el, { pointerId: 1, clientX: x });
};

/** How many history entries the editor holds: one Undo press per entry, until the button is dead. */
async function undoCount(): Promise<number> {
  let n = 0;
  for (;;) {
    const btn = screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement;
    if (btn.disabled) break;
    await act(async () => {
      fireEvent.click(btn);
    });
    await frames();
    n += 1;
    if (n > 300) break;
  }
  return n;
}

describe("#491 the motion gesture gate", () => {
  it("G0: a plain held arrow key is one history entry", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      fireEvent.keyDown(slider, { key: "ArrowUp" });
    });
    for (let k = 0; k < 20; k++) {
      await act(async () => {
        fireEvent.keyDown(slider, { key: "ArrowUp", repeat: true });
      });
    }
    await act(async () => {
      fireEvent.keyUp(slider, { key: "ArrowUp" });
    });
    await frames();
    expect(await undoCount()).toBe(1);
  });

  it("G1: the focus lost in the middle of a held key leaves the gate open, and the Spin/Off after it is not in the history", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      fireEvent.keyDown(slider, { key: "ArrowUp" });
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.3));

    // The focus goes elsewhere while the key is still down: no keyup reaches the slider, so the
    // gate is never closed. The Slider has no blur handler at all (Slider.tsx:182-213).
    await act(async () => {
      fireEvent.blur(slider);
    });

    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Off" }));
    });
    await frames();
    expect(draft().motion).toEqual({ type: "none" });

    // The Off must be a change of its own: one Undo brings the spin back.
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.3));
  });

  it("G2: a key held while the pointer drags is one gesture, one entry: the pointer's end does not close the key's hold", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      fireEvent.keyDown(slider, { key: "ArrowUp" });
    });
    await act(async () => {
      drag(slider);
      fireEvent.pointerUp(slider, { pointerId: 1, clientX: 320 });
    });
    // The key is still down: its repeats are part of the same gesture, not new entries.
    for (let k = 0; k < 20; k++) {
      await act(async () => {
        fireEvent.keyDown(slider, { key: "ArrowUp", repeat: true });
      });
    }
    await act(async () => {
      fireEvent.keyUp(slider, { key: "ArrowUp" });
    });
    await frames();
    expect(await undoCount()).toBe(1);
  });

  it("G3: a key held while another key is tapped: the tap's keyup must not close the hold's gate", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      fireEvent.keyDown(slider, { key: "ArrowUp" });
    });
    // The owner taps Shift while the arrow is down: the keyup of Shift reaches the slider and
    // closes the gate (Slider.tsx:178-180), so the arrow's repeats find it closed.
    await act(async () => {
      fireEvent.keyUp(slider, { key: "Shift" });
    });
    for (let k = 0; k < 20; k++) {
      await act(async () => {
        fireEvent.keyDown(slider, { key: "ArrowUp", repeat: true });
      });
    }
    await act(async () => {
      fireEvent.keyUp(slider, { key: "ArrowUp" });
    });
    await frames();
    expect(await undoCount()).toBe(1);
  });

  it("G4: Home and End are one entry each", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      fireEvent.keyDown(slider, { key: "Home" });
    });
    await act(async () => {
      fireEvent.keyUp(slider, { key: "Home" });
    });
    await frames();
    expect(draft().motion).toEqual(spin(-5));
    await act(async () => {
      fireEvent.keyDown(slider, { key: "End" });
    });
    await act(async () => {
      fireEvent.keyUp(slider, { key: "End" });
    });
    await frames();
    expect(draft().motion).toEqual(spin(5));
    expect(await undoCount()).toBe(2);
  });

  it("G5: a held PageUp is one history entry", async () => {
    show(blank(12, "ring", spin(0.25)));
    await settle();
    const slider = sliderOf();
    await act(async () => {
      fireEvent.keyDown(slider, { key: "PageUp" });
    });
    for (let k = 0; k < 20; k++) {
      await act(async () => {
        fireEvent.keyDown(slider, { key: "PageUp", repeat: true });
      });
    }
    await act(async () => {
      fireEvent.keyUp(slider, { key: "PageUp" });
    });
    await frames();
    expect(await undoCount()).toBe(1);
  });

  it("G6: the Spin/Off segmented control is one entry of its own", async () => {
    show(blank(12, "ring"));
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Spin" }));
    });
    await frames();
    expect(draft().motion).toEqual(spin(0.25));
    await act(async () => {
      fireEvent.click(screen.getByRole("radio", { name: "Off" }));
    });
    await frames();
    expect(draft().motion).toEqual({ type: "none" });
    expect(await undoCount()).toBe(2);
  });
});

describe("#491 the new mount guard under StrictMode", () => {
  it("S1: under StrictMode the editor still puts a painted pattern on the lights", async () => {
    render(
      <StrictMode>
        <PatternEditor pattern={blank(12, "ring")} onSave={vi.fn()} onCancel={vi.fn()} />
      </StrictMode>,
    );
    await settle();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Fill all" }));
    });
    await frames();
    // The last draft must carry the fill: the paint reached the lights.
    const sent = api.putPatternDraft.mock.calls.map((c) => (c[0] as { pattern: Pattern }).pattern);
    expect(sent.length).toBeGreaterThan(0);
    expect(sent.at(-1)!.frames[0]!.colors.some((c) => c !== OFF)).toBe(true);
  });
});
