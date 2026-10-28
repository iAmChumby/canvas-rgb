// Colour values that are data, not chrome. Components never hold a hex literal; a default that
// has to be a hex string lives here.

/** The value a colour control shows when nothing is set (black). */
export const BLACK_HEX = "#000000";

/** True for a `#rrggbb` string, the form the daemon sends for every Rgb8 value. */
export function isHex(value: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(value);
}

/**
 * Reads a colour the way a person types it: `#rgb`, `#rrggbb`, `rgb` or `rrggbb`, in either
 * case. Returns `#rrggbb` in lower case, or null when the text is not a colour.
 */
export function parseHex(input: string): string | null {
  const raw = input.trim().replace(/^#/, "");
  if (!/^[0-9a-fA-F]{3}$|^[0-9a-fA-F]{6}$/.test(raw)) return null;
  const full =
    raw.length === 3
      ? raw
          .split("")
          .map((c) => c + c)
          .join("")
      : raw;
  return `#${full.toLowerCase()}`;
}

/** A `#rrggbb` colour as its three channels, 0 to 255. */
export function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const full = parseHex(hex) ?? BLACK_HEX;
  return {
    r: parseInt(full.slice(1, 3), 16),
    g: parseInt(full.slice(3, 5), 16),
    b: parseInt(full.slice(5, 7), 16),
  };
}

/** A colour as hue (0-360), saturation (0-1) and value (0-1). */
export interface Hsv {
  h: number;
  s: number;
  v: number;
}

/** A colour as hue, saturation and value. A grey keeps the hue it was given. */
export function hexToHsv(hex: string, keepHue = 0): Hsv {
  const { r, g, b } = hexToRgb(hex);
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  let h = keepHue;
  if (d > 0) {
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h = ((h * 60) + 360) % 360;
  }
  return { h, s: max === 0 ? 0 : d / max, v: max / 255 };
}

/** Hue, saturation and value back to a `#rrggbb` colour. */
export function hsvToHex({ h, s, v }: Hsv): string {
  const hue = ((h % 360) + 360) % 360;
  const c = v * s;
  const x = c * (1 - Math.abs(((hue / 60) % 2) - 1));
  const m = v - c;
  const seg = Math.floor(hue / 60) % 6;
  const rgb = [
    [c, x, 0],
    [x, c, 0],
    [0, c, x],
    [0, x, c],
    [x, 0, c],
    [c, 0, x],
  ][seg] ?? [0, 0, 0];
  const hex = (n: number) =>
    Math.max(0, Math.min(255, Math.round((n + m) * 255)))
      .toString(16)
      .padStart(2, "0");
  return `#${hex(rgb[0] ?? 0)}${hex(rgb[1] ?? 0)}${hex(rgb[2] ?? 0)}`;
}
