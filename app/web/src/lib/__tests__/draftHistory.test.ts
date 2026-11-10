// The stack builder's undo and redo (#257 phase 1): the pure history the builder keeps beside its
// draft. The rule that matters most is what counts as one step: a drag, a resize or a slider
// scrub is one press of undo, not one per frame.

import { describe, expect, it } from "vitest";
import {
  canRedo,
  canUndo,
  endRun,
  historyOf,
  MERGE_GAP_MS,
  redo,
  record,
  reset,
  undo,
  type History,
} from "@/lib/draftHistory";

/** A draft standing in for the stack: any small changing value does. */
type Draft = { value: number };
const at = (value: number): Draft => ({ value });
const value = (history: History<Draft>) => history.draft.value;

describe("the draft's history", () => {
  it("starts with no steps either way", () => {
    const history = historyOf(at(1));
    expect(canUndo(history)).toBe(false);
    expect(canRedo(history)).toBe(false);
    expect(value(undo(history))).toBe(1);
    expect(value(redo(history))).toBe(1);
  });

  it("takes one change back and forward again", () => {
    const history = record(historyOf(at(1)), at(2));
    expect(canUndo(history)).toBe(true);
    const back = undo(history);
    expect(value(back)).toBe(1);
    expect(canRedo(back)).toBe(true);
    expect(value(redo(back))).toBe(2);
  });

  it("folds every change of one held drag into a single step", () => {
    let history = historyOf(at(0));
    for (let n = 1; n <= 20; n++) history = record(history, at(n), { run: "area:l1:1", hold: true });
    expect(value(history)).toBe(20);
    expect(history.past).toHaveLength(1);
    const back = undo(history);
    expect(value(back)).toBe(0);
    expect(canRedo(back)).toBe(true);
  });

  it("keeps a held run open however long the drag pauses", () => {
    let history = record(historyOf(at(0)), at(1), { run: "area:l1:1", hold: true, now: 0 });
    history = record(history, at(2), { run: "area:l1:1", hold: true, now: MERGE_GAP_MS * 20 });
    expect(history.past).toHaveLength(1);
    expect(value(undo(history))).toBe(0);
  });

  it("ends a held run when its owner says the gesture is over", () => {
    let history = record(historyOf(at(0)), at(1), { run: "area:l1:1", hold: true });
    history = endRun(history);
    history = record(history, at(2), { run: "area:l1:1", hold: true });
    expect(history.past).toHaveLength(2);
    expect(value(undo(undo(history)))).toBe(0);
  });

  it("merges a scrub that keeps sending, and starts a new step after a pause", () => {
    let history = record(historyOf(at(0)), at(1), { run: "opacity:l1", now: 0 });
    history = record(history, at(2), { run: "opacity:l1", now: 300 });
    history = record(history, at(3), { run: "opacity:l1", now: 600 });
    expect(history.past).toHaveLength(1);
    history = record(history, at(4), { run: "opacity:l1", now: 600 + MERGE_GAP_MS + 1 });
    expect(history.past).toHaveLength(2);
  });

  it("does not merge two different runs", () => {
    let history = record(historyOf(at(0)), at(1), { run: "opacity:l1" });
    history = record(history, at(2), { run: "opacity:l2" });
    expect(history.past).toHaveLength(2);
  });

  it("gives a change with no run a step of its own", () => {
    let history = historyOf(at(0));
    history = record(history, at(1));
    history = record(history, at(2));
    expect(history.past).toHaveLength(2);
    expect(value(undo(undo(history)))).toBe(0);
  });

  it("drops the future when a new edit follows an undo", () => {
    const history = record(record(historyOf(at(0)), at(1)), at(2));
    const back = undo(history);
    expect(canRedo(back)).toBe(true);
    const branched = record(back, at(9));
    expect(canRedo(branched)).toBe(false);
    expect(value(redo(branched))).toBe(9);
  });

  it("ignores a change that changes nothing", () => {
    const draft = at(1);
    const history = record(historyOf(draft), draft, { run: "area:l1:1" });
    expect(canUndo(history)).toBe(false);
  });

  it("keeps at most a hundred steps", () => {
    let history = historyOf(at(0));
    for (let n = 1; n <= 130; n++) history = record(history, at(n));
    expect(history.past).toHaveLength(100);
    expect(history.past[0]?.value).toBe(30);
  });

  it("resets to a checkpoint with no steps behind it", () => {
    const history = record(record(historyOf(at(0)), at(1)), at(2));
    const fresh = reset(history, at(7));
    expect(value(fresh)).toBe(7);
    expect(canUndo(fresh)).toBe(false);
    expect(canRedo(fresh)).toBe(false);
  });
});
