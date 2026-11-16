// The versions the stack builder's history hands out, read by its draft mirror (#332).
//
// The builder's bug needed a loaded machine: a commit of an older draft reached the mirror after an
// edit had written a newer one. Here that order is made by hand, with the real hook: the mirror is
// given the stale commit after the write, with no wait and no clock, and must keep the write. The
// same hook must also hand undo, redo and a reset a newer version, or the mirror would refuse a
// draft it has to follow.

import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { DraftMirror } from "@/views/effects/draftMirror";
import { useStackHistory } from "@/views/effects/useStackHistory";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";

function stack(layers: string[]): Stack {
  return {
    id: "stack:",
    layers: layers.map(
      (name, index) => ({ id: `l${index + 1}`, name, effect: "solid", props: {} }) as StackLayer,
    ),
  };
}

const names = (draft: Stack) => draft.layers.map((layer) => layer.name);
const empty = stack([]);
const sunset = stack(["Sunset", "Heat", "Sparkles"]);
const both = stack(["Sunset", "Heat", "Sparkles", "Warm white"]);

function builder() {
  const hook = renderHook(() => useStackHistory(empty, () => false));
  const first = { draft: hook.result.current.draft, version: hook.result.current.versionOf };
  const mirror = new DraftMirror(first.draft, first.version);
  /** What the builder's effect does after a commit. */
  const commit = () => mirror.commit(hook.result.current.draft, hook.result.current.versionOf);
  return { hook, first, mirror, commit };
}

describe("the stack history's versions and the draft mirror", () => {
  it("keeps a starter's layers when the builder's first commit arrives after them", () => {
    const { hook, first, mirror, commit } = builder();
    let at = 0;
    act(() => {
      at = hook.result.current.setDraft(sunset);
      mirror.write(sunset, at);
    });
    expect(at).toBeGreaterThan(first.version);
    // The late commit of the empty draft the builder mounted with: the order the loaded run had.
    mirror.commit(first.draft, first.version);
    expect(names(mirror.current)).toEqual(["Sunset", "Heat", "Sparkles"]);
    // The write's own commit carries the write's version.
    expect(hook.result.current.versionOf).toBe(at);
    commit();
    expect(names(mirror.current)).toEqual(["Sunset", "Heat", "Sparkles"]);
  });

  it("gives undo, redo and a reset a newer version, so the mirror follows them", () => {
    const { hook, mirror, commit } = builder();
    let wrote = 0;
    act(() => {
      wrote = hook.result.current.setDraft(sunset);
      mirror.write(sunset, wrote);
    });
    commit();

    act(() => hook.result.current.undoNow());
    expect(hook.result.current.versionOf).toBeGreaterThan(wrote);
    commit();
    expect(names(mirror.current)).toEqual([]);

    const undone = hook.result.current.versionOf;
    act(() => hook.result.current.redoNow());
    expect(hook.result.current.versionOf).toBeGreaterThan(undone);
    commit();
    expect(names(mirror.current)).toEqual(["Sunset", "Heat", "Sparkles"]);

    const redone = hook.result.current.versionOf;
    let reset = 0;
    act(() => {
      reset = hook.result.current.resetDraft(both);
    });
    expect(reset).toBeGreaterThan(redone);
    commit();
    expect(names(mirror.current)).toEqual(["Sunset", "Heat", "Sparkles", "Warm white"]);
  });

  it("numbers two changes of one tick in the order they were asked for", () => {
    const { hook, mirror, commit } = builder();
    let one = 0;
    let two = 0;
    act(() => {
      one = hook.result.current.setDraft(sunset);
      two = hook.result.current.setDraft(both);
    });
    expect(two).toBeGreaterThan(one);
    expect(hook.result.current.versionOf).toBe(two);
    commit();
    expect(names(mirror.current)).toEqual(["Sunset", "Heat", "Sparkles", "Warm white"]);
  });
});
