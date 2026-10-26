// Fan floors as the daemon applies them. Mirrors crates/canvas-proto/src/fans.rs
// (DEFAULT_FLOOR_PERCENT, CPU_FAN_MIN_PERCENT, HOT_*). The API is still the authority: a value
// the API refuses comes back as a 422 and is shown inline.

export const CPU_FAN = "cpu_fan";
export const DEFAULT_FLOOR = 20;
export const CPU_FAN_MIN = 30;
export const HOT_TEMP_C = 50;
export const HOT_FLOOR = 35;
export const HOT_CPU_FAN_FLOOR = 50;

/** The lowest percent a header can be set to. */
export function baseFloor(header: string, configured: number): number {
  const floor = Math.max(configured, DEFAULT_FLOOR);
  return header === CPU_FAN ? Math.max(floor, CPU_FAN_MIN) : floor;
}

/** The floor a manual header is raised to while its curve temperature is at or above 50 C. */
export function hotFloor(header: string, configured: number): number {
  return Math.max(baseFloor(header, configured), header === CPU_FAN ? HOT_CPU_FAN_FLOOR : HOT_FLOOR);
}

/** G4's rule, with the numbers filled in. */
export function hotRuleText(): string {
  return `Below ${HOT_FLOOR}% (CPU_FAN ${HOT_CPU_FAN_FLOOR}%) is raised while the curve's temperature is ≥ ${HOT_TEMP_C} °C`;
}
