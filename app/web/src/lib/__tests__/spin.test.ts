// OKLab and Spin in the editor, against the vectors the daemon's Rust makes (#468).
//
// `spin.vectors.json` is written by crates/canvas-core/tests/spin_vectors.rs from the daemon's own
// blend and spin; the `oklab` section is Ottosson's published values. This file reads the same JSON
// with the editor's TypeScript, so the preview and the lights cannot drift.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { fromOklab, mixOklab, toOklab } from "@/lib/oklab";
import { SPIN_TURNS_DEFAULT, SPIN_TURNS_MAX, spinColours, spinMotion, spinOf, spinPhase } from "@/lib/spin";

interface Vectors {
  oklab: { hex: string; lab: number[] }[];
  mix: { from: string; to: string; t: number; expect: string }[];
  spin: { name: string; colours: string[]; turns_per_s: number; t: number; expect: string[] }[];
}

const vectors = JSON.parse(readFileSync(join(__dirname, "spin.vectors.json"), "utf8")) as Vectors;

describe("OKLab", () => {
  it("matches Ottosson's published values", () => {
    for (const { hex, lab } of vectors.oklab) {
      const got = toOklab(hex);
      lab.forEach((want, i) => expect(Math.abs((got[i] as number) - want), `${hex} lab[${i}]`).toBeLessThan(2e-5));
    }
  });

  it("round-trips every colour on a lattice", () => {
    for (let r = 0; r <= 255; r += 15)
      for (let g = 0; g <= 255; g += 15)
        for (let b = 0; b <= 255; b += 15) {
          const hex = `#${[r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("")}`;
          expect(fromOklab(toOklab(hex))).toBe(hex);
        }
  });

  it("blends exactly as the Rust does, case by case", () => {
    expect(vectors.mix.length).toBeGreaterThan(5);
    for (const c of vectors.mix) expect(mixOklab(c.from, c.to, c.t), `${c.from} to ${c.to} at ${c.t}`).toBe(c.expect);
  });

  it("goes red to blue through purple, not RGB's (128, 0, 128)", () => {
    const mid = mixOklab("#ff0000", "#0000ff", 0.5);
    expect(mid).toBe("#8c53a2");
    expect(mid).not.toBe("#800080");
  });
});

describe("Spin", () => {
  it("plays every vector exactly as the Rust does", () => {
    expect(vectors.spin.length).toBeGreaterThan(8);
    for (const c of vectors.spin) expect(spinColours(c.colours, c.turns_per_s, c.t), c.name).toEqual(c.expect);
  });

  it("shifts a ring of 8 by two LEDs a second at 0.25 turns a second", () => {
    const ring = vectors.spin[0]!.colours;
    const shifted = spinColours(ring, 0.25, 1);
    expect(shifted).toEqual(ring.map((_, i) => ring[(i + 8 - 2) % 8]));
    expect(spinColours(ring, 0.25, 4)).toEqual(ring);
  });

  it("keeps its phase in range for any speed and time", () => {
    for (const turns of [-5, -0.25, 0, 0.25, 5])
      for (const t of [0, 1e-9, 0.3, 1, 17.77, 3600, 1e7]) {
        const p = spinPhase(turns, t);
        expect(p).toBeGreaterThanOrEqual(0);
        expect(p).toBeLessThan(1);
      }
    expect(spinPhase(Number.NaN, 1)).toBe(0);
  });

  it("writes and reads the spine's motion, clamping and refusing as the spine does", () => {
    expect(spinMotion(SPIN_TURNS_DEFAULT)).toEqual({ type: "spin", turns_per_s: 0.25 });
    expect(spinOf(spinMotion(0.25))).toBe(0.25);
    expect(spinOf(spinMotion(99))).toBe(SPIN_TURNS_MAX);
    expect(spinOf(spinMotion(-99))).toBe(-SPIN_TURNS_MAX);
    expect(spinOf({ type: "none" })).toBeNull();
    expect(spinOf({ type: "rotate", speed: 2 } as never)).toBeNull();
    expect(spinOf({ type: "spin", turns_per_s: "fast" } as never)).toBeNull();
    expect(spinOf({ type: "spin", turns_per_s: 6 } as never)).toBeNull();
  });
});
