// The White effect's colour, for the demo: the same curve as the daemon's
// `canvas_proto::color::kelvin_to_rgb` (Tanner Helland's blackbody fit, clamped to
// 1800-6500 K, blue never under 12 %) and `kelvin_to_led` (decoded to linear light, #50),
// so a demo stack with a White layer shows the bytes the lights are sent.

export const KELVIN_MIN = 1800;
export const KELVIN_MAX = 6500;
const BLUE_FLOOR = 0.12;

/** A white of colour temperature `kelvin`, as 8-bit display (sRGB-encoded) RGB. */
export function kelvinToRgb(kelvin: number): [number, number, number] {
  return kelvinUnit(kelvin).map((v) => Math.round(v * 255)) as [number, number, number];
}

/** One sRGB-encoded channel, 0..1, to linear light (IEC 61966-2-1). */
export function srgbToLinear(v: number): number {
  const c = Math.min(1, Math.max(0, v));
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

/**
 * The white of colour temperature `kelvin` as the LED drive levels the White effect sends,
 * 8-bit: the display white decoded to linear light. 1800 K is `#ff3603`.
 */
export function kelvinToLed(kelvin: number): [number, number, number] {
  return kelvinUnit(kelvin).map((v) => Math.round(srgbToLinear(v) * 255)) as [number, number, number];
}

function kelvinUnit(kelvin: number): [number, number, number] {
  const k = Number.isFinite(kelvin) ? Math.min(KELVIN_MAX, Math.max(KELVIN_MIN, kelvin)) : KELVIN_MIN;
  const t = k / 100;
  const red = t <= 66 ? 255 : 329.698727446 * Math.pow(t - 60, -0.1332047592);
  const green = t <= 66 ? 99.4708025861 * Math.log(t) - 161.1195681661 : 288.1221695283 * Math.pow(t - 60, -0.0755148492);
  const blue = t >= 66 ? 255 : t <= 19 ? 0 : 138.5177312231 * Math.log(t - 10) - 305.0447927307;
  const unit = (v: number) => Math.min(1, Math.max(0, v / 255));
  return [unit(red), unit(green), Math.max(unit(blue), BLUE_FLOOR)];
}
