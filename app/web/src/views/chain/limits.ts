// The limits of the chain setup that the daemon enforces and the screens state.

/**
 * The most times one run of LEDs may be shown by a header (`Light::copies`). A typo guard, not a
 * hardware limit. It must equal `COPIES_MAX` in `crates/canvas-proto/src/scene.rs`: a test reads
 * the Rust file and compares.
 */
export const COPIES_MAX = 32;

/**
 * How many times a header shows a light: 2 or more for a header whose devices mirror each other
 * (the light is one run of LEDs shown that many times), otherwise 1.
 */
export function shownTimes(light: { light: { copies?: number | null } }): number {
  const n = light.light.copies;
  return typeof n === "number" && n >= 2 ? n : 1;
}

/** "shown 8 times" for a mirrored light, "" for one that is shown once. */
export function shownWords(light: { light: { copies?: number | null } }): string {
  const n = shownTimes(light);
  return n >= 2 ? `shown ${n} times` : "";
}
