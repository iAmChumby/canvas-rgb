// The builder's draft mirror (#332): an async edit composes on `mirror.current`, and React's
// commits must not put an older draft back under it. Under load the builder's first commit (its
// empty draft) reached the mirror after a starter had already written its layers; the next queued
// starter then composed on an empty stack and the first starter's layers were gone. The reverse
// also happens: a load or a save resets the draft after an edit's write, and that reset is newer.

import { describe, expect, it } from "vitest";
import { DraftMirror } from "@/views/effects/draftMirror";
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

const empty = stack([]);
const sunset = stack(["Sunset", "Heat", "Sparkles"]);
const both = stack(["Sunset", "Heat", "Sparkles", "Warm white"]);

describe("DraftMirror", () => {
  it("keeps the mirror on a change a commit of an older one arrives after", () => {
    const mirror = new DraftMirror(empty, 1);
    // A starter's layers are written as change 2, and its state has not been committed yet.
    mirror.write(sunset, 2);
    // The builder's own first commit (change 1, the empty draft) lands late, as it did under load.
    mirror.commit(empty, 1);
    expect(mirror.current).toBe(sunset);
    // The next queued starter reads the layers the first one added.
    expect(mirror.current.layers.map((layer) => layer.name)).toEqual(["Sunset", "Heat", "Sparkles"]);
  });

  it("follows a load that resets the draft after an edit's write", () => {
    const mirror = new DraftMirror(empty, 1);
    mirror.write(sunset, 2);
    // The stack finished loading as change 3: it is newer than the edit, so it wins.
    mirror.commit(both, 3);
    expect(mirror.current).toBe(both);
  });

  it("follows the write's own commit", () => {
    const mirror = new DraftMirror(empty, 1);
    mirror.write(sunset, 2);
    mirror.commit(sunset, 2);
    expect(mirror.current).toBe(sunset);
    mirror.commit(both, 3);
    expect(mirror.current).toBe(both);
  });

  it("ignores a write older than what it holds", () => {
    const mirror = new DraftMirror(both, 5);
    mirror.write(sunset, 4);
    expect(mirror.current).toBe(both);
  });
});
