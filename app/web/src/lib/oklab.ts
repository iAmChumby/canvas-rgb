// OKLab blending (#468): the one place the editor mixes two colours.
//
// Gradients and spin blend in OKLab (Björn Ottosson, 2020, https://bottosson.github.io/posts/oklab/),
// a colour space matched to the eye: red to blue passes through purple at an even lightness, where
// an RGB average goes dark and muddy. The daemon blends the same way when it spins a pattern
// (`crates/canvas-core/src/pattern_spin.rs`); both read `__tests__/spin.vectors.json`, so a preview
// and the lights agree to the 8-bit level.
//
// The arithmetic is written in the same order as the Rust (doubles on both sides): do not
// "simplify" it without regenerating the vectors from Rust.

import { hexToRgb } from "@/lib/color";

/** A colour as `[L, a, b]`. */
export type Lab = [number, number, number];

function toLinear(channel: number): number {
  const c = channel / 255;
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

function fromLinear(linear: number): number {
  const c = Math.min(1, Math.max(0, linear));
  const encoded = c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
  return Math.min(255, Math.max(0, Math.round(encoded * 255)));
}

/** A `#rrggbb` colour in OKLab. */
export function toOklab(hex: string): Lab {
  const { r: red, g: green, b: blue } = hexToRgb(hex);
  const r = toLinear(red);
  const g = toLinear(green);
  const b = toLinear(blue);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

/** An OKLab colour as `#rrggbb`, clamped into gamut. */
export function fromOklab([bigL, a, b]: Lab): string {
  const l1 = bigL + 0.3963377774 * a + 0.2158037573 * b;
  const m1 = bigL - 0.1055613458 * a - 0.0638541728 * b;
  const s1 = bigL - 0.0894841775 * a - 1.291485548 * b;
  const l = l1 * l1 * l1;
  const m = m1 * m1 * m1;
  const s = s1 * s1 * s1;
  const channels = [
    fromLinear(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    fromLinear(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    fromLinear(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];
  return `#${channels.map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

/**
 * `from` to `to` at `t` (0 is `from`, 1 is `to`), blended in OKLab. `t` outside 0..1 is clamped.
 * The ends and two alike colours are returned as they are, never through a round trip.
 */
export function mixOklab(from: string, to: string, t: number, fromLab?: Lab, toLab?: Lab): string {
  const f = from.toLowerCase();
  const u = to.toLowerCase();
  const at = Number.isFinite(t) ? Math.min(1, Math.max(0, t)) : 0;
  if (f === u || at <= 0) return f;
  if (at >= 1) return u;
  const a = fromLab ?? toOklab(f);
  const b = toLab ?? toOklab(u);
  return fromOklab([a[0] + (b[0] - a[0]) * at, a[1] + (b[1] - a[1]) * at, a[2] + (b[2] - a[2]) * at]);
}
