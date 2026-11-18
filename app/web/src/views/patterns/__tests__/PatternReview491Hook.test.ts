// Behaviours the reviews of PR #491 proved (review pass 3); kept so no later fix pass regresses them.
// Probes A, B, G, H of the second review, and the attacks on the
// serial undo lane (press during a pending write, many presses, undo+redo during a write, a
// failed write mid-queue, unmount mid-write, an edit between the two top-entry checks).

import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { usePatternEditor } from "@/views/patterns/usePatternEditor";
import type { Arrangement } from "@/proto/Arrangement";
import type { Pattern } from "@/proto/Pattern";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
const RED = "#ff2d55";

let frames: FrameRequestCallback[] = [];
let nextFrame = 0;

beforeEach(() => {
  frames = [];
  nextFrame = 0;
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    nextFrame += 1;
    frames.push(cb);
    return nextFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => {
    frames = frames.filter((_, i) => i !== id - 1);
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function ring(on: Record<number, string> = {}): Pattern {
  const colors = Array.from({ length: 12 }, (_, i) => on[i] ?? OFF);
  return {
    id: "pattern:Intake",
    built_on: { led_count: 12, arrangement: "ring" },
    frames: [{ colors, hold: 0 }],
    motion: { type: "none" },
  };
}

const colours = (pattern: Pattern): string[] => pattern.frames[0]?.colors ?? [];

/** Let every queued microtask and every queued frame run. */
async function drain(): Promise<void> {
  for (let i = 0; i < 60; i++) await Promise.resolve();
  const queued = frames.splice(0, frames.length);
  for (const cb of queued) cb(0);
  for (let i = 0; i < 60; i++) await Promise.resolve();
}

const spin = (t: number): Pattern["motion"] => ({ type: "spin", turns_per_s: t }) as Pattern["motion"];

describe("#491 probes A, B, G, H rebuilt against the head", () => {
  it("A: an edit during the undo's light write leaves the light with the pattern's shape", async () => {
    let land: () => void = () => {};
    const write = vi.fn((_id: string, arrangement: Arrangement) =>
      arrangement === "ring"
        ? new Promise<void>((resolve) => {
            land = resolve;
          })
        : Promise.resolve(),
    );
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    act(() => {
      result.current.undo();
    });
    act(() => {
      result.current.stroke([1]);
    });
    await act(async () => {
      land();
      await drain();
    });
    expect(write).toHaveBeenCalledWith("L", "ring");
    expect(write).toHaveBeenLastCalledWith("L", "strip");
    expect(result.current.builtOn.arrangement).toBe("strip");
  });

  it("B: two undo presses while the light write is in flight undo two steps", async () => {
    const landers: Array<() => void> = [];
    const write = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          landers.push(resolve);
        }),
    );
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    const before = [...colours(result.current.pattern)];
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    act(() => result.current.stroke([1]));
    const painted = [...colours(result.current.pattern)];
    act(() => result.current.setShape("ring", undefined, { light: "L", from: "strip", to: "ring" }));

    act(() => {
      result.current.undo();
      result.current.undo();
    });
    await act(async () => {
      landers.forEach((l) => l());
      await drain();
    });
    expect(result.current.builtOn.arrangement).toBe("strip");
    expect(colours(result.current.pattern)).toEqual(before);
    expect(colours(result.current.pattern)).not.toEqual(painted);
  });

  it("G: a slider drag (bracketed by begin/end) is one history entry and does not evict the paint", async () => {
    const { result } = renderHook(() => usePatternEditor(ring({ 0: RED }), { onDraft: () => {} }));
    act(() => {
      result.current.beginStroke();
      result.current.stroke([1]);
      result.current.endStroke();
    });
    const painted = [...colours(result.current.pattern)];

    act(() => result.current.beginMotionDrag());
    for (let k = 1; k <= 120; k++) act(() => result.current.setMotion(spin(k * 0.05)));
    act(() => result.current.endMotionDrag());

    for (let k = 0; k < 100; k++) act(() => result.current.undo());
    // The paint is undone by the history: the drag did not push it out.
    expect(colours(result.current.pattern)).not.toEqual(painted);
    expect(result.current.canUndo).toBe(false);
  });

  it("H: one press undoes the whole slider drag", async () => {
    const { result } = renderHook(() => usePatternEditor(ring({ 0: RED }), { onDraft: () => {} }));
    act(() => result.current.beginMotionDrag());
    for (let k = 1; k <= 15; k++) act(() => result.current.setMotion(spin(Number((0.25 + k * 0.05).toFixed(2)))));
    act(() => result.current.endMotionDrag());
    expect(result.current.motion).toEqual(spin(1));
    act(() => result.current.undo());
    expect(result.current.motion).toEqual({ type: "none" });
    expect(result.current.canUndo).toBe(false);
  });
});

describe("#491 the serial undo lane", () => {
  it("P1: a second press during a pending write undoes the next step once the write lands", async () => {
    const landers: Array<() => void> = [];
    const write = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          landers.push(resolve);
        }),
    );
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    const before = [...colours(result.current.pattern)];
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    act(() => result.current.stroke([1]));
    const painted = [...colours(result.current.pattern)];
    act(() => result.current.setShape("ring", undefined, { light: "L", from: "strip", to: "ring" }));

    act(() => {
      result.current.undo();
      result.current.undo();
    });
    // Only the first write has gone out; the second press waits for it.
    expect(write).toHaveBeenCalledTimes(1);
    await act(async () => {
      landers.forEach((l) => l());
      await drain();
    });
    // The second step (the paint) needs no light write, so only one write went out.
    expect(write).toHaveBeenCalledTimes(1);
    expect(result.current.builtOn.arrangement).toBe("strip");
    expect(colours(result.current.pattern)).toEqual(before);
    expect(colours(result.current.pattern)).not.toEqual(painted);
    // The first shape change is still there: two presses undid two of the three steps.
    expect(result.current.canUndo).toBe(true);
  });

  it("P2: five presses undo five steps, one at a time", async () => {
    const write = vi.fn<(id: string, arrangement: Arrangement) => Promise<void>>(async () => {});
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    const before = [...colours(result.current.pattern)];
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    act(() => result.current.stroke([1]));
    act(() => result.current.setShape("ring", undefined, { light: "L", from: "strip", to: "ring" }));
    act(() => result.current.stroke([2]));
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));

    act(() => {
      result.current.undo();
      result.current.undo();
      result.current.undo();
      result.current.undo();
      result.current.undo();
    });
    await act(async () => {
      await drain();
    });
    expect(result.current.canUndo).toBe(false);
    expect(result.current.canRedo).toBe(true);
    expect(result.current.builtOn.arrangement).toBe("ring");
    expect(colours(result.current.pattern)).toEqual(before);
    // One light write per shape step, in order.
    expect(write.mock.calls.map((c) => c[1])).toEqual(["ring", "strip", "ring"]);
  });

  it("P3: a redo pressed during a pending undo write runs after it and leaves light and pattern in step", async () => {
    const landers: Array<() => void> = [];
    const write = vi.fn<(id: string, arrangement: Arrangement) => Promise<void>>(
      () =>
        new Promise<void>((resolve) => {
          landers.push(resolve);
        }),
    );
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    act(() => {
      result.current.undo();
      result.current.redo();
    });
    // The redo waits for the undo's write: it is not run on top of it.
    expect(write).toHaveBeenCalledTimes(1);
    await act(async () => {
      landers.splice(0).forEach((l) => l());
      await drain();
    });
    await act(async () => {
      landers.splice(0).forEach((l) => l());
      await drain();
    });
    // The undo wrote "ring", the redo wrote "strip" again: the light follows the pattern.
    expect(write.mock.calls.map((c) => c[1])).toEqual(["ring", "strip"]);
    expect(result.current.builtOn.arrangement).toBe("strip");
    expect(result.current.canUndo).toBe(true);
    expect(result.current.canRedo).toBe(false);
  });

  it("P4: a failed write leaves the step, and the lane is not stuck behind it", async () => {
    let fail = true;
    const write = vi.fn(async () => {
      if (fail) throw new Error("the light file could not be saved");
    });
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    await act(async () => {
      result.current.undo();
      await drain();
    });
    // The write failed: the shape and the step stay.
    expect(result.current.builtOn.arrangement).toBe("strip");
    expect(result.current.canUndo).toBe(true);

    // The next press is not stuck behind the failed one: it tries again.
    fail = false;
    await act(async () => {
      result.current.undo();
      await drain();
    });
    expect(write).toHaveBeenCalledTimes(2);
    expect(result.current.builtOn.arrangement).toBe("ring");
    expect(result.current.canUndo).toBe(false);
  });

  it("P4b: a failed write in the middle of a queue does not strand the later presses", async () => {
    const landers: Array<() => void> = [];
    let failNext = false;
    const write = vi.fn(() => {
      if (failNext) {
        failNext = false;
        return Promise.reject(new Error("nope"));
      }
      return new Promise<void>((resolve) => {
        landers.push(resolve);
      });
    });
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    const before = [...colours(result.current.pattern)];
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    act(() => result.current.stroke([1]));
    act(() => result.current.setShape("ring", undefined, { light: "L", from: "strip", to: "ring" }));

    failNext = true;
    act(() => {
      result.current.undo();
      result.current.undo();
      result.current.undo();
    });
    // Let the failed write settle, then release every write the lane still holds.
    for (let round = 0; round < 6; round++) {
      await act(async () => {
        landers.splice(0).forEach((l) => l());
        await drain();
      });
    }
    // The lane drained: the queued presses were not stranded behind the failed write, and the
    // history moved for the ones that worked.
    expect(colours(result.current.pattern)).toEqual(before);
    expect(result.current.canUndo).toBe(true);
  });

  it("P5: an unmount during the undo's write does not put a draft on the lights afterwards", async () => {
    let land: () => void = () => {};
    const draft = vi.fn();
    const write = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          land = resolve;
        }),
    );
    const { result, unmount } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: draft, onShapeLight: write }),
    );
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    act(() => {
      result.current.undo();
    });
    unmount();
    // The editor is gone: nothing may go on the lights from now on.
    expect(draft.mock.calls.length).toBe(0);
    await act(async () => {
      land();
      await drain();
    });
    expect(draft.mock.calls.length).toBe(0);
  });
});

describe("#491 the Devices-change rule and the forward write after a mid-write edit", () => {
  it("N3: a light changed in Devices is left alone and the reason is named", async () => {
    let current: Arrangement = "strip";
    const write = vi.fn(async () => {});
    const skipped = vi.fn();
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), {
        onDraft: () => {},
        onShapeLight: write,
        getShapeLight: () => current,
        onShapeSkipped: skipped,
      }),
    );
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    current = "ring";
    await act(async () => {
      result.current.undo();
      await drain();
    });
    expect(write).not.toHaveBeenCalled();
    expect(skipped).toHaveBeenCalledWith(expect.objectContaining({ light: "L" }), "strip", "ring");
    expect(result.current.builtOn.arrangement).toBe("strip");
    expect(result.current.canUndo).toBe(true);
  });

  /** A held write so an edit can land while it is in flight, plus the store it keeps. */
  function heldWriter() {
    const store = { arrangement: "strip" as Arrangement };
    let land: () => void = () => {};
    const write = vi.fn((_id: string, arrangement: Arrangement) => {
      if (arrangement === "ring")
        return new Promise<void>((resolve) => {
          land = () => {
            store.arrangement = arrangement;
            resolve();
          };
        });
      store.arrangement = arrangement;
      return Promise.resolve();
    });
    return { store, write, land: () => land() };
  }

  it("P6a: with a fresh read of the light, the mid-write edit leaves the light with the pattern", async () => {
    const { store, write, land } = heldWriter();
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), {
        onDraft: () => {},
        onShapeLight: write,
        // A fresh read: the editor sees the light as the daemon has it now.
        getShapeLight: () => store.arrangement,
      }),
    );
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    store.arrangement = "strip";
    act(() => {
      result.current.undo();
    });
    act(() => {
      result.current.stroke([1]);
    });
    await act(async () => {
      land();
      await drain();
    });
    // The undo wrote "ring"; the edit landed; the light goes back to the shape the pattern kept.
    expect(write.mock.calls.map((c) => c[1])).toEqual(["ring", "strip"]);
    expect(store.arrangement).toBe("strip");
    expect(result.current.builtOn.arrangement).toBe("strip");
  });

  it("P6b: with a stale read of the light (what the editor holds between renders), the light is left out of step", async () => {
    const { store, write, land } = heldWriter();
    // What the editor's last render saw. The app updates the store on every write (refreshLights)
    // and the rendered copy a render later.
    const rendered = { arrangement: "strip" as Arrangement };
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), {
        onDraft: () => {},
        onShapeLight: write,
        getShapeLight: () => rendered.arrangement,
      }),
    );
    act(() => result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }));
    rendered.arrangement = "strip";
    act(() => {
      result.current.undo();
    });
    act(() => {
      result.current.stroke([1]);
    });
    await act(async () => {
      land();
      await drain();
    });
    // The editor still reads the light as it was before the undo's write, so it believes the light
    // was changed elsewhere and refuses the forward write.
    expect(rendered.arrangement).toBe("strip");
    expect(store.arrangement).toBe("ring");
    expect(result.current.builtOn.arrangement).toBe("strip");
  });
});
