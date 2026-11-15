import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { usePatternEditor } from "@/views/patterns/usePatternEditor";
import type { Pattern } from "@/proto/Pattern";

// The editor's state, without a DOM: the tools, the colour field, the history and the document
// the draft sends. The clock is a stubbed rAF, so nothing waits.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";
const RED = "#ff2d55";
const GREEN = "#00c46a";

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

describe("the pattern editor's state", () => {
  it("paint sets the dot to the colour and off sets it to black", () => {
    const { result } = editor();
    act(() => result.current.setColour(GREEN));
    act(() => result.current.setTool("paint"));
    act(() => result.current.stroke([3, 4]));
    expect(colours(result.current.pattern)[3]).toBe(GREEN);
    expect(colours(result.current.pattern)[4]).toBe(GREEN);
    // The rest of the device is untouched.
    expect(colours(result.current.pattern)[5]).toBe(OFF);

    act(() => result.current.setTool("off"));
    act(() => result.current.stroke([3]));
    expect(colours(result.current.pattern)[3]).toBe(OFF);
    expect(colours(result.current.pattern)[4]).toBe(GREEN);
  });

  it("pick takes the colour of a dot into the colour field", () => {
    const { result } = editor(ring({ 0: RED }));
    act(() => result.current.setTool("pick"));
    act(() => result.current.stroke([0]));
    expect(result.current.colour).toBe(RED);
    // Picking reads the device; it never paints it.
    expect(colours(result.current.pattern)[0]).toBe(RED);
    expect(result.current.canUndo).toBe(false);
  });

  it("fill all sets every dot", () => {
    const { result } = editor(ring({ 0: RED }));
    act(() => result.current.setColour(GREEN));
    act(() => result.current.fillAll());
    expect(colours(result.current.pattern)).toEqual(Array(12).fill(GREEN));
  });

  it("undo and redo restore the exact colours", () => {
    const { result } = editor();
    const start = [...colours(result.current.pattern)];
    expect(result.current.canUndo).toBe(false);
    expect(result.current.canRedo).toBe(false);

    act(() => result.current.setColour(RED));
    act(() => {
      result.current.beginStroke();
      result.current.stroke([0, 1, 2]);
      result.current.endStroke();
    });
    const painted = [...colours(result.current.pattern)];
    expect(painted.slice(0, 3)).toEqual([RED, RED, RED]);
    expect(result.current.canUndo).toBe(true);

    act(() => result.current.undo());
    expect(colours(result.current.pattern)).toEqual(start);
    expect(result.current.canRedo).toBe(true);

    act(() => result.current.redo());
    expect(colours(result.current.pattern)).toEqual(painted);
    expect(result.current.canRedo).toBe(false);

    // A stroke made after an undo throws the redo away, as an editor does.
    act(() => result.current.undo());
    act(() => result.current.fillAll());
    expect(result.current.canRedo).toBe(false);
    expect(colours(result.current.pattern)).toEqual(Array(12).fill(RED));
  });
});
