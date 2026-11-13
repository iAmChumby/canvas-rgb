// Rubber-band drag limits (issue #187, rule 3) and the settle back (rule 4).
//
// Past a limit a part does not stop dead: it follows the pointer with diminishing
// resistance, on iOS's resistance curve. An overshoot `o` (how far past the limit
// the pointer wants to go) in a span of dimension `d` (the limit's span) with
// constant `c` moves only
//
//   resisted = (o * c * d) / (d + c * o)
//
// A small overshoot tracks the pointer (resisted ≈ c*o); a large one levels off
// toward `d`, so the part can never be dragged arbitrarily far past its bound.
// Inside the limits the position is the pointer's, exactly: no easing while held
// (rule 4) — easing applies only to the settle after release, which is a plain
// clamp back to the limit (settleToLimit) under a CSS transition.

/** The resistance constant: how hard the part pushes back, from the first pixel. */
export const RUBBER_C = 0.55;

/** How far an overshoot of `overshoot` in a span of `dimension` actually moves. */
export function resistedOvershoot(overshoot: number, dimension: number, c: number = RUBBER_C): number {
  if (!Number.isFinite(overshoot) || !Number.isFinite(dimension) || overshoot <= 0 || dimension <= 0) return 0;
  return (overshoot * c * dimension) / (dimension + c * overshoot);
}

/**
 * The pointer's position, resisted past either limit. Inside [min, max] this is
 * the value itself — the held drag follows the pointer exactly.
 */
export function rubberBand(value: number, min: number, max: number, c: number = RUBBER_C): number {
  const span = max - min;
  if (!Number.isFinite(value) || !Number.isFinite(span) || span <= 0) return value;
  if (value < min) return min - resistedOvershoot(min - value, span, c);
  if (value > max) return max + resistedOvershoot(value - max, span, c);
  return value;
}

/** The release: back to the limit itself, in bounds or not yet past the limit. */
export function settleToLimit(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
