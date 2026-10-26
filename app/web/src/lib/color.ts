// Colour values that are data, not chrome. Components never hold a hex literal; a default that
// has to be a hex string lives here.

/** The value a colour control shows when nothing is set (black). */
export const BLACK_HEX = "#000000";

/** True for a `#rrggbb` string, the form the daemon sends for every Rgb8 value. */
export function isHex(value: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(value);
}
