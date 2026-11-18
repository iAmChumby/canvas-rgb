// The Gradient paint tool's maths (#468): stops placed around a ring (or along a line), turned into
// one colour per LED. The editor writes the colours into the frame; the stops are editor state and
// nothing about them is stored.
//
// A stop sits at a position `at` in 0..1. On a ring the LEDs are `i / n` (the circle's LED 0 at the
// top, and LED `n` would be LED 0 again), and with **Wrap** the last stop meets the first across the
// seam: the colours after the last stop run back to the first stop's, so a ring that is then spun
// shows no seam. A line puts its LEDs at `i / (n - 1)`, first LED at 0, last at 1, and the colours
// before the first stop and after the last stay as those stops. Blending is in OKLab, the same
// helper the daemon's Spin uses (`lib/oklab.ts`).

import { BLACK_HEX, hsvToHex } from "@/lib/color";
import { mixOklab } from "@/lib/oklab";

export interface GradientStop {
  /** Where the stop sits, 0 up to (not including) 1 on a wrapped ring; 0..1 on a line. */
  at: number;
  /** `#rrggbb`. */
  colour: string;
}

/** How many stops a gradient starts with. */
export const SEED_STOPS = 3;
/** The fewest stops a gradient keeps: one is a flat colour, which Fill all already does. */
export const MIN_STOPS = 2;
/** The most stops: a stop per ~12 LEDs of the smallest fan would already be crowded. */
export const MAX_STOPS = 8;

/**
 * Three stops spaced evenly round the wheel, starting from `hue` (degrees): the hue wheel is the
 * seed, so a new gradient is a rainbow that starts where the colour in hand starts.
 */
export function seedStops(hue: number, wrap: boolean): GradientStop[] {
  const count = SEED_STOPS;
  return Array.from({ length: count }, (_, k) => ({
    // A wrapped ring spaces them by thirds; a line puts the first at the start and the last at the end.
    at: wrap ? k / count : k / (count - 1),
    colour: hsvToHex({ h: (hue + (360 * k) / count) % 360, s: 0.85, v: 1 }),
  }));
}

/**
 * Stops describing colours already painted on the device (#481): one stop per LED while there are
 * few enough, otherwise an even sample of at most `MAX_STOPS`. On a device whose every LED has its
 * own stop, applying the stops gives the same colours back; a larger device is only sampled, so the
 * editor must not write the sampled blend back until the owner edits a stop.
 */
export function stopsFromColours(colours: readonly string[], wrap: boolean): GradientStop[] {
  const n = colours.length;
  if (n === 0) return seedStops(0, wrap);
  const take = Math.min(n, MAX_STOPS);
  const stops: GradientStop[] = [];
  for (let k = 0; k < take; k++) {
    const i = take === n ? k : Math.round((k * (n - 1)) / (take - 1));
    stops.push({ at: ledPosition(i, n, wrap), colour: colours[i] ?? BLACK_HEX });
  }
  // A gradient keeps at least two stops; a one-LED device gets a flat pair.
  if (stops.length === 1) stops.push({ at: wrap ? 0.5 : 1, colour: stops[0]!.colour });
  return stops;
}

/** Where LED `index` of `leds` sits on 0..1 (see the header): `i / n` when wrapped, `i / (n - 1)` on a line. */
export function ledPosition(index: number, leds: number, wrap: boolean): number {
  if (leds <= 1) return 0;
  return wrap ? index / leds : index / (leds - 1);
}

/** The LED nearest a position: the inverse of [`ledPosition`], for dragging a stop to a dot. */
export function ledNear(at: number, leds: number, wrap: boolean): number {
  if (leds <= 1) return 0;
  const raw = Math.round(at * (wrap ? leds : leds - 1));
  return wrap ? ((raw % leds) + leds) % leds : Math.min(leds - 1, Math.max(0, raw));
}

/** The colour at a position, from stops in any order. No stops is black. */
export function colourAt(stops: readonly GradientStop[], at: number, wrap: boolean): string {
  if (stops.length === 0) return "#000000";
  const sorted = [...stops].sort((a, b) => a.at - b.at);
  const first = sorted[0] as GradientStop;
  const last = sorted[sorted.length - 1] as GradientStop;
  if (sorted.length === 1) return first.colour.toLowerCase();
  let before: { at: number; colour: string } | undefined;
  let after: { at: number; colour: string } | undefined;
  for (const stop of sorted) {
    if (stop.at <= at) before = stop;
    else if (after === undefined) after = stop;
  }
  if (wrap) {
    // The last stop meets the first across the seam, one lap on.
    before ??= { at: last.at - 1, colour: last.colour };
    after ??= { at: first.at + 1, colour: first.colour };
  } else {
    before ??= first;
    after ??= last;
  }
  const span = after.at - before.at;
  if (!(span > 0)) return before.colour.toLowerCase();
  return mixOklab(before.colour, after.colour, (at - before.at) / span);
}

/** One colour per LED. */
export function gradientColours(stops: readonly GradientStop[], leds: number, wrap: boolean): string[] {
  return Array.from({ length: Math.max(0, leds) }, (_, i) => colourAt(stops, ledPosition(i, leds, wrap), wrap));
}
