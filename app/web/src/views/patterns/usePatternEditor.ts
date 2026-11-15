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
import { BLACK_HEX, hsvToHex, isHex, parseHex } from "@/lib/color";
import { withColors } from "@/lib/patternFit";
import type { Pattern } from "@/proto/Pattern";
import type { Rgb8 } from "@/proto/Rgb8";

/** The tool in the hand. `fill` fills the whole device, from a key or from any dot. */
export type Tool = "paint" | "off" | "pick" | "fill";

/** How many strokes the history holds. */
export const HISTORY_DEPTH = 100;

/** The colour a pattern starts with when every LED of it is off: white, from the colour model. */
const WHITE = hsvToHex({ h: 0, s: 0, v: 1 });

export interface PatternEditorState {
  /** The document the draft and a save carry: the loaded pattern with this frame's colours. */
  pattern: Pattern;
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

  const commit = useCallback(
    (next: Rgb8[]) => {
      coloursRef.current = next;
      setColours(next);
      pending.current = withColors(base, next);
      if (frame.current !== null) return;
      if (typeof requestAnimationFrame !== "function") {
        flush();
        return;
      }
      frame.current = requestAnimationFrame(flush);
    },
    [base, flush],
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

  const pushUndo = useCallback((snapshot: Rgb8[]) => {
    const next = [...undoRef.current, snapshot];
    undoRef.current = next.length > HISTORY_DEPTH ? next.slice(next.length - HISTORY_DEPTH) : next;
    setUndoStack(undoRef.current);
    redoRef.current = [];
    setRedoStack(redoRef.current);
  }, []);

  const setColour = useCallback((next: string) => {
    colourRef.current = next;
    setColourState(next);
  }, []);

  const setTool = useCallback((next: Tool) => {
    toolRef.current = next;
    setToolState(next);
  }, []);

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

  const pattern = withColors(base, colours);

  const canUndo = undoStack.length > 0;
  const canRedo = redoStack.length > 0;

  const markSaved = useCallback(() => setSaved(coloursRef.current), []);

  return {
    pattern,
    colours,
    colour,
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
    dirty: !sameColours(colours, saved),
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
