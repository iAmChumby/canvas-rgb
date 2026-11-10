// Undo and redo for the stack builder's draft (#257 phase 1). The builder owns one draft and the
// steps that led to it: `past` is what undo takes back, `future` is what redo takes forward.
//
// The one rule that needs more than a stack of values is what counts as one step. A drag, a
// resize or a slider scrub sends a change every frame, and undo has to put the whole gesture
// back in one press. A change may name its `run`: changes with the same run coalesce into one
// step. A run marked `hold` lasts until `endRun` (a pointer drag, which may sit still for any
// length of time); any other run also lasts while the changes keep arriving inside
// `MERGE_GAP_MS`, and a pause longer than that starts a new step (a fresh arrow-key nudge).
//
// Everything here is pure, so the builder's undo rules are tested without a DOM.

/** How long a run that is not held open may pause before the next change is a new step. */
export const MERGE_GAP_MS = 1_200;

/** How many steps undo can take back: an old step falls off the bottom of the list. */
export const HISTORY_MAX = 100;

export interface MergeState {
  /** The gesture the last change belonged to. */
  run: string;
  /** When that change landed, in ms. */
  at: number;
  /** The gesture ends only when its owner says so (a pointer drag), never by a pause. */
  hold: boolean;
}

export interface History<T> {
  draft: T;
  past: T[];
  future: T[];
  /** The run the last change was part of, or null when the last change was a step of its own. */
  merge: MergeState | null;
}

export interface RecordOptions {
  /** The gesture this change belongs to. Omit for a change that is a step of its own. */
  run?: string;
  /** The run lasts until `endRun`, however long the gesture pauses. */
  hold?: boolean;
  /** The clock, for tests and for a caller that already read it. */
  now?: number;
}

/** A draft with no steps behind it. */
export function historyOf<T>(draft: T): History<T> {
  return { draft, past: [], future: [], merge: null };
}

export function canUndo<T>(history: History<T>): boolean {
  return history.past.length > 0;
}

export function canRedo<T>(history: History<T>): boolean {
  return history.future.length > 0;
}

/**
 * One change. A change that belongs to the run the last change belonged to is folded into that
 * step; anything else pushes the draft it replaces onto the past and clears the future (a new
 * edit after an undo is the end of the branch that was undone).
 */
export function record<T>(history: History<T>, next: T, opts: RecordOptions = {}): History<T> {
  if (Object.is(next, history.draft)) return history;
  const { run, hold = false, now = Date.now() } = opts;
  const merge = history.merge;
  const continues =
    run !== undefined &&
    merge !== null &&
    merge.run === run &&
    (merge.hold || now - merge.at <= MERGE_GAP_MS);
  if (continues) {
    return { ...history, draft: next, merge: { run: run as string, at: now, hold: merge.hold || hold } };
  }
  return {
    draft: next,
    past: [...history.past, history.draft].slice(-HISTORY_MAX),
    future: [],
    merge: run === undefined ? null : { run, at: now, hold },
  };
}

/** The gesture is over: the next change, whatever run it names, starts a new step. */
export function endRun<T>(history: History<T>): History<T> {
  return history.merge === null ? history : { ...history, merge: null };
}

export function undo<T>(history: History<T>): History<T> {
  const prev = history.past[history.past.length - 1];
  if (prev === undefined) return history;
  return {
    draft: prev,
    past: history.past.slice(0, -1),
    future: [history.draft, ...history.future].slice(0, HISTORY_MAX),
    merge: null,
  };
}

export function redo<T>(history: History<T>): History<T> {
  const next = history.future[0];
  if (next === undefined) return history;
  return {
    draft: next,
    past: [...history.past, history.draft].slice(-HISTORY_MAX),
    future: history.future.slice(1),
    merge: null,
  };
}

/** A new draft with no steps behind it: a load, a save or a revert is a checkpoint. */
export function reset<T>(history: History<T>, next: T): History<T> {
  return Object.is(history.draft, next) && !history.merge && !history.past.length && !history.future.length
    ? history
    : historyOf(next);
}
