"use client";

// The pattern editor's state: the colours of one frame, the tool in the hand, the colour field and
// the history. No DOM, no request: the editor is a colour array per LED, and every change is
// handed to `onDraft` as the whole document, at most once per animation frame.
//
// The document is never rebuilt. `withDoc` spreads the loaded pattern and replaces its frames, and
// every frame edit spreads the frame it changes, so a pattern a later version wrote keeps its
// motion and any key this editor has never heard of (the plan's
// `saving_keeps_fields_the_editor_does_not_know`).
//
// Frames (#468): the document is a list of frames, one of them selected. Painting edits the
// selected frame; adding, duplicating, deleting and reordering frames, a frame's hold, the
// transition after it, Loop and the fps shortcut edit the list. One history serves all of them: an
// entry is the frames (and Loop) as they were, plus the selection, so undo puts back whatever the
// last change was.
//
// The editor is mounted on one pattern. A different pattern is a different editor (the component
// that opens it gives it a key), so the loaded document is read once and kept.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BLACK_HEX, hexToHsv, hsvToHex, isHex, parseHex } from "@/lib/color";
import {
  gradientColours,
  MAX_STOPS,
  MIN_STOPS,
  seedStops,
  stopsFromColours,
  type GradientStop,
} from "@/lib/gradient";
import {
  addFrame as addFrameTo,
  applyFps as applyFpsTo,
  deleteFrame as deleteFrameFrom,
  docOf,
  duplicateFrame as duplicateFrameIn,
  fpsOf,
  moveFrame as moveFrameIn,
  selectedAfterMove,
  setHold as setHoldOf,
  setLoop as setLoopOf,
  setTransition as setTransitionOf,
  withDoc,
  type FrameDoc,
} from "@/lib/patternFrames";
import type { Shape } from "@/components/ShapePicker";
import type { BuiltOn } from "@/proto/BuiltOn";
import type { Frame } from "@/proto/Frame";
import type { GridSize } from "@/proto/GridSize";
import type { Motion } from "@/proto/Motion";
import type { Pattern } from "@/proto/Pattern";
import type { Rgb8 } from "@/proto/Rgb8";
import type { Transition } from "@/proto/Transition";

/**
 * The tool in the hand. `fill` fills the whole device, from a key or from any dot. `gradient`
 * (#468) lays stops round the ring (or along the line) and writes the blend of them into the frame.
 */
export type Tool = "paint" | "off" | "pick" | "fill" | "gradient";

/** The Gradient tool's state: editor-only. The colours it writes are the truth, nothing here is saved. */
export interface GradientState {
  stops: GradientStop[];
  /** The stop the colour field is editing. */
  selected: number;
  /** The last stop meets the first across the seam: a ring's default, a line's off. */
  wrap: boolean;
}

/** How many strokes the history holds. */
export const HISTORY_DEPTH = 100;

/** The colour a pattern starts with when every LED of it is off: white, from the colour model. */
const WHITE = hsvToHex({ h: 0, s: 0, v: 1 });

export interface PatternEditorState {
  /** The document the draft and a save carry: the loaded pattern with this frame's colours, its shape and its motion. */
  pattern: Pattern;
  /** What the pattern is built on now: the loaded shape until the owner picks another. */
  builtOn: BuiltOn;
  /** How the frames move now: the loaded motion until the owner changes it. */
  motion: Motion;
  /** Sets the motion (Spin, or none). The draft goes out like a stroke. */
  setMotion(motion: Motion): void;
  /** Lays the pattern out as another shape; a grid takes the cells the zone reports. */
  setShape(shape: Shape, grid?: GridSize): void;
  /** The Gradient tool's stops and wrap, or null until the tool has been picked. */
  gradient: GradientState | null;
  selectStop(index: number): void;
  /** Moves a stop to a position and writes the new blend. One drag is one history entry. */
  moveStop(index: number, at: number): void;
  addStop(): void;
  removeStop(index: number): void;
  setWrap(wrap: boolean): void;
  /** A stop's drag began / ended: the moves between are one history entry. */
  beginStopDrag(): void;
  endStopDrag(): void;
  /** The frames, in play order. */
  frames: Frame[];
  /** The frame being painted. */
  selected: number;
  selectFrame(index: number): void;
  /** A new blank frame after the selected one; it becomes the selected one. */
  addFrame(): void;
  /** A copy of the selected frame right after it. */
  duplicateFrame(): void;
  /** The selected frame removed (a pattern keeps at least one). */
  deleteFrame(): void;
  /** Frame `from` moved to place `to`; the selection follows the frame it was on. */
  moveFrame(from: number, to: number): void;
  /** How long a frame holds, in ms. */
  setHold(index: number, ms: number): void;
  /** How a frame passes to the next (for the last, to the first). */
  setTransition(index: number, patch: Partial<Transition>): void;
  /** After the last frame, back to the first (the default) or stay on the last. */
  loop: boolean;
  setLoop(loop: boolean): void;
  /** Every frame holds `round(1000 / fps)` ms and every transition is a cut. */
  applyFps(fps: number): void;
  /** The fps the holds come to, or null when they differ (no fps is ever stored). */
  fps: number | null;
  /** The selected frame's colours, in LED order. */
  colours: Rgb8[];
  colour: string;
  tool: Tool;
  setColour(colour: string): void;
  setTool(tool: Tool): void;
  /** Applies the current tool to those dots, as one part of a stroke. */
  stroke(indices: number[]): void;
  /** A pointer went down: the next change is the start of one history entry. */
  beginStroke(): void;
  /** The pointer went up: the next change starts a new entry. */
  endStroke(): void;
  fillAll(): void;
  undo(): void;
  redo(): void;
  /** The document has been saved as it stands: `dirty` clears until the next change. */
  markSaved(): void;
  canUndo: boolean;
  canRedo: boolean;
  /** The colours differ from the loaded (or last saved) ones. */
  dirty: boolean;
}

export function usePatternEditor(
  initial: Pattern,
  opts: { onDraft(pattern: Pattern): void },
): PatternEditorState {
  // The loaded document: the frame's colours padded to the device's LED count, so a pattern that
  // is missing colours still draws every LED.
  const [base] = useState<Pattern>(initial);
  const [builtOn, setBuiltOnState] = useState<BuiltOn>(base.built_on);
  const builtOnRef = useRef<BuiltOn>(base.built_on);
  const [motion, setMotionState] = useState<Motion>(base.motion);
  const motionRef = useRef<Motion>(base.motion);
  /** The shape and motion as saved last, or loaded: what `dirty` measures against. */
  const [savedShape, setSavedShape] = useState<{ arrangement: BuiltOn["arrangement"]; motion: Motion }>({
    arrangement: base.built_on.arrangement,
    motion: base.motion,
  });
  const ledCount = Math.max(1, Math.floor(base.built_on.led_count));
  // The loaded frames, each padded to the device's LED count so a frame that is missing colours
  // still draws every LED. A pattern with no frames (validation refuses one) gets a blank one.
  const [loaded] = useState<FrameDoc>(() => loadedDoc(base, ledCount));
  const [doc, setDocState] = useState<FrameDoc>(loaded);
  const docRef = useRef<FrameDoc>(loaded);
  const [selected, setSelectedState] = useState(0);
  const selectedRef = useRef(0);
  /** The selected frame's colours: what the tools paint. */
  const coloursRef = useRef<Rgb8[]>(loaded.frames[0]?.colors ?? []);
  /** The frames as they were saved last, or loaded: what `dirty` measures against. */
  const [saved, setSaved] = useState<string>(() => docKey(loaded));
  const [colour, setColourState] = useState<string>(() => firstOn(loaded.frames[0]?.colors ?? []) ?? WHITE);
  const colourRef = useRef(colour);
  const [tool, setToolState] = useState<Tool>("paint");
  const toolRef = useRef<Tool>(tool);
  const [undoStack, setUndoStack] = useState<Snapshot[]>([]);
  const undoRef = useRef<Snapshot[]>([]);
  const [redoStack, setRedoStack] = useState<Snapshot[]>([]);
  const redoRef = useRef<Snapshot[]>([]);
  /** True between the first change of a pointer stroke and its pointer-up. */
  const inStroke = useRef(false);
  const [gradient, setGradientState] = useState<GradientState | null>(null);
  const gradientRef = useRef<GradientState | null>(null);
  /** True between a stop's first move and its release: one history entry. */
  const inStopDrag = useRef(false);

  // The draft goes out on the animation frame, not on the pointer event: a drag is many events
  // and at most one request per frame. `pending` keeps the newest document, so a frame carries the
  // whole stroke so far and never an older colour array.
  const draft = useRef(opts.onDraft);
  const pending = useRef<Pattern | null>(null);
  const frame = useRef<number | null>(null);

  useEffect(() => {
    draft.current = opts.onDraft;
  }, [opts.onDraft]);

  const flush = useCallback(() => {
    frame.current = null;
    const next = pending.current;
    pending.current = null;
    if (next) draft.current(next);
  }, []);

  /** Puts the document as it stands on the history, before a change to it (#481). */
  const pushUndo = useCallback(() => {
    const next = [
      ...undoRef.current,
      { doc: docRef.current, selected: selectedRef.current, builtOn: builtOnRef.current, motion: motionRef.current },
    ];
    undoRef.current = next.length > HISTORY_DEPTH ? next.slice(next.length - HISTORY_DEPTH) : next;
    setUndoStack(undoRef.current);
    redoRef.current = [];
    setRedoStack(redoRef.current);
  }, []);

  /** The document as it stands: the loaded pattern with these frames, and the shape and motion in hand. */
  const compose = useCallback(
    (frames: FrameDoc): Pattern => ({
      ...withDoc(base, frames),
      built_on: builtOnRef.current,
      motion: motionRef.current,
    }),
    [base],
  );

  /** Queues the document for the lights: at most one request per animation frame, the newest wins. */
  const schedule = useCallback(
    (next: Pattern) => {
      pending.current = next;
      if (frame.current !== null) return;
      if (typeof requestAnimationFrame !== "function") {
        flush();
        return;
      }
      frame.current = requestAnimationFrame(flush);
    },
    [flush],
  );

  /** Makes these frames and this selection the document, and sends it to the lights. */
  const applyDoc = useCallback(
    (next: FrameDoc, at: number) => {
      const index = Math.min(Math.max(0, at), next.frames.length - 1);
      docRef.current = next;
      selectedRef.current = index;
      coloursRef.current = next.frames[index]?.colors ?? [];
      setDocState(next);
      setSelectedState(index);
      schedule(compose(next));
    },
    [compose, schedule],
  );

  /** The selected frame's colours replaced. */
  const commit = useCallback(
    (next: Rgb8[]) => {
      const here = docRef.current;
      const at = selectedRef.current;
      applyDoc({ ...here, frames: here.frames.map((f, i) => (i === at ? { ...f, colors: next } : f)) }, at);
    },
    [applyDoc],
  );

  const setMotion = useCallback(
    (next: Motion) => {
      if (sameMotion(motionRef.current, next)) return;
      pushUndo();
      motionRef.current = next;
      setMotionState(next);
      schedule(compose(docRef.current));
    },
    [compose, pushUndo, schedule],
  );

  const setGradient = useCallback((next: GradientState | null) => {
    gradientRef.current = next;
    setGradientState(next);
  }, []);

  /**
   * Writes the blend of `next`'s stops into the frame. `entry` starts a history entry for it;
   * `"silent"` leaves that to the caller (a shape change pushes its own entry first, #481).
   */
  const applyGradient = useCallback(
    (next: GradientState, entry: "new" | "drag" | "silent") => {
      setGradient(next);
      const colours = gradientColours(next.stops, coloursRef.current.length, next.wrap);
      if (sameColours(colours, coloursRef.current)) return;
      if (entry === "new") pushUndo();
      else if (entry === "drag" && !inStopDrag.current) {
        inStopDrag.current = true;
        pushUndo();
      }
      commit(colours);
    },
    [commit, pushUndo, setGradient],
  );

  // A queued frame is dropped with the editor: its draft is the draft's `DELETE`.
  useEffect(
    () => () => {
      if (frame.current !== null) {
        if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(frame.current);
        frame.current = null;
      }
      pending.current = null;
    },
    [],
  );

  /** The colour in hand: with the Gradient tool, the selected stop's. */
  const setColour = useCallback(
    (next: string) => {
      colourRef.current = next;
      setColourState(next);
      const g = gradientRef.current;
      if (toolRef.current === "gradient" && g) {
        const stops = g.stops.map((stop, i) => (i === g.selected ? { ...stop, colour: next } : stop));
        applyGradient({ ...g, stops }, "new");
      }
    },
    [applyGradient],
  );

  const setTool = useCallback(
    (next: Tool) => {
      toolRef.current = next;
      setToolState(next);
      if (next !== "gradient") return;
      // The first pick seeds the stops. A pattern that already has colours painted on it keeps
      // them: the stops are read off those colours and no blend is written until the owner edits a
      // stop (#481). An all-off pattern has nothing to lose, so it still gets the hue wheel's new
      // gradient. Picking the tool again keeps the stops in place.
      const wrap = builtOnRef.current.arrangement === "ring";
      const painted = firstOn(coloursRef.current);
      const seeded = gradientRef.current ?? (painted
        ? { stops: stopsFromColours(coloursRef.current, wrap), selected: 0, wrap }
        : { stops: seedStops(hexToHsv(colourRef.current).h, wrap), selected: 0, wrap });
      setGradient(seeded);
      if (!painted) applyGradient(seeded, "new");
    },
    [applyGradient],
  );

  const selectStop = useCallback(
    (index: number) => {
      const g = gradientRef.current;
      if (!g || index < 0 || index >= g.stops.length) return;
      setGradient({ ...g, selected: index });
    },
    [setGradient],
  );

  const moveStop = useCallback(
    (index: number, at: number) => {
      const g = gradientRef.current;
      const stop = g?.stops[index];
      if (!g || !stop) return;
      const bounded = g.wrap ? ((at % 1) + 1) % 1 : Math.min(1, Math.max(0, at));
      if (bounded === stop.at) return;
      const stops = g.stops.map((s, i) => (i === index ? { ...s, at: bounded } : s));
      applyGradient({ ...g, stops, selected: index }, "drag");
    },
    [applyGradient],
  );

  const addStop = useCallback(() => {
    const g = gradientRef.current;
    if (!g || g.stops.length >= MAX_STOPS) return;
    // The new stop goes in the middle of the widest gap, with the colour already there.
    const sorted = g.stops.map((s, i) => ({ ...s, i })).sort((a, b) => a.at - b.at);
    let best = { gap: -1, at: 0 };
    sorted.forEach((s, k) => {
      const nextAt = k + 1 < sorted.length ? (sorted[k + 1] as { at: number }).at : g.wrap ? (sorted[0] as { at: number }).at + 1 : s.at;
      const gap = nextAt - s.at;
      if (gap > best.gap) best = { gap, at: (s.at + gap / 2) % 1 };
    });
    const colours = gradientColours(g.stops, coloursRef.current.length, g.wrap);
    const led = Math.min(colours.length - 1, Math.max(0, Math.round(best.at * (g.wrap ? colours.length : colours.length - 1))));
    const stops = [...g.stops, { at: best.at, colour: colours[led] ?? BLACK_HEX }];
    applyGradient({ ...g, stops, selected: stops.length - 1 }, "new");
  }, [applyGradient]);

  const removeStop = useCallback(
    (index: number) => {
      const g = gradientRef.current;
      if (!g || g.stops.length <= MIN_STOPS) return;
      const stops = g.stops.filter((_, i) => i !== index);
      applyGradient({ ...g, stops, selected: Math.min(g.selected, stops.length - 1) }, "new");
    },
    [applyGradient],
  );

  const setWrap = useCallback(
    (wrap: boolean) => {
      const g = gradientRef.current;
      if (!g || g.wrap === wrap) return;
      applyGradient({ ...g, wrap }, "new");
    },
    [applyGradient],
  );

  const beginStopDrag = useCallback(() => {
    inStopDrag.current = false;
  }, []);
  const endStopDrag = useCallback(() => {
    inStopDrag.current = false;
  }, []);

  const setShape = useCallback(
    (shape: Shape, grid?: GridSize) => {
      const was = builtOnRef.current;
      if (was.arrangement === shape && (shape !== "grid" || grid === undefined)) return;
      pushUndo();
      const { grid: oldGrid, ...rest } = was;
      const cells = shape === "grid" ? (grid ?? oldGrid) : undefined;
      const next: BuiltOn = { ...rest, arrangement: shape, ...(cells ? { grid: cells } : {}) };
      builtOnRef.current = next;
      setBuiltOnState(next);
      // A ring wraps its gradient; a line does not: the stops are re-laid for the new shape.
      const g = gradientRef.current;
      if (g && g.wrap !== (shape === "ring")) applyGradient({ ...g, wrap: shape === "ring" }, "silent");
      else schedule(compose(docRef.current));
    },
    [applyGradient, compose, pushUndo, schedule],
  );

  const beginStroke = useCallback(() => {
    inStroke.current = false;
  }, []);

  const endStroke = useCallback(() => {
    inStroke.current = false;
  }, []);

  const fillAll = useCallback(() => {
    const current = coloursRef.current;
    const value = toolRef.current === "off" ? BLACK_HEX : colourRef.current;
    if (current.every((c) => c === value)) return;
    pushUndo();
    commit(Array.from({ length: current.length }, () => value));
  }, [commit, pushUndo]);

  const stroke = useCallback(
    (indices: number[]) => {
      const current = coloursRef.current;
      const valid = indices.filter((i) => Number.isInteger(i) && i >= 0 && i < current.length);
      if (valid.length === 0) return;

      if (toolRef.current === "pick") {
        const from = valid.find((i) => isOn(current[i]));
        const picked = from === undefined ? null : current[from];
        if (picked !== null && picked !== undefined) setColour(picked);
        return;
      }
      if (toolRef.current === "fill") {
        fillAll();
        return;
      }
      // The Gradient tool is driven by its stops, not by a press on a dot.
      if (toolRef.current === "gradient") return;

      const value = toolRef.current === "off" ? BLACK_HEX : colourRef.current;
      const next = [...current];
      let changed = false;
      for (const i of valid) {
        if (next[i] !== value) {
          next[i] = value;
          changed = true;
        }
      }
      if (!changed) return;
      if (!inStroke.current) {
        inStroke.current = true;
        pushUndo();
      }
      commit(next);
    },
    [commit, fillAll, pushUndo, setColour],
  );

  /** Restores everything a snapshot holds: the frames, the shape and the motion (#481). */
  const restore = useCallback(
    (snapshot: Snapshot) => {
      builtOnRef.current = snapshot.builtOn;
      setBuiltOnState(snapshot.builtOn);
      motionRef.current = snapshot.motion;
      setMotionState(snapshot.motion);
      applyDoc(snapshot.doc, snapshot.selected);
    },
    [applyDoc],
  );

  const undo = useCallback(() => {
    const previous = undoRef.current[undoRef.current.length - 1];
    if (!previous) return;
    undoRef.current = undoRef.current.slice(0, -1);
    setUndoStack(undoRef.current);
    redoRef.current = [
      ...redoRef.current,
      { doc: docRef.current, selected: selectedRef.current, builtOn: builtOnRef.current, motion: motionRef.current },
    ];
    setRedoStack(redoRef.current);
    inStroke.current = false;
    restore(previous);
  }, [restore]);

  const redo = useCallback(() => {
    const next = redoRef.current[redoRef.current.length - 1];
    if (!next) return;
    redoRef.current = redoRef.current.slice(0, -1);
    setRedoStack(redoRef.current);
    undoRef.current = [
      ...undoRef.current,
      { doc: docRef.current, selected: selectedRef.current, builtOn: builtOnRef.current, motion: motionRef.current },
    ];
    setUndoStack(undoRef.current);
    inStroke.current = false;
    restore(next);
  }, [restore]);

  // ---- frames (#468). Each change is one history entry and goes to the lights at once.

  /** A frame edit: the history takes the document as it was, then the new one is the document. */
  const edit = useCallback(
    (next: FrameDoc | null, at: number) => {
      if (next === null || next === docRef.current) return;
      pushUndo();
      inStroke.current = false;
      applyDoc(next, at);
    },
    [applyDoc, pushUndo],
  );

  const selectFrame = useCallback(
    (index: number) => {
      if (index === selectedRef.current || index < 0 || index >= docRef.current.frames.length) return;
      // The Gradient tool's stops describe the frame they were laid on: another frame starts over.
      if (gradientRef.current !== null) {
        gradientRef.current = null;
        setGradientState(null);
        if (toolRef.current === "gradient") {
          toolRef.current = "paint";
          setToolState("paint");
        }
      }
      inStroke.current = false;
      selectedRef.current = index;
      coloursRef.current = docRef.current.frames[index]?.colors ?? [];
      setSelectedState(index);
    },
    [],
  );

  const addFrame = useCallback(() => {
    const made = addFrameTo(docRef.current, selectedRef.current);
    if (made) edit(made.doc, made.selected);
  }, [edit]);

  const duplicateFrame = useCallback(() => {
    const made = duplicateFrameIn(docRef.current, selectedRef.current);
    if (made) edit(made.doc, made.selected);
  }, [edit]);

  const deleteFrame = useCallback(() => {
    const made = deleteFrameFrom(docRef.current, selectedRef.current);
    if (made) edit(made.doc, made.selected);
  }, [edit]);

  const moveFrame = useCallback(
    (from: number, to: number) => {
      edit(moveFrameIn(docRef.current, from, to), selectedAfterMove(selectedRef.current, from, to));
    },
    [edit],
  );

  const setHold = useCallback(
    (index: number, ms: number) => edit(setHoldOf(docRef.current, index, ms), selectedRef.current),
    [edit],
  );

  const setTransition = useCallback(
    (index: number, patch: Partial<Transition>) =>
      edit(setTransitionOf(docRef.current, index, patch), selectedRef.current),
    [edit],
  );

  const setLoop = useCallback(
    (loop: boolean) => edit(setLoopOf(docRef.current, loop), selectedRef.current),
    [edit],
  );

  const applyFps = useCallback(
    (fps: number) => edit(applyFpsTo(docRef.current, fps), selectedRef.current),
    [edit],
  );

  const pattern: Pattern = {
    ...withDoc(base, doc),
    built_on: builtOn,
    motion,
  };

  const canUndo = undoStack.length > 0;
  const canRedo = redoStack.length > 0;

  const markSaved = useCallback(() => {
    setSaved(docKey(docRef.current));
    setSavedShape({ arrangement: builtOnRef.current.arrangement, motion: motionRef.current });
  }, []);

  const colours = doc.frames[selected]?.colors ?? [];
  const key = useMemo(() => docKey(doc), [doc]);

  const shownColour =
    tool === "gradient" && gradient ? (gradient.stops[gradient.selected]?.colour ?? colour) : colour;

  return {
    pattern,
    builtOn,
    motion,
    setMotion,
    setShape,
    gradient,
    selectStop,
    moveStop,
    addStop,
    removeStop,
    setWrap,
    beginStopDrag,
    endStopDrag,
    frames: doc.frames,
    selected,
    selectFrame,
    addFrame,
    duplicateFrame,
    deleteFrame,
    moveFrame,
    setHold,
    setTransition,
    loop: doc.playback?.loop !== false,
    setLoop,
    applyFps,
    fps: fpsOf(doc),
    colours,
    colour: shownColour,
    tool,
    setColour,
    setTool,
    stroke,
    beginStroke,
    endStroke,
    fillAll,
    undo,
    redo,
    markSaved,
    canUndo,
    canRedo,
    dirty:
      key !== saved ||
      builtOn.arrangement !== savedShape.arrangement ||
      JSON.stringify(motion) !== JSON.stringify(savedShape.motion),
  };
}

/** One entry of the history: the frames (and Loop), the shape, the motion and the selected frame. */
interface Snapshot {
  doc: FrameDoc;
  selected: number;
  builtOn: BuiltOn;
  motion: Motion;
}

/** Motion is a small JSON value; equality by value keeps a re-pick off the history. */
function sameMotion(a: Motion, b: Motion): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The loaded frames, each padded to the device's LED count; a pattern with none gets one blank frame. */
function loadedDoc(pattern: Pattern, ledCount: number): FrameDoc {
  const pad = (frame: Frame): Frame => {
    if (frame.colors.length === ledCount) return frame;
    return { ...frame, colors: Array.from({ length: ledCount }, (_, i) => frame.colors[i] ?? BLACK_HEX) };
  };
  const doc = docOf(pattern);
  const frames = doc.frames.length > 0 ? doc.frames.map(pad) : [{ colors: Array.from({ length: ledCount }, () => BLACK_HEX), hold: 0 }];
  return { ...doc, frames };
}

/** The frames and Loop as text: what `dirty` compares, by value, so a stroke painted back is no change. */
function docKey(doc: FrameDoc): string {
  return JSON.stringify([doc.frames, doc.playback ?? null]);
}

/** The first colour that is on, or null when every LED is off. */
function firstOn(colours: Rgb8[]): string | null {
  for (const c of colours) if (isOn(c) && isHex(c)) return c.toLowerCase();
  return null;
}

/** True when a colour is anything but off. A value that is not a colour reads as off. */
function isOn(colour: string | undefined): boolean {
  if (colour === undefined) return false;
  const hex = parseHex(colour);
  if (hex === null) return false;
  return hex !== BLACK_HEX;
}

function sameColours(a: Rgb8[], b: Rgb8[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}
