import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { usePatternEditor } from "@/views/patterns/usePatternEditor";
import type { Arrangement } from "@/proto/Arrangement";
import type { Pattern } from "@/proto/Pattern";

// Regression tests for the second review of #491. Each one watches the pattern the editor puts
// on the draft and the light it writes, not the history array: the user sees the light's shape and
// the shape the header names, and the daemon receives the document. The write is held open so an
// edit can land while it is in flight, which is the window the review found.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
const RED = "#ff2d55";

let frames: FrameRequestCallback[] = [];

beforeEach(() => {
  frames = [];
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    frames.push(cb);
    return frames.length;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => {
    frames = frames.filter((_, i) => i !== id - 1);
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

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

const colours = (pattern: Pattern): string[] => pattern.frames[0]?.colors ?? [];

function editor(pattern: Pattern = ring()) {
  return renderHook(() => usePatternEditor(pattern, { onDraft: () => {} }));
}

/** The serialised history queue runs on microtasks; let every queued step and frame finish. */
async function drain(): Promise<void> {
  for (let i = 0; i < 20; i++) await Promise.resolve();
  const queued = frames.splice(0, frames.length);
  for (const cb of queued) cb(0);
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

describe("the pattern editor's undo window (#491)", () => {
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
    act(() =>
      result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }),
    );
    expect(result.current.builtOn.arrangement).toBe("strip");

    // The undo writes the light back to a ring. Paint lands before that write resolves.
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

    // The light went back to a ring for the step, then forward to the strip the pattern kept.
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
    act(() =>
      result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }),
    );
    act(() => result.current.stroke([1]));
    const painted = [...colours(result.current.pattern)];
    act(() =>
      result.current.setShape("ring", undefined, { light: "L", from: "strip", to: "ring" }),
    );

    // Two presses, back to back, each held at its light write.
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

  it("H (hook gate): a drag of many motion ticks is one undo entry", () => {
    const { result } = editor();
    act(() => result.current.beginMotionDrag());
    for (let k = 1; k <= 15; k++) {
      act(() =>
        result.current.setMotion({
          type: "spin",
          turns_per_s: Number((0.25 + k * 0.05).toFixed(2)),
        } as Pattern["motion"]),
      );
    }
    act(() => result.current.endMotionDrag());
    expect(result.current.motion).toEqual({ type: "spin", turns_per_s: 1 });

    act(() => result.current.undo());
    expect(result.current.motion).toEqual({ type: "none" });
    expect(result.current.canUndo).toBe(false);
  });

  it("N3: undo does not overwrite a change the owner made to the light in Devices", async () => {
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
    act(() =>
      result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }),
    );
    // Devices changed the light back to a ring after this editor made it a strip.
    current = "ring";
    await act(async () => {
      result.current.undo();
      await drain();
    });

    expect(write).not.toHaveBeenCalled();
    expect(skipped).toHaveBeenCalledWith(expect.objectContaining({ light: "L" }), "strip", "ring");
    // The step and the shape stay, so the note row can name the conflict.
    expect(result.current.builtOn.arrangement).toBe("strip");
    expect(result.current.canUndo).toBe(true);
  });

  it("N6: undoing a ring-to-strip change restores the gradient's wrap", () => {
    const { result } = editor();
    act(() => result.current.setTool("gradient"));
    expect(result.current.gradient!.wrap).toBe(true);
    act(() => result.current.setShape("strip"));
    expect(result.current.gradient!.wrap).toBe(false);
    act(() => result.current.undo());
    expect(result.current.builtOn.arrangement).toBe("ring");
    expect(result.current.gradient!.wrap).toBe(true);
  });

  it("C: undoing a motion change never writes a light", async () => {
    const write = vi.fn(async () => {});
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    act(() =>
      result.current.setMotion({ type: "spin", turns_per_s: 0.25 } as Pattern["motion"]),
    );
    await act(async () => {
      result.current.undo();
      await drain();
    });
    expect(write).not.toHaveBeenCalled();
    expect(result.current.motion).toEqual({ type: "none" });
  });

  it("D: a new edit after undoing a shape change clears the redo stack", async () => {
    const write = vi.fn(async () => {});
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    act(() =>
      result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }),
    );
    await act(async () => {
      result.current.undo();
      await drain();
    });
    expect(result.current.canRedo).toBe(true);
    act(() => result.current.stroke([1]));
    expect(result.current.canRedo).toBe(false);
  });

  it("P5: unmounting during the undo's light write sends no draft afterwards", async () => {
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
    act(() =>
      result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }),
    );
    act(() => {
      result.current.undo();
    });
    unmount();
    expect(draft).not.toHaveBeenCalled();
    await act(async () => {
      land();
      await drain();
    });
    expect(draft).not.toHaveBeenCalled();
  });

  it("F: three shape changes and three undos keep the light and the pattern in step", async () => {
    const write = vi.fn(async () => {});
    const { result } = renderHook(() =>
      usePatternEditor(ring({ 0: RED }), { onDraft: () => {}, onShapeLight: write }),
    );
    act(() =>
      result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }),
    );
    act(() =>
      result.current.setShape("ring", undefined, { light: "L", from: "strip", to: "ring" }),
    );
    act(() =>
      result.current.setShape("strip", undefined, { light: "L", from: "ring", to: "strip" }),
    );
    expect(result.current.builtOn.arrangement).toBe("strip");

    for (const arrangement of ["ring", "strip", "ring"] as const) {
      await act(async () => {
        result.current.undo();
        await drain();
      });
      expect(write).toHaveBeenLastCalledWith("L", arrangement);
      expect(result.current.builtOn.arrangement).toBe(arrangement);
    }
  });
});
