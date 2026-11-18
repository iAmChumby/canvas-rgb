import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { layoutDots, type Dot } from "@/views/patterns/dotLayout";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { Pattern } from "@/proto/Pattern";

// #481 items 7 and 8. The lights play whatever pattern the draft carries from the moment it lands,
// so what the editor sends decides what the owner sees on them: the selected frame, as a still,
// while painting; the whole timeline while Play is on. And the preview's clock is the lights' clock:
// it starts when the PUT that started the lights was answered, not when the dots first drew.

const { api } = vi.hoisted(() => ({
  api: { putPatternDraft: vi.fn(), deleteDraft: vi.fn(), putPattern: vi.fn() },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { PatternEditor } from "@/views/patterns/PatternEditor";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
const RED = "#ff0000";
const GREEN = "#00ff00";
const BLUE = "#0000ff";
const BOX = 400;

let pending = new Map<number, FrameRequestCallback>();
let nextFrame = 0;
/** What the fake 2D context was asked to fill, one entry per dot, per frame drawn. */
let fills: string[][] = [];
/** The page's clock: `performance.now()` as the test sets it. */
let clock = 0;
/** Answers of the draft PUTs the test is holding back, oldest first. */
let held: Array<() => void> = [];
let holding = false;

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
  window.matchMedia = ((query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
});

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "setInterval", "clearTimeout", "clearInterval", "Date"] });
  pending = new Map();
  nextFrame = 0;
  fills = [];
  clock = 1_000;
  held = [];
  holding = false;
  vi.spyOn(performance, "now").mockImplementation(() => clock);
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    nextFrame += 1;
    pending.set(nextFrame, cb);
    return nextFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => void pending.delete(id));
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
  Element.prototype.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: BOX, height: BOX, right: BOX, bottom: BOX, x: 0, y: 0, toJSON() {} }) as DOMRect;
  api.putPatternDraft.mockReset().mockImplementation(
    () =>
      holding
        ? new Promise<DraftStatus>((resolve) => held.push(() => resolve(status())))
        : Promise.resolve(status()),
  );
  api.deleteDraft.mockReset().mockResolvedValue(status({ active: false }));
  api.putPattern.mockReset();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

function status(over: Partial<DraftStatus> = {}): DraftStatus {
  return { active: true, props: {}, lease_remaining_ms: 15_000, heartbeat_ms: 5_000, lease_ms: 15_000, ...over };
}

/** A ring of 12 with `colours[f]` on LED 0 of frame f, every other LED off, each frame holding `hold` ms. */
function framed(colours: string[], hold = 300, fade = false): Pattern {
  return {
    id: "pattern:Intake",
    built_on: { led_count: 12, arrangement: "ring" },
    frames: colours.map((c) => ({
      colors: Array.from({ length: 12 }, (_, i) => (i === 0 ? c : OFF)),
      hold,
      ...(fade ? { transition: { kind: "fade" as const, ms: 200, easing: "linear" as const } } : {}),
    })),
    motion: { type: "none" },
  };
}

function show(pattern: Pattern, onSave: (p: Pattern) => void = vi.fn()) {
  const view = render(<PatternEditor pattern={pattern} onSave={onSave} onCancel={vi.fn()} />);
  const canvas = view.container.querySelector("canvas") as HTMLCanvasElement;
  const dots: Dot[] = layoutDots(pattern.built_on, BOX, BOX);
  return { canvas, dots };
}

const settle = () => act(async () => {});

/** Runs every queued animation frame at `now`: the editor's draft goes out on this clock, and so does the preview. */
async function frames(now = clock): Promise<void> {
  const queued = [...pending.values()];
  pending.clear();
  await act(async () => {
    for (const cb of queued) cb(now);
  });
}

const at = (dot: Dot) => ({ clientX: dot.x, clientY: dot.y, pointerId: 1 });
const stroke = async (canvas: HTMLCanvasElement, dot: Dot) => {
  fireEvent.pointerDown(canvas, at(dot));
  fireEvent.pointerUp(canvas, { pointerId: 1 });
  await frames();
};

const calls = () => api.putPatternDraft.mock.calls.map((call) => call[0] as { pattern: Pattern; seq: number });
/** The document the lights were last sent. */
const sent = (): Pattern => calls().at(-1)!.pattern;
const heads = (p: Pattern): string[] => p.frames.map((f) => f.colors[0]!);

const key = (n: number) => screen.getByRole("button", { name: new RegExp(`^Frame ${n} of `) });
const play = () => fireEvent.click(screen.getByRole("button", { name: "Play preview" }));
const stop = () => fireEvent.click(screen.getByRole("button", { name: "Stop preview" }));
/** The ink of LED 0 in the newest frame the dots drew. */
const ledZero = (): string | undefined => fills.at(-1)?.[0];

describe("what the lights are given while the owner paints (#481, item 7)", () => {
  it("is the selected frame alone, as a still, for every stroke, and the lights never get the other frames", async () => {
    const { canvas, dots } = show(framed([RED, GREEN, BLUE]));
    await settle();
    // Opening the editor puts frame 1 on the lights, not the three frames in time.
    expect(heads(sent())).toEqual([RED]);

    fireEvent.click(key(2));
    await frames();
    expect(heads(sent())).toEqual([GREEN]);

    await stroke(canvas, dots[3]!);
    await stroke(canvas, dots[4]!);
    await stroke(canvas, dots[5]!);
    // Every PUT since the editor opened carries exactly one frame: the daemon fits one frame per
    // stroke, and nothing it plays moves.
    for (const call of calls()) expect(call.pattern.frames).toHaveLength(1);
    expect(sent().frames[0]!.colors.slice(0, 6)).toEqual([GREEN, OFF, OFF, RED, RED, RED]);
  });

  it("carries no transition and no Loop, so the still has nothing to fade or loop to", async () => {
    show({ ...framed([RED, GREEN], 300, true), playback: { loop: false } });
    await settle();
    const first = sent();
    expect(first.frames).toHaveLength(1);
    expect(first.frames[0]!.transition).toBeUndefined();
    expect(first.playback).toBeUndefined();
    expect(first.motion).toEqual({ type: "none" });
  });

  it("keeps the pattern's shape and motion on the still, so a spin still turns", async () => {
    const spin = { ...framed([RED, GREEN]), motion: { type: "spin", turns_per_s: 0.5 } } as Pattern;
    show(spin);
    await settle();
    expect(sent().motion).toEqual({ type: "spin", turns_per_s: 0.5 });
    expect(sent().built_on).toEqual(spin.built_on);
  });

  it("saves the whole document, with every frame painted", async () => {
    const onSave = vi.fn();
    const { canvas, dots } = show(framed([RED, GREEN, BLUE]), onSave);
    await settle();
    fireEvent.click(key(3));
    await frames();
    await stroke(canvas, dots[7]!);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
    });
    const saved = onSave.mock.calls[0]![0] as Pattern;
    expect(saved.frames).toHaveLength(3);
    expect(heads(saved)).toEqual([RED, GREEN, BLUE]);
    expect(saved.frames[2]!.colors[7]).toBe(RED);
  });

  it("the heartbeat re-sends the still, never the frames in time", async () => {
    const { canvas, dots } = show(framed([RED, GREEN]));
    await settle();
    fireEvent.click(key(2));
    await frames();
    await stroke(canvas, dots[2]!);
    const before = calls().length;
    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    expect(calls().length).toBe(before + 1);
    expect(sent().frames).toHaveLength(1);
    expect(heads(sent())).toEqual([GREEN]);
  });
});

describe("what the lights are given while the owner plays (#481, item 7)", () => {
  it("Play puts the whole timeline on the lights, exactly as it will be saved, and Stop puts the frame back", async () => {
    const loaded = framed([RED, GREEN, BLUE], 300, true);
    show(loaded);
    await settle();
    expect(sent().frames).toHaveLength(1);

    play();
    await frames();
    expect(sent()).toEqual(loaded);

    stop();
    await frames();
    expect(sent().frames).toHaveLength(1);
    expect(heads(sent())).toEqual([RED]);
  });

  it("a stroke made while it plays sends the whole timeline with the stroke in it", async () => {
    const { canvas, dots } = show(framed([RED, GREEN]));
    await settle();
    play();
    await frames();
    await stroke(canvas, dots[4]!);
    expect(sent().frames).toHaveLength(2);
    expect(sent().frames[0]!.colors[4]).toBe(RED);
    expect(sent().frames[1]!.colors[4]).toBe(OFF);
  });

  it("the heartbeat re-sends the whole timeline while it plays", async () => {
    show(framed([RED, GREEN]));
    await settle();
    play();
    await frames();
    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    expect(sent().frames).toHaveLength(2);
  });

  it("a timeline that cannot play (one frame left) is not sent as playing", async () => {
    show(framed([RED, GREEN]));
    await settle();
    play();
    await frames();
    expect(sent().frames).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: "Delete frame" }));
    await frames();
    // One frame is a still anyway, and Play is off: the lights are on the frame that is left.
    expect(sent().frames).toHaveLength(1);
    expect(screen.getByRole("button", { name: /^Nothing to play/ })).toBeTruthy();
  });

  it("selecting another frame while it plays leaves the lights on the whole timeline", async () => {
    show(framed([RED, GREEN, BLUE]));
    await settle();
    play();
    await frames();
    fireEvent.click(key(3));
    await frames();
    expect(sent().frames).toHaveLength(3);
  });
});

describe("the preview runs on the lights' clock (#481, item 8)", () => {
  it("starts when the PUT that started the lights was answered, and shows frame 1 until then", async () => {
    // Frame 1 red for 1000 ms, then frame 2 blue for 1000 ms, a cut between, looping.
    show(framed([RED, BLUE], 1_000));
    await settle();
    await frames();

    // Play at t = 1000. The daemon is slow: the PUT's answer comes at t = 3000, so the lights'
    // clock started in the middle of the round trip, t = 2000.
    holding = true;
    clock = 1_000;
    play();
    await frames(1_000);
    expect(calls().at(-1)!.pattern.frames).toHaveLength(2);

    // Before the answer the preview waits on its first moment, however late the next frame is.
    clock = 2_900;
    await frames(2_900);
    expect(ledZero()).toBe(RED);

    clock = 3_000;
    await act(async () => {
      held.splice(0).forEach((release) => release());
    });
    // The lights' clock is 2000: at t = 3500 they are 1500 ms in, frame 2 (blue). A preview that
    // had started its own clock at its first frame (t = 1000) would be 2500 ms in and show frame 1.
    clock = 3_500;
    await frames(3_500);
    expect(ledZero()).toBe(BLUE);
    clock = 4_200;
    await frames(4_200);
    // 2200 ms in, a cycle of 2000 ms: 200 ms into frame 1 again (red).
    expect(ledZero()).toBe(RED);
  });

  it("is not restarted by the heartbeat, which the daemon answers without starting its clock", async () => {
    show(framed([RED, BLUE], 1_000));
    await settle();
    await frames();
    clock = 2_000;
    play();
    await frames(2_000);
    // The lights' clock is 2000. A beat answered at t = 6500 must not move it.
    clock = 6_500;
    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    clock = 7_200;
    await frames(7_200);
    // 5200 ms in, 1200 ms into the 2000 ms cycle: frame 2 (blue). A preview restarted at the beat
    // (t = 6500) would be 700 ms in and show frame 1.
    expect(ledZero()).toBe(BLUE);
  });

  it("restarts with the lights when a change to the timeline reaches them", async () => {
    const { canvas, dots } = show(framed([RED, BLUE], 1_000));
    await settle();
    await frames();
    clock = 2_000;
    play();
    await frames(2_000);
    // A stroke at t = 5000 is a new document: the daemon starts its clock again there.
    clock = 5_000;
    await stroke(canvas, dots[5]!);
    clock = 5_400;
    await frames(5_400);
    expect(ledZero()).toBe(RED);
    clock = 6_200;
    await frames(6_200);
    expect(ledZero()).toBe(BLUE);
  });

  it("an answer to a PUT sent before Play was pressed does not set the clock", async () => {
    const { canvas, dots } = show(framed([RED, BLUE], 1_000));
    await settle();
    await frames();
    // A stroke whose answer is slow, then Play at once: the stroke's answer arrives after Play.
    holding = true;
    clock = 1_000;
    await stroke(canvas, dots[5]!);
    play();
    await frames(1_000);
    expect(held.length).toBe(2);
    // The stroke's answer (the older PUT) comes first, at t = 9000; it is not the whole timeline's start.
    clock = 9_000;
    await act(async () => {
      held[0]!();
    });
    clock = 9_100;
    await frames(9_100);
    // The preview still waits on the PUT that Play sent.
    expect(ledZero()).toBe(RED);
    // That one is answered at t = 9500 (sent at 1000): its clock is 5250. At 10 400 the lights are
    // 5150 ms in, 1150 ms into a 2000 ms cycle: frame 2 (blue).
    clock = 9_500;
    await act(async () => {
      held[1]!();
    });
    clock = 10_400;
    await frames(10_400);
    expect(ledZero()).toBe(BLUE);
  });
});
