// The editor's timingKey against the engine's PatternTiming (#518).
//
// timingKey.vectors.json is written by crates/canvas-core/examples/timing_key_vectors.rs from the
// daemon's own timing rule; the Rust test fails when the file is stale. This file runs timingKey
// over the same pairs, so the clock a colour-only stroke keeps in the editor is the clock the engine
// keeps on the lights.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { timingKey } from "@/lib/patternDraft";
import type { Pattern } from "@/proto/Pattern";

interface Pair {
  name: string;
  a: Pattern;
  b: Pattern;
  timing_a: Record<string, unknown>;
  timing_b: Record<string, unknown>;
  same_timing: boolean;
}

interface Vectors {
  note: string;
  pairs: Pair[];
}

const vectors = JSON.parse(readFileSync(join(__dirname, "timingKey.vectors.json"), "utf8")) as Vectors;

/** The PatternTiming fields this pair changes, by comparing the engine's own timing objects. */
function changedFields(pair: Pair): string[] {
  return Object.keys(pair.timing_a).filter(
    (field) => JSON.stringify(pair.timing_a[field]) !== JSON.stringify(pair.timing_b[field]),
  );
}

describe("timingKey against the engine's PatternTiming", () => {
  it("gives the daemon's verdict on every pair", () => {
    expect(vectors.pairs.length).toBeGreaterThanOrEqual(13);
    for (const pair of vectors.pairs) {
      expect(timingKey(pair.a) === timingKey(pair.b), `${pair.name}: the editor's verdict`).toBe(pair.same_timing);
    }
  });

  it("changes each timing field on its own, and no field goes untested", () => {
    const fields = Object.keys(vectors.pairs[0]!.timing_a).sort();
    // A field added to PatternTiming changes this list: the vectors must gain a pair for it.
    expect(fields).toEqual(["frames", "id", "motion", "playback"]);
    for (const pair of vectors.pairs) {
      const changed = changedFields(pair);
      if (pair.same_timing) {
        expect(changed, `${pair.name}: a same-timing pair changes no field`).toEqual([]);
      } else {
        expect(changed, `${pair.name}: one field at a time`).toHaveLength(1);
      }
    }
    for (const field of fields) {
      expect(
        vectors.pairs.some((pair) => changedFields(pair).join() === field),
        `${field} is changed by at least one pair`,
      ).toBe(true);
    }
  });
});
