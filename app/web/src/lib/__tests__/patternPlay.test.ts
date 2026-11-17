// Frames in time in the editor, against the vectors the daemon's Rust makes (#468).
//
// `pattern.vectors.json` is written by crates/canvas-core/tests/pattern_vectors.rs from the daemon's
// own engine (the hold, cut and still cases by hand); this file reads the same JSON with the
// editor's TypeScript, so the preview and the lights cannot drift.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { beatAt, cycleMs, ease, evalPattern, frameLabs, isStill, timelineOf } from "@/lib/patternPlay";
import { mixOklab } from "@/lib/oklab";
import type { Easing } from "@/proto/Easing";
import type { Pattern } from "@/proto/Pattern";

interface Vectors {
  ease: { easing: Easing; u: number; expect: number }[];
  cases: { name: string; pattern: Pattern; at: { t: number; expect: string[] }[] }[];
}

const vectors = JSON.parse(readFileSync(join(__dirname, "pattern.vectors.json"), "utf8")) as Vectors;

describe("easing", () => {
  it("is exactly what the Rust makes, for every easing and every u", () => {
    expect(vectors.ease.length).toBe(36);
    for (const c of vectors.ease) expect(ease(c.easing, c.u), `${c.easing} at ${c.u}`).toBe(c.expect);
  });

  it("maps the ends exactly and clamps outside them", () => {
    for (const e of ["linear", "ease_in", "ease_out", "ease_in_out"] as Easing[]) {
      expect(ease(e, 0)).toBe(0);
      expect(ease(e, 1)).toBe(1);
      expect(ease(e, -4)).toBe(0);
      expect(ease(e, 9)).toBe(1);
      expect(ease(e, Number.NaN)).toBe(0);
    }
  });
});

describe("frames in time", () => {
  it("plays every vector exactly as the Rust does", () => {
    expect(vectors.cases.length).toBeGreaterThanOrEqual(10);
    for (const c of vectors.cases) {
      const spin = (c.pattern.motion as { turns_per_s?: number }).turns_per_s ?? null;
      const timeline = timelineOf(c.pattern);
      const labs = frameLabs(c.pattern.frames);
      for (const at of c.at) {
        expect(evalPattern(c.pattern, at.t, spin, timeline, labs), `${c.name} at ${at.t} ms`).toEqual(at.expect);
        // The cache changes nothing.
        expect(evalPattern(c.pattern, at.t, spin), `${c.name} at ${at.t} ms (no cache)`).toEqual(at.expect);
      }
    }
  });

  it("takes the fade's midpoint in OKLab, not RGB", () => {
    const fade = vectors.cases.find((c) => c.name.startsWith("fade linear"))!;
    const mid = evalPattern(fade.pattern, 150);
    expect(mid[0]).toBe(mixOklab("#ff0000", "#00ff00", 0.5));
    expect(mid[0]).not.toBe("#808000");
  });

  it("has no seam: the cycle's end is frame 0 and just before it is almost frame 0", () => {
    const seam = vectors.cases.find((c) => c.name.startsWith("last frame fades"))!;
    const cycle = cycleMs(seam.pattern);
    expect(cycle).toBe(300);
    const before = evalPattern(seam.pattern, cycle - 1e-6);
    const after = evalPattern(seam.pattern, cycle);
    expect(after).toEqual(seam.pattern.frames[0]!.colors);
    expect(before).toEqual(after);
  });

  it("is still for one frame and for frames that take no time", () => {
    for (const c of vectors.cases) {
      const t = timelineOf(c.pattern);
      if (c.name.startsWith("one frame") || c.name.startsWith("frames that take no time")) {
        expect(isStill(t), c.name).toBe(true);
        expect(beatAt(t, 12345)).toEqual({ kind: "hold", frame: 0 });
      }
    }
  });

  it("stops on the last frame when the pattern does not loop", () => {
    const c = vectors.cases.find((x) => x.name.startsWith("not looping"))!;
    expect(beatAt(timelineOf(c.pattern), 1e9)).toEqual({ kind: "hold", frame: 1 });
    expect(cycleMs(c.pattern)).toBe(300);
  });
});
