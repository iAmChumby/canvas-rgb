import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { layoutDots, type Dot } from "@/views/patterns/dotLayout";
import type { Pattern } from "@/proto/Pattern";

// The pattern editor: the dot surface painted with the pointer, the draft it keeps on the lights,
// the document it saves, and what it does when the page closes. The daemon is mocked, and the
// clock is a stubbed rAF plus fake timers: nothing here waits for a real frame or a real second.

const { api } = vi.hoisted(() => ({
  api: {
    putPatternDraft: vi.fn(),
    deleteDraft: vi.fn(),
    putPattern: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { PatternEditor } from "@/views/patterns/PatternEditor";
import type { DraftStatus } from "@/proto/DraftStatus";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
/** A ring of 12 drawn in a 400 x 400 box: the box the canvas is measured with in these tests. */
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
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    nextFrame += 1;
    pending.set(nextFrame, cb);
    return nextFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => void pending.delete(id));
  api.putPatternDraft.mockReset().mockResolvedValue(status());
  api.deleteDraft.mockReset().mockResolvedValue(status({ active: false }));
  api.putPattern.mockReset();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

function status(over: Partial<DraftStatus> = {}): DraftStatus {
  return {
    active: true,
    props: {},
    lease_remaining_ms: 15_000,
    heartbeat_ms: 5_000,
    lease_ms: 15_000,
    ...over,
  };
}

/** A ring of 12, `on` naming the LEDs that are not off. */
function ring(on: Record<number, string> = {}): Pattern {
  const colors = Array.from({ length: 12 }, (_, i) => on[i] ?? OFF);
  return {
    id: "pattern:Intake",
    built_on: { led_count: 12, arrangement: "ring" },
    frames: [{ colors, hold: 0 }],
    motion: { type: "none" },
  };
}

/** The pattern's colours, in LED order. */
const coloursOf = (pattern: Pattern): string[] => pattern.frames[0]?.colors ?? [];
/** The LEDs that are not off. */
const lit = (colors: string[]): number[] =>
  colors.map((c, i) => (c === OFF ? -1 : i)).filter((i) => i >= 0);

/** Renders the editor with a canvas measured at 400 x 400 and the dots that box gives. */
function show(
  pattern: Pattern = ring(),
  props: { note?: string | null; onSave?: (p: Pattern) => void; onCancel?: () => void } = {},
) {
  const view = render(
    <PatternEditor
      pattern={pattern}
      note={props.note}
      onSave={props.onSave ?? vi.fn()}
      onCancel={props.onCancel ?? vi.fn()}
    />,
  );
  const canvas = view.container.querySelector("canvas") as HTMLCanvasElement;
  const rect = { left: 0, top: 0, width: BOX, height: BOX, right: BOX, bottom: BOX, x: 0, y: 0, toJSON() {} } as DOMRect;
  canvas.getBoundingClientRect = () => rect;
  const dots: Dot[] = layoutDots(pattern.built_on, BOX, BOX);
  return { view, canvas, dots };
}

/** Lets the promises of the requests already sent settle. */
const settle = () => act(async () => {});

/** Runs every queued animation frame: the editor's draft goes out on this clock. */
async function frames(): Promise<void> {
  const queued = [...pending.values()];
  pending.clear();
  await act(async () => {
    for (const cb of queued) cb(0);
  });
}

const at = (dot: Dot) => ({ clientX: dot.x, clientY: dot.y, pointerId: 1 });

describe("the pattern editor", () => {
  it("one pointer stroke is one undo step", async () => {
    const { canvas, dots } = show();
    await settle();

    fireEvent.pointerDown(canvas, at(dots[0]!));
    fireEvent.pointerMove(canvas, at(dots[1]!));
    fireEvent.pointerMove(canvas, at(dots[2]!));
    fireEvent.pointerUp(canvas, { pointerId: 1 });
    await frames();

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await frames();
    expect(lit(coloursOf(api.putPatternDraft.mock.calls.at(-1)![0].pattern))).toEqual([]);

    fireEvent.click(screen.getByRole("button", { name: "Redo" }));
    await frames();
    expect(lit(coloursOf(api.putPatternDraft.mock.calls.at(-1)![0].pattern))).toEqual([0, 1, 2]);
  });

  it("at most one draft PUT per animation frame while painting", async () => {
    const { canvas, dots } = show();
    await settle();
    api.putPatternDraft.mockClear();

    fireEvent.pointerDown(canvas, at(dots[0]!));
    fireEvent.pointerMove(canvas, at(dots[1]!));
    fireEvent.pointerMove(canvas, at(dots[2]!));
    // Two moves, one frame: nothing has gone out yet, and one PUT carries both.
    expect(api.putPatternDraft).not.toHaveBeenCalled();
    await frames();
    expect(api.putPatternDraft).toHaveBeenCalledTimes(1);
    const first = api.putPatternDraft.mock.calls[0]![0];
    expect(lit(coloursOf(first.pattern))).toEqual([0, 1, 2]);
    // The page's own epoch, 16 hex characters, and a rising sequence.
    expect(first.epoch).toMatch(/^[0-9a-f]{16}$/);
    expect(first.seq).toBe(2);

    fireEvent.pointerMove(canvas, at(dots[3]!));
    await frames();
    expect(api.putPatternDraft).toHaveBeenCalledTimes(2);
    const second = api.putPatternDraft.mock.calls[1]![0];
    expect(second.seq).toBeGreaterThan(first.seq);
    expect(lit(coloursOf(second.pattern))).toEqual([0, 1, 2, 3]);

    // A pointer-up with no move is not a change: it sends nothing.
    fireEvent.pointerUp(canvas, { pointerId: 1 });
    await frames();
    expect(api.putPatternDraft).toHaveBeenCalledTimes(2);
  });

  it("saving_keeps_fields_the_editor_does_not_know", async () => {
    const onSave = vi.fn();
    const loaded = {
      id: "pattern:Intake",
      built_on: { led_count: 12, arrangement: "ring" },
      frames: [{ colors: Array.from({ length: 12 }, () => OFF), hold: 5, label: "first" }],
      motion: { type: "rotate", speed: 2 },
      tags: ["a"],
    } as unknown as Pattern;
    const { canvas, dots } = show(loaded, { onSave });
    await settle();

    fireEvent.pointerDown(canvas, at(dots[4]!));
    fireEvent.pointerUp(canvas, { pointerId: 1 });
    await frames();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(onSave).toHaveBeenCalledTimes(1);
    const saved = onSave.mock.calls[0]![0] as unknown as Record<string, unknown>;
    expect(saved.id).toBe("pattern:Intake");
    expect(saved.built_on).toEqual({ led_count: 12, arrangement: "ring" });
    expect(saved.motion).toEqual({ type: "rotate", speed: 2 });
    expect(saved.tags).toEqual(["a"]);
    const frame = (saved.frames as { colors: string[]; hold: number; label?: string }[])[0]!;
    expect((saved.frames as unknown[]).length).toBe(1);
    expect(frame.hold).toBe(5);
    expect(frame.label).toBe("first");
    // The stroke is in the saved document, and the paint colour came from the pattern itself.
    expect(lit(frame.colors)).toEqual([4]);
  });

  it("a beat before the next render carries the newest colours", async () => {
    const { canvas, dots } = show();
    await settle();
    api.putPatternDraft.mockClear();

    // The stroke's frame and the heartbeat land in one batch, before React re-renders and runs an
    // effect: the beat must carry the stroke's document, never the colours of the render it began
    // in, which the daemon would take as the newer stroke.
    await act(async () => {
      fireEvent.pointerDown(canvas, at(dots[0]!));
      fireEvent.pointerUp(canvas, { pointerId: 1 });
      for (const cb of [...pending.values()]) cb(0);
      pending.clear();
      vi.advanceTimersByTime(5_000);
      await Promise.resolve();
    });

    const calls = api.putPatternDraft.mock.calls.map((call) => call[0] as { pattern: Pattern; seq: number });
    expect(calls.length).toBe(2);
    expect(lit(coloursOf(calls[0]!.pattern))).toEqual([0]);
    expect(lit(coloursOf(calls[1]!.pattern))).toEqual([0]);
    expect(calls[1]!.seq).toBeGreaterThan(calls[0]!.seq);
  });

  it("a save that fails keeps the strokes and the dirty mark and says why", async () => {
    const onSave = vi.fn().mockRejectedValue(new Error("the daemon could not write it"));
    const { canvas, dots } = show(ring(), { onSave });
    await settle();

    fireEvent.pointerDown(canvas, at(dots[0]!));
    fireEvent.pointerUp(canvas, { pointerId: 1 });
    await frames();
    expect(screen.getByText("Unsaved changes")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await settle();

    expect(onSave).toHaveBeenCalledTimes(1);
    // The sentence is the editor's own, in the row its other notes use, and the mark still says
    // the document is unsaved.
    expect(screen.getByText("the daemon could not write it")).toBeTruthy();
    expect(screen.getByText("Unsaved changes")).toBeTruthy();
    // The stroke is still the editor's: the next beat carries it.
    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    expect(lit(coloursOf(api.putPatternDraft.mock.calls.at(-1)![0].pattern))).toEqual([0]);
  });

  it("a save that lands clears the dirty mark", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const { canvas, dots } = show(ring(), { onSave });
    await settle();

    fireEvent.pointerDown(canvas, at(dots[0]!));
    fireEvent.pointerUp(canvas, { pointerId: 1 });
    await frames();
    expect(screen.getByText("Unsaved changes")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await settle();

    expect(onSave).toHaveBeenCalledTimes(1);
    expect(screen.getByText("No unsaved changes")).toBeTruthy();
  });

  it("closing the editor deletes the draft", async () => {
    const { view } = show();
    await settle();
    const epoch = api.putPatternDraft.mock.calls[0]![0].epoch as string;
    view.unmount();
    expect(api.deleteDraft).toHaveBeenCalledWith(epoch);
  });

  it("the no-device note is shown", async () => {
    api.putPatternDraft.mockResolvedValue(status({ note: "no device of this kind is connected" }));
    show();
    await settle();
    expect(screen.getByText("no device of this kind is connected")).toBeTruthy();
  });

  it("the draft beats while the editor is open", async () => {
    show();
    await settle();
    api.putPatternDraft.mockClear();
    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    expect(api.putPatternDraft).toHaveBeenCalledTimes(1);
    // A beat renews the lease with a new sequence: it is not the stroke it repeats.
    expect(api.putPatternDraft.mock.calls[0]![0].seq).toBe(2);
  });
});
