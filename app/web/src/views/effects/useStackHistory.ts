"use client";

// The builder's draft history (#257 phase 4): one draft and the steps that led to it, moved out of
// StackBuilder.tsx unchanged. Every edit is one step, so undo takes back a whole drag or slider
// scrub and not one frame of it; the draft and its history live in one piece of state, so a change
// and its step can never come apart.
//
// Each change also carries a version (#332). React commits state in its own time, and the builder's
// async edits read a mirror of the newest draft: the version is what lets that mirror tell a commit
// of an older change from a commit of a newer one, so a late commit cannot put it back. See
// `DraftMirror`.

import { useRef, useState } from "react";
import {
  canRedo,
  canUndo,
  historyOf,
  record,
  redo,
  reset,
  undo,
  type History,
} from "@/lib/draftHistory";
import type { Stack } from "@/proto/Stack";

/** The draft's history and the version of the change that produced it. */
interface Versioned {
  history: History<Stack>;
  version: number;
}

export function useStackHistory(initial: Stack, blocked: () => boolean) {
  const [state, setState] = useState<Versioned>(() => ({ history: historyOf(initial), version: 0 }));
  const edit = state.history;
  const draft = edit.draft;
  /** The newest version handed out: every queued change takes the next number. */
  const version = useRef(0);
  /** Undo, redo and resets so far: a mask that lands after one of these is not applied. */
  const historyMoves = useRef(0);
  /** Undo or redo was asked for: the next draft change may have taken the picked layer away. */
  const steppedRef = useRef(false);

  /** The version of the change just queued; a mirror of the draft reads it against its own. */
  const nextVersion = (): number => {
    version.current += 1;
    return version.current;
  };

  /** A change to the draft: a step of its own, or part of the undo run it names. */
  const setEdit = (update: (history: History<Stack>) => History<Stack>): number => {
    const at = nextVersion();
    setState((current) => ({ history: update(current.history), version: at }));
    return at;
  };

  const setDraft = (update: Stack | ((current: Stack) => Stack), run?: string, hold = false): number =>
    setEdit((history) =>
      record(
        history,
        typeof update === "function" ? update(history.draft) : update,
        run === undefined ? {} : { run, hold },
      ),
    );

  /** A checkpoint with no steps behind it: a load, a save or a revert. */
  const resetDraft = (next: Stack): number => {
    historyMoves.current += 1;
    return setEdit((history) => reset(history, next));
  };

  // Undo and redo go through the functional updater, so a change and a step asked for in the same
  // tick compose in order (a ref set in an effect would still hold the draft before the change).
  // A drag in progress finishes first: its gesture is one step, and a step taken in the middle
  // would tear it in two.
  const undoNow = () => {
    if (blocked()) return;
    steppedRef.current = true;
    historyMoves.current += 1;
    setEdit((history) => undo(history));
  };
  const redoNow = () => {
    if (blocked()) return;
    steppedRef.current = true;
    historyMoves.current += 1;
    setEdit((history) => redo(history));
  };

  return {
    edit,
    setEdit,
    draft,
    setDraft,
    resetDraft,
    /** The version of the draft on screen: what a mirror compares a commit against. */
    versionOf: state.version,
    historyMoves,
    steppedRef,
    undoNow,
    redoNow,
    canUndo: canUndo(edit),
    canRedo: canRedo(edit),
  };
}
