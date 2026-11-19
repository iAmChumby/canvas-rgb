// The frames of a pattern, edited (#468): add, duplicate, delete, reorder, a frame's hold, the
// transition after it, Loop, and the fps shortcut.
//
// Pure functions over the document's frames. Each returns new frames and never changes the ones it
// is given, so an undo entry is the frames as they were, shared and free. A frame is always
// *spread* when it changes (`{ ...frame, hold }`), never rebuilt, so a field a later version wrote
// into it (the generated `Frame` type cannot hold the daemon's flattened extras) survives every edit.

import { BLACK_HEX } from "@/lib/color";
import { HOLD_MS_MAX, PATTERN_FRAMES_MAX, TRANSITION_MS_MAX } from "@/lib/patternPlay";
import type { Easing } from "@/proto/Easing";
import type { Frame } from "@/proto/Frame";
import type { Pattern } from "@/proto/Pattern";
import type { Playback } from "@/proto/Playback";
import type { Transition } from "@/proto/Transition";

/** The frames and Loop of a document: what the strip edits and an undo restores. */
export interface FrameDoc {
  frames: Frame[];
  playback?: Playback;
}

/** How long a new frame holds when no frame beside it says: half a second. */
export const NEW_HOLD_MS = 500;
/** How long a fade takes when the owner first chooses Fade. */
export const NEW_FADE_MS = 400;
/** The fps control's range: 1 frame a second up to 60. */
export const FPS_MIN = 1;
export const FPS_MAX = 60;

/** A whole number of milliseconds within 0..max; anything that is not a number is 0. */
function wholeMs(ms: number, max: number): number {
  return Number.isFinite(ms) ? Math.min(max, Math.max(0, Math.round(ms))) : 0;
}

/** Whether another frame may be added: a pattern holds at most `PATTERN_FRAMES_MAX`. */
export function canAdd(doc: FrameDoc): boolean {
  return doc.frames.length < PATTERN_FRAMES_MAX;
}

/** `frame` with `transition` as its connector; undefined is a cut, written as no transition at all. */
function withConnector(frame: Frame, transition: Transition | undefined): Frame {
  if (JSON.stringify(frame.transition) === JSON.stringify(transition)) return frame;
  const next: Frame = { ...frame };
  if (transition === undefined) delete next.transition;
  else next.transition = transition;
  return next;
}

/**
 * The loop connector (last frame to frame 1, or Stop) belongs to the last PLACE, not to a frame
 * (#468). The engine reads it from the last frame's `transition`, so every edit that changes which
 * frame is last hands it over: the new last frame takes `loop`, and the frame that stops being last
 * (`demoted`, an index in `frames`) becomes a middle connector, a cut like any new one. Middle
 * connectors stay with their frames.
 */
function carryLoop(frames: Frame[], loop: Transition | undefined, demoted: number | null): Frame[] {
  const last = frames.length - 1;
  return frames.map((f, i) => (i === last ? withConnector(f, loop) : i === demoted ? withConnector(f, undefined) : f));
}

/** A new frame after `at`: every LED off, holding as long as the frame it follows, passing to the next as a cut. */
export function addFrame(doc: FrameDoc, at: number): { doc: FrameDoc; selected: number } | null {
  if (!canAdd(doc)) return null;
  const here = doc.frames[at];
  const colors = (here?.colors ?? []).map(() => BLACK_HEX);
  const fresh: Frame = { colors, hold: here?.hold ? here.hold : NEW_HOLD_MS };
  // A frame that holds 0 ms is never shown once a second frame exists (a still pattern, and every
  // 0.16.0 pattern, holds 0): it takes the same half second, in the same edit, so the first Add on a
  // still pattern plays both frames instead of only the new blank one.
  const kept = here && here.hold === 0 ? { ...here, hold: NEW_HOLD_MS } : here;
  const before = kept ? [...doc.frames.slice(0, at), kept] : doc.frames.slice(0, at + 1);
  const added = [...before, fresh, ...doc.frames.slice(at + 1)];
  // Added at the end: the loop connector goes to the new last frame; the old one now passes to it as a cut.
  const atEnd = at === doc.frames.length - 1;
  const frames = atEnd ? carryLoop(added, here?.transition, at) : added;
  return { doc: { ...doc, frames }, selected: at + 1 };
}

/** A copy of frame `at` right after it, hold and transition too. */
export function duplicateFrame(doc: FrameDoc, at: number): { doc: FrameDoc; selected: number } | null {
  const here = doc.frames[at];
  if (!here || !canAdd(doc)) return null;
  const copy: Frame = { ...here, colors: [...here.colors] };
  const copied = [...doc.frames.slice(0, at + 1), copy, ...doc.frames.slice(at + 1)];
  // Copied at the end: the copy is last and keeps the loop; the original now passes to it as a cut.
  const frames = at === doc.frames.length - 1 ? carryLoop(copied, here.transition, at) : copied;
  return { doc: { ...doc, frames }, selected: at + 1 };
}

/** Frame `at` removed. A pattern always keeps one frame. The selection moves to the frame that took its place. */
export function deleteFrame(doc: FrameDoc, at: number): { doc: FrameDoc; selected: number } | null {
  if (doc.frames.length <= 1 || at < 0 || at >= doc.frames.length) return null;
  const left = doc.frames.filter((_, i) => i !== at);
  // The last frame deleted: the one before it is last now and takes the loop connector.
  const frames = at === doc.frames.length - 1 ? carryLoop(left, doc.frames[at]?.transition, null) : left;
  return { doc: { ...doc, frames }, selected: Math.min(at, frames.length - 1) };
}

/** Frame `from` moved to place `to` (the others close up). Null when nothing moves. */
export function moveFrame(doc: FrameDoc, from: number, to: number): FrameDoc | null {
  const n = doc.frames.length;
  if (from === to || from < 0 || to < 0 || from >= n || to >= n) return null;
  const frames = [...doc.frames];
  const [moved] = frames.splice(from, 1);
  frames.splice(to, 0, moved as Frame);
  // Into or out of the last place: the loop connector stays with the place, and the frame that left it passes on as a cut.
  const last = n - 1;
  if (from !== last && to !== last) return { ...doc, frames };
  const demoted = from === last ? to : frames.indexOf(doc.frames[last] as Frame);
  return { ...doc, frames: carryLoop(frames, doc.frames[last]?.transition, demoted) };
}

/** The frame order after `from` goes to place `to`, as the indices each place now holds (for the selection to follow). */
export function selectedAfterMove(selected: number, from: number, to: number): number {
  if (selected === from) return to;
  if (from < selected && to >= selected) return selected - 1;
  if (from > selected && to <= selected) return selected + 1;
  return selected;
}

/** Frame `at` holds for `ms` milliseconds (whole, 0 to the spine's cap). */
export function setHold(doc: FrameDoc, at: number, ms: number): FrameDoc {
  const here = doc.frames[at];
  const hold = wholeMs(ms, HOLD_MS_MAX);
  if (!here || here.hold === hold) return doc;
  return { ...doc, frames: doc.frames.map((f, i) => (i === at ? { ...f, hold } : f)) };
}

/**
 * Frame `at` passes to the next as `patch` says. A Cut with nothing else remembered writes no
 * transition at all (the 0.16.0 shape, which reads as a cut); once a fade has been set its length
 * and pace are kept when the owner flips back to Cut, so Fade comes back the way it was.
 */
export function setTransition(doc: FrameDoc, at: number, patch: Partial<Transition>): FrameDoc {
  const here = doc.frames[at];
  if (!here) return doc;
  const was: Transition = here.transition ?? { kind: "cut", ms: NEW_FADE_MS, easing: "linear" };
  const next: Transition = {
    kind: patch.kind ?? was.kind,
    ms: patch.ms === undefined ? was.ms : wholeMs(patch.ms, TRANSITION_MS_MAX),
    easing: (patch.easing ?? was.easing) as Easing,
  };
  const frame: Frame = { ...here };
  if (next.kind === "cut" && here.transition === undefined) delete frame.transition;
  else frame.transition = next;
  if (here.transition !== undefined && JSON.stringify(here.transition) === JSON.stringify(frame.transition)) return doc;
  if (here.transition === undefined && frame.transition === undefined) return doc;
  return { ...doc, frames: doc.frames.map((f, i) => (i === at ? frame : f)) };
}

/** Loop on or off. Looping is the default, so it is written as no `playback` at all (the shape a 0.16.0 file has). */
export function setLoop(doc: FrameDoc, loop: boolean): FrameDoc {
  const looping = doc.playback?.loop !== false;
  if (looping === loop) return doc;
  const next: FrameDoc = { ...doc };
  if (loop) delete next.playback;
  else next.playback = { ...(doc.playback ?? {}), loop: false };
  return next;
}

/**
 * The fps shortcut: every frame holds `round(1000 / fps)` ms and every transition is a cut. There
 * is no stored fps: the holds are the one truth, and [`fpsOf`] reads the number back from them.
 */
export function applyFps(doc: FrameDoc, fps: number): FrameDoc {
  const clamped = Math.min(FPS_MAX, Math.max(FPS_MIN, Number.isFinite(fps) ? fps : FPS_MIN));
  const hold = Math.round(1000 / clamped);
  const frames = doc.frames.map((f) => {
    const frame: Frame = { ...f, hold };
    delete frame.transition;
    return frame;
  });
  return { ...doc, frames };
}

/**
 * The fps the frames come to when they all hold the same time and every transition is a cut (so
 * `applyFps` made them, or an owner typed alike); null when they differ.
 */
export function fpsOf(doc: FrameDoc): number | null {
  const first = doc.frames[0];
  if (!first || first.hold <= 0) return null;
  for (const f of doc.frames) {
    if (f.hold !== first.hold) return null;
    if (f.transition !== undefined && f.transition.kind !== "cut") return null;
  }
  const fps = 1000 / first.hold;
  return fps >= FPS_MIN && fps <= FPS_MAX + 0.5 ? Math.round(fps) : null;
}

/** The frames and Loop of a pattern. */
export function docOf(pattern: Pick<Pattern, "frames" | "playback">): FrameDoc {
  return pattern.playback === undefined ? { frames: pattern.frames } : { frames: pattern.frames, playback: pattern.playback };
}

/** The pattern with this document's frames and Loop in place of its own; every other field is spread as it came. */
export function withDoc(pattern: Pattern, doc: FrameDoc): Pattern {
  const { playback: _old, ...rest } = pattern;
  return doc.playback === undefined ? { ...rest, frames: doc.frames } : { ...rest, frames: doc.frames, playback: doc.playback };
}
