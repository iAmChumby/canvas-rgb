// The maths of a light's LED order (#468, phase 3): which physical LED is position 0 and which
// way the run goes. Pure, so the notch's keys and its hit can be settled without a canvas.

import type { Dot } from "@/views/patterns/dotLayout";

/**
 * The key that moves the first LED: one LED per arrow press, wrapping round the run, and Home
 * resets to physical LED 0 (the way every light before #468 was). Null for any other key.
 */
export function stepFirstLed(current: number, key: string, count: number): number | null {
  if (count <= 0) return null;
  if (key === "Home") return 0;
  const delta =
    key === "ArrowRight" || key === "ArrowDown"
      ? 1
      : key === "ArrowLeft" || key === "ArrowUp"
        ? -1
        : 0;
  if (delta === 0) return null;
  return (((current + delta) % count) + count) % count;
}

/** The dot the notch sits on, or null when there are no LEDs. */
export function notchDot(dots: Dot[], firstLed: number): Dot | null {
  if (dots.length === 0) return null;
  return dots[((firstLed % dots.length) + dots.length) % dots.length] ?? null;
}

/** The nearest LED to a point, for dragging the notch. Null when the press is off every dot. */
export function nearestLed(dots: Dot[], x: number, y: number): number | null {
  let best: number | null = null;
  let distance = Infinity;
  for (const dot of dots) {
    const d = Math.hypot(dot.x - x, dot.y - y);
    if (d < distance) {
      distance = d;
      best = dot.index;
    }
  }
  return best;
}
