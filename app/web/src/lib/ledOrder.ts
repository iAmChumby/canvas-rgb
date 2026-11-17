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

/**
 * The physical LED that is position 0 now. A first LED the owner set (and that still fits the
 * light) is it. With none, a forward run starts on LED 0 and a reversed one on the last LED: the
 * flip every reversed light made before #468, so the notch is drawn where the lights really start.
 */
export function effectiveFirstLed(
  firstLed: number | null | undefined,
  reversed: boolean,
  count: number,
): number {
  if (count <= 0) return 0;
  if (firstLed != null && firstLed >= 0 && firstLed < count) return firstLed;
  return reversed ? count - 1 : 0;
}

/**
 * The patch that turns a light round. The first LED stays first and `reversed` sets the way the
 * run goes from it, so the first LED in force is sent with the flag: without it a light with no
 * first LED would jump from LED 0 to its last LED, or back, as the direction changed.
 */
export function directionPatch(
  next: boolean,
  firstLed: number | null | undefined,
  reversed: boolean,
  count: number,
): { reversed: boolean; first_led: number } {
  return { reversed: next, first_led: effectiveFirstLed(firstLed, reversed, count) };
}

/**
 * The LED the identify route flashes for a light's first LED. Identify addresses a LED of the
 * zone (`Run.led`, `crates/canvas-daemon/src/identify.rs`), but `first_led` counts inside the
 * light, which for a chain part or a placed slice starts at `leds.start` of the zone. Position 0
 * is the first LED in either direction, so `reversed` changes nothing here.
 */
export function identifyLed(slice: { start: number } | null | undefined, firstLed: number): number {
  return (slice?.start ?? 0) + firstLed;
}

/**
 * Where a pointer is in the SVG's own coordinates. The view is `viewBox` w x h inside a box of any
 * shape, and SVG's default `preserveAspectRatio` (xMidYMid meet) scales it uniformly and centres
 * it, so a box that is not the view's shape leaves bands the picture does not cover. A straight
 * px-to-viewBox ratio ignores those bands and picks the wrong LED.
 */
export function clientToView(
  rect: { left: number; top: number; width: number; height: number },
  w: number,
  h: number,
  clientX: number,
  clientY: number,
): { x: number; y: number } {
  if (rect.width <= 0 || rect.height <= 0 || w <= 0 || h <= 0) return { x: 0, y: 0 };
  const scale = Math.min(rect.width / w, rect.height / h);
  const offsetX = (rect.width - w * scale) / 2;
  const offsetY = (rect.height - h * scale) / 2;
  return {
    x: (clientX - rect.left - offsetX) / scale,
    y: (clientY - rect.top - offsetY) / scale,
  };
}
