"use client";

// The pattern editor's state: the colours of one frame, the tool in the hand, the colour field and
// the history. No DOM, no request: the editor is a colour array per LED, and every change is
// handed to `onDraft` as the whole document, at most once per animation frame.
//
// The document is never rebuilt. `withColors` spreads the loaded pattern and replaces
// `frames[0].colors`, so a pattern a later version wrote keeps its second frame, its motion and
// any key this editor has never heard of (the plan's `saving_keeps_fields_the_editor_does_not_know`).
//
// The editor is mounted on one pattern. A different pattern is a different editor (the component
// that opens it gives it a key), so the loaded document is read once and kept.

import { useCallback, useEffect, useRef, useState } from "react";
import { BLACK_HEX, hexToHsv, hsvToHex, isHex, parseHex } from "@/lib/color";
import {
  gradientColours,
  MAX_STOPS,
  MIN_STOPS,
  seedStops,
  type GradientStop,
} from "@/lib/gradient";
import { withColors } from "@/lib/patternFit";
import type { Shape } from "@/components/ShapePicker";
import type { BuiltOn } from "@/proto/BuiltOn";
import type { GridSize } from "@/proto/GridSize";
import type { Motion } from "@/proto/Motion";
import type { Pattern } from "@/proto/Pattern";
import type { Rgb8 } from "@/proto/Rgb8";

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
  /** The frame's colours, in LED order. */
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
  const [loaded] = useState<Rgb8[]>(() => coloursOf(base, ledCount));

  const [colours, setColours] = useState<Rgb8[]>(loaded);
  const coloursRef = useRef<Rgb8[]>(loaded);
  /** The colours as they were saved last, or loaded: what `dirty` measures against. */
  const [saved, setSaved] = useState<Rgb8[]>(loaded);
  const [colour, setColourState] = useState<string>(() => firstOn(loaded) ?? WHITE);
  const colourRef = useRef(colour);
  const [tool, setToolState] = useState<Tool>("paint");
  const toolRef = useRef<Tool>(tool);
  const [undoStack, setUndoStack] = useState<Rgb8[][]>([]);
  const undoRef = useRef<Rgb8[][]>([]);
  const [redoStack, setRedoStack] = useState<Rgb8[][]>([]);
  const redoRef = useRef<Rgb8[][]>([]);
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

  const pushUndo = useCallback((snapshot: Rgb8[]) => {
    const next = [...undoRef.current, snapshot];
    undoRef.current = next.length > HISTORY_DEPTH ? next.slice(next.length - HISTORY_DEPTH) : next;
    setUndoStack(undoRef.current);
    redoRef.current = [];
    setRedoStack(redoRef.current);
  }, []);

  /** The document as it stands: the loaded pattern with these colours, and the shape and motion in hand. */
  const compose = useCallback(
    (colours: Rgb8[]): Pattern => {
      const doc = withColors(base, colours);
      return {
        ...doc,
        built_on: builtOnRef.current,
        motion: motionRef.current,
      };
    },
    [base],
  );

  /** Queues the document for the lights: at most one request per animation frame, the newest wins. */
  const schedule = useCallback(
    (doc: Pattern) => {
      pending.current = doc;
      if (frame.current !== null) return;
      if (typeof requestAnimationFrame !== "function") {
        flush();
        return;
      }
      frame.current = requestAnimationFrame(flush);
    },
    [flush],
  );

  const commit = useCallback(
    (next: Rgb8[]) => {
      coloursRef.current = next;
      setColours(next);
      schedule(compose(next));
    },
    [compose, schedule],
  );

  const setMotion = useCallback(
    (next: Motion) => {
      motionRef.current = next;
      setMotionState(next);
      schedule(compose(coloursRef.current));
    },
    [compose, schedule],
  );

  const setGradient = useCallback((next: GradientState | null) => {
    gradientRef.current = next;
    setGradientState(next);
  }, []);

  /** Writes the blend of `next`'s stops into the frame. `entry` starts a history entry for it. */
  const applyGradient = useCallback(
    (next: GradientState, entry: "new" | "drag") => {
      setGradient(next);
      const colours = gradientColours(next.stops, coloursRef.current.length, next.wrap);
      if (sameColours(colours, coloursRef.current)) return;
      if (entry === "new" || !inStopDrag.current) {
        if (entry === "drag") inStopDrag.current = true;
        pushUndo(coloursRef.current);
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
      // The Gradient tool seeds its stops from the hue wheel, starting where the colour in hand
      // starts, the first time it is picked; picking it again keeps the stops in place.
      const wrap = builtOnRef.current.arrangement === "ring";
      const seeded = gradientRef.current ?? {
        stops: seedStops(hexToHsv(colourRef.current).h, wrap),
        selected: 0,
        wrap,
      };
      applyGradient(seeded, "new");
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
      const { grid: oldGrid, ...rest } = was;
      const cells = shape === "grid" ? (grid ?? oldGrid) : undefined;
      const next: BuiltOn = { ...rest, arrangement: shape, ...(cells ? { grid: cells } : {}) };
      builtOnRef.current = next;
      setBuiltOnState(next);
      // A ring wraps its gradient; a line does not: the stops are re-laid for the new shape.
      const g = gradientRef.current;
      if (g && g.wrap !== (shape === "ring")) applyGradient({ ...g, wrap: shape === "ring" }, "new");
      else schedule(compose(coloursRef.current));
    },
    [applyGradient, compose, schedule],
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
    pushUndo(current);
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
        pushUndo(current);
      }
      commit(next);
    },
    [commit, fillAll, pushUndo, setColour],
  );

  const undo = useCallback(() => {
    const previous = undoRef.current[undoRef.current.length - 1];
    if (!previous) return;
    undoRef.current = undoRef.current.slice(0, -1);
    setUndoStack(undoRef.current);
    redoRef.current = [...redoRef.current, coloursRef.current];
    setRedoStack(redoRef.current);
    inStroke.current = false;
    commit(previous);
  }, [commit]);

  const redo = useCallback(() => {
    const next = redoRef.current[redoRef.current.length - 1];
    if (!next) return;
    redoRef.current = redoRef.current.slice(0, -1);
    setRedoStack(redoRef.current);
    undoRef.current = [...undoRef.current, coloursRef.current];
    setUndoStack(undoRef.current);
    inStroke.current = false;
    commit(next);
  }, [commit]);

  const pattern: Pattern = {
    ...withColors(base, colours),
    built_on: builtOn,
    motion,
  };

  const canUndo = undoStack.length > 0;
  const canRedo = redoStack.length > 0;

  const markSaved = useCallback(() => {
    setSaved(coloursRef.current);
    setSavedShape({ arrangement: builtOnRef.current.arrangement, motion: motionRef.current });
  }, []);

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
      !sameColours(colours, saved) ||
      builtOn.arrangement !== savedShape.arrangement ||
      JSON.stringify(motion) !== JSON.stringify(savedShape.motion),
  };
}

/** A frame's colours, in LED order, padded to the device's LED count. */
function coloursOf(pattern: Pattern, ledCount: number): Rgb8[] {
  const frame = pattern.frames[0]?.colors ?? [];
  return Array.from({ length: ledCount }, (_, i) => frame[i] ?? BLACK_HEX);
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
