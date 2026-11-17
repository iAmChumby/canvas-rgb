import { describe, expect, it } from "vitest";
import {
  addFrame,
  applyFps,
  deleteFrame,
  docOf,
  duplicateFrame,
  fpsOf,
  moveFrame,
  selectedAfterMove,
  setHold,
  setLoop,
  setTransition,
  withDoc,
  type FrameDoc,
} from "@/lib/patternFrames";
import type { Frame } from "@/proto/Frame";
import type { Pattern } from "@/proto/Pattern";

const frame = (c: string, hold = 500, extra: Record<string, unknown> = {}): Frame =>
  ({ colors: [c, c], hold, ...extra }) as Frame;
const three = (): FrameDoc => ({ frames: [frame("#ff0000", 100), frame("#00ff00", 200), frame("#0000ff", 300)] });

describe("editing the frames of a pattern", () => {
  it("adds a blank frame after the selected one, holding as long as it, and selects it", () => {
    const made = addFrame(three(), 0)!;
    expect(made.selected).toBe(1);
    expect(made.doc.frames.map((f) => f.colors[0])).toEqual(["#ff0000", "#000000", "#00ff00", "#0000ff"]);
    expect(made.doc.frames[1]).toEqual({ colors: ["#000000", "#000000"], hold: 100 });
  });

  it("gives a new frame half a second when the frame beside it holds none", () => {
    const made = addFrame({ frames: [frame("#ff0000", 0)] }, 0)!;
    expect(made.doc.frames[1]!.hold).toBe(500);
  });

  it("duplicates a frame with its hold, its transition and any field it carries", () => {
    const doc: FrameDoc = {
      frames: [
        { ...frame("#ff0000", 100, { label: "kept" }), transition: { kind: "fade", ms: 300, easing: "ease_in" } },
        frame("#00ff00"),
      ],
    };
    const made = duplicateFrame(doc, 0)!;
    expect(made.selected).toBe(1);
    expect(made.doc.frames[1]).toEqual(doc.frames[0]);
    // A copy, not the same colour array: painting one frame never paints the other.
    expect(made.doc.frames[1]!.colors).not.toBe(doc.frames[0]!.colors);
  });

  it("refuses an add past the cap and a delete of the only frame", () => {
    const full: FrameDoc = { frames: Array.from({ length: 256 }, () => frame("#ff0000")) };
    expect(addFrame(full, 0)).toBeNull();
    expect(duplicateFrame(full, 0)).toBeNull();
    expect(deleteFrame({ frames: [frame("#ff0000")] }, 0)).toBeNull();
  });

  it("deletes a frame and the selection lands on the one that took its place", () => {
    expect(deleteFrame(three(), 1)!.selected).toBe(1);
    expect(deleteFrame(three(), 2)!.selected).toBe(1);
    expect(deleteFrame(three(), 0)!.doc.frames.map((f) => f.hold)).toEqual([200, 300]);
  });

  it("moves a frame and the selection follows the frame it was on", () => {
    const doc = three();
    expect(moveFrame(doc, 0, 2)!.frames.map((f) => f.hold)).toEqual([200, 300, 100]);
    expect(moveFrame(doc, 1, 1)).toBeNull();
    expect(moveFrame(doc, 0, 3)).toBeNull();
    expect(selectedAfterMove(0, 0, 2)).toBe(2);
    expect(selectedAfterMove(1, 0, 2)).toBe(0);
    expect(selectedAfterMove(2, 0, 1)).toBe(2);
    expect(selectedAfterMove(1, 2, 0)).toBe(2);
  });

  it("never changes the document it is given", () => {
    const doc = three();
    const snapshot = JSON.stringify(doc);
    addFrame(doc, 1);
    deleteFrame(doc, 1);
    moveFrame(doc, 0, 2);
    setHold(doc, 0, 999);
    setTransition(doc, 0, { kind: "fade" });
    setLoop(doc, false);
    applyFps(doc, 10);
    expect(JSON.stringify(doc)).toBe(snapshot);
  });

  it("sets a hold as a whole number inside the spine's cap", () => {
    expect(setHold(three(), 0, 250.6).frames[0]!.hold).toBe(251);
    expect(setHold(three(), 0, -5).frames[0]!.hold).toBe(0);
    expect(setHold(three(), 0, 9_999_999).frames[0]!.hold).toBe(600_000);
    expect(setHold(three(), 0, Number.NaN).frames[0]!.hold).toBe(0);
    const same = three();
    expect(setHold(same, 0, 100)).toBe(same);
  });

  it("keeps the fields a frame carries through every edit", () => {
    const doc: FrameDoc = { frames: [frame("#ff0000", 100, { label: "kept" }), frame("#00ff00")] };
    for (const next of [setHold(doc, 0, 5), setTransition(doc, 0, { kind: "fade" }), applyFps(doc, 10), moveFrame(doc, 0, 1)!]) {
      expect(next.frames.some((f) => (f as unknown as { label?: string }).label === "kept")).toBe(true);
    }
  });
});

describe("a frame's transition", () => {
  it("writes no transition for a cut that was never a fade (the 0.16.0 shape)", () => {
    const doc = three();
    expect(setTransition(doc, 0, { kind: "cut" })).toBe(doc);
    expect(doc.frames[0]!.transition).toBeUndefined();
  });

  it("makes a fade of 400 ms, linear, when Fade is first chosen", () => {
    const next = setTransition(three(), 0, { kind: "fade" });
    expect(next.frames[0]!.transition).toEqual({ kind: "fade", ms: 400, easing: "linear" });
  });

  it("changes the duration and the pace on their own, and clamps the duration", () => {
    let doc = setTransition(three(), 1, { kind: "fade" });
    doc = setTransition(doc, 1, { ms: 1234.4 });
    doc = setTransition(doc, 1, { easing: "ease_out" });
    expect(doc.frames[1]!.transition).toEqual({ kind: "fade", ms: 1234, easing: "ease_out" });
    expect(setTransition(doc, 1, { ms: 1e9 }).frames[1]!.transition!.ms).toBe(60_000);
    expect(setTransition(doc, 1, { ms: -1 }).frames[1]!.transition!.ms).toBe(0);
  });

  it("remembers the fade's length and pace when it is flipped to Cut and back", () => {
    let doc = setTransition(three(), 0, { kind: "fade", ms: 900, easing: "ease_in_out" });
    doc = setTransition(doc, 0, { kind: "cut" });
    expect(doc.frames[0]!.transition!.kind).toBe("cut");
    doc = setTransition(doc, 0, { kind: "fade" });
    expect(doc.frames[0]!.transition).toEqual({ kind: "fade", ms: 900, easing: "ease_in_out" });
  });
});

describe("Loop", () => {
  it("is on by default and written as no playback at all", () => {
    const doc = three();
    expect(setLoop(doc, true)).toBe(doc);
    const off = setLoop(doc, false);
    expect(off.playback).toEqual({ loop: false });
    const back = setLoop(off, true);
    expect("playback" in back).toBe(false);
  });
});

describe("fps", () => {
  it("sets every hold to round(1000 / fps) and every transition to a cut, storing no fps", () => {
    const doc: FrameDoc = {
      frames: [
        { ...frame("#ff0000", 100), transition: { kind: "fade", ms: 300, easing: "linear" } },
        { ...frame("#00ff00", 900), transition: { kind: "cut", ms: 50, easing: "ease_in" } },
        frame("#0000ff", 40),
      ],
    };
    const next = applyFps(doc, 12);
    expect(next.frames.map((f) => f.hold)).toEqual([83, 83, 83]);
    expect(next.frames.every((f) => f.transition === undefined)).toBe(true);
    expect(JSON.stringify(next)).not.toMatch(/fps/);
    expect(applyFps(doc, 24).frames[0]!.hold).toBe(42);
    expect(applyFps(doc, 0).frames[0]!.hold).toBe(1000);
    expect(applyFps(doc, 500).frames[0]!.hold).toBe(17);
  });

  it("reads the fps back from the holds when they are alike and every transition cuts", () => {
    expect(fpsOf(applyFps(three(), 10))).toBe(10);
    expect(fpsOf(applyFps(three(), 24))).toBe(24);
    expect(fpsOf(three())).toBeNull();
    expect(fpsOf(setTransition(applyFps(three(), 10), 0, { kind: "fade" }))).toBeNull();
    expect(fpsOf({ frames: [frame("#ff0000", 0)] })).toBeNull();
  });
});

describe("the document", () => {
  const loaded = {
    id: "pattern:X",
    built_on: { led_count: 2, arrangement: "strip" },
    frames: [frame("#ff0000"), frame("#00ff00")],
    motion: { type: "rotate", speed: 2 },
    tags: ["kept"],
  } as unknown as Pattern;

  it("puts the frames and Loop back into the pattern and spreads everything else as it came", () => {
    const doc = setLoop(docOf(loaded), false);
    const saved = withDoc(loaded, doc);
    expect(saved.playback).toEqual({ loop: false });
    expect((saved as unknown as { tags: string[] }).tags).toEqual(["kept"]);
    expect(saved.motion).toEqual({ type: "rotate", speed: 2 });
    expect("playback" in withDoc(saved, setLoop(doc, true))).toBe(false);
  });
});
