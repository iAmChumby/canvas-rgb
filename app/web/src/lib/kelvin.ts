// The White effect's colour, for the demo: the same curve as the daemon's
// `canvas_effects::color::kelvin_to_rgb` (Tanner Helland's blackbody fit, clamped to
// 1800-6500 K, blue never under 12 %), so a demo stack with a White layer shows the
// white the lights would.

export const KELVIN_MIN = 1800;
export const KELVIN_MAX = 6500;
const BLUE_FLOOR = 0.12;

/** A white of colour temperature `kelvin`, as 8-bit RGB. */
export function kelvinToRgb(kelvin: number): [number, number, number] {
  const k = Number.isFinite(kelvin) ? Math.min(KELVIN_MAX, Math.max(KELVIN_MIN, kelvin)) : KELVIN_MIN;
  const t = k / 100;
  const red = t <= 66 ? 255 : 329.698727446 * Math.pow(t - 60, -0.1332047592);
  const green = t <= 66 ? 99.4708025861 * Math.log(t) - 161.1195681661 : 288.1221695283 * Math.pow(t - 60, -0.0755148492);
  const blue = t >= 66 ? 255 : t <= 19 ? 0 : 138.5177312231 * Math.log(t - 10) - 305.0447927307;
  const unit = (v: number) => Math.min(1, Math.max(0, v / 255));
  const byte = (v: number) => Math.round(v * 255);
  return [byte(unit(red)), byte(unit(green)), byte(Math.max(unit(blue), BLUE_FLOOR))];
}
