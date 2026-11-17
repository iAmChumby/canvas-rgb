// Spin (#468): a pattern that turns along its LEDs, as the editor's preview draws it.
//
// The daemon plays a spinning pattern with `SpinSource::at` in `crates/canvas-core/src/pattern_spin.rs`;
// this is the same function in TypeScript, checked against the same vectors
// (`__tests__/spin.vectors.json`). The turn is a pure function of the time: `phase = frac(turns/s * t)`,
// a shift of `phase * m` LEDs (`m` the LED count, so the speed is in laps and a ring of 8 and a ring
// of 40 turn alike), the fraction blended between neighbours in OKLab. A positive speed moves the
// picture towards higher LED numbers, a negative one back; rings and lines both wrap.

import { mixOklab, toOklab, type Lab } from "@/lib/oklab";
import type { Motion } from "@/proto/Motion";

/** `SPIN_TURNS_MAX` from the spine: the fastest a pattern spins, in laps a second, either way. */
export const SPIN_TURNS_MAX = 5;
/** `SPIN_TURNS_DEFAULT` from the spine: one lap every four seconds. */
export const SPIN_TURNS_DEFAULT = 0.25;

/** The part of a shift that is a whole LED: below this no blend is made. */
const WHOLE = 1e-9;

/** The speed of a spin motion in laps a second, or null for any other motion or a bad speed. */
export function spinOf(motion: Motion): number | null {
  if (motion.type !== "spin") return null;
  const speed = (motion as { turns_per_s?: unknown }).turns_per_s;
  if (typeof speed !== "number" || !Number.isFinite(speed) || Math.abs(speed) > SPIN_TURNS_MAX) return null;
  return speed;
}

/** A spin motion: `{"type":"spin","turns_per_s":0.25}`. A speed past the range is clamped, as the spine's is. */
export function spinMotion(turnsPerS: number): Motion {
  const speed = Number.isFinite(turnsPerS) ? Math.min(SPIN_TURNS_MAX, Math.max(-SPIN_TURNS_MAX, turnsPerS)) : 0;
  return { type: "spin", turns_per_s: speed } as Motion;
}

/** How far round a spin has turned `t` seconds in: 0 up to (never including) 1. */
export function spinPhase(turnsPerS: number, t: number): number {
  const turns = turnsPerS * t;
  if (!Number.isFinite(turns)) return 0;
  const phase = turns - Math.floor(turns);
  return phase >= 1 ? 0 : phase;
}

/**
 * The colours `t` seconds into a spin: `out[i]` is the source at `i - phase * m`, the fraction
 * blended in OKLab. `labs` (the OKLab of each colour) saves the conversion on every frame.
 */
export function spinColours(colours: readonly string[], turnsPerS: number, t: number, labs?: readonly Lab[]): string[] {
  const m = colours.length;
  if (m < 2) return [...colours];
  const shift = spinPhase(turnsPerS, t) * m;
  let whole = Math.min(Math.floor(shift), m - 1);
  const fraction = shift - whole;
  const blend = fraction >= WHOLE && fraction < 1 - WHOLE;
  if (fraction >= 1 - WHOLE) whole = (whole + 1) % m;
  return colours.map((_, i) => {
    const here = (i + m - whole) % m;
    const current = colours[here] ?? "#000000";
    if (!blend) return current.toLowerCase();
    const behind = (here + m - 1) % m;
    return mixOklab(current, colours[behind] ?? "#000000", fraction, labs?.[here], labs?.[behind]);
  });
}

/** The OKLab of every colour, for a preview that turns the same colours every frame. */
export function labsOf(colours: readonly string[]): Lab[] {
  return colours.map((c) => toOklab(c));
}
