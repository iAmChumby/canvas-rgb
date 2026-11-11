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

/**
 * Issue #297: the fixed threshold under which the editor warns that a CPU line keeps the
 * closed-loop "Don't exceed" hold engaged, so the fans run hard to hold it. It is a fixed rule and
 * not a reading of the part's idle temperature: `HOT_TEMP_C` (50 °C, the code's own hot line, still
 * the automatic line for every board sensor) plus 10 °C. The warning says what the rule is, never
 * that it knows where this CPU idles.
 */
export const CPU_IDLE_LIMIT_C = HOT_TEMP_C + 10;

/** Whether a user-set CPU hot limit is at or under the warning threshold (#297). */
export function cpuLimitInIdleRange(kind: string, limitC: number): boolean {
  return kind === "cpu" && limitC <= CPU_IDLE_LIMIT_C;
}

/**
 * G4's rule and issue #255's closed loop, with the numbers filled in. Issue #130: the line is
 * the source's hot limit now. Above the line the header speeds up until the source is back under
 * it, and at the fixed safety ceiling it runs at full speed. The source is the short name the
 * daemon's own reason sentence uses, so the page and the status agree.
 */
export function hotRuleText(limitC: number | undefined, source: string, ceilingC: number | undefined): string {
  // The service rounds the limit the same way before putting it in the status reason, so the
  // sentence and the reason always name the same number.
  const line = limitC === undefined ? "its hot limit" : `${Math.round(limitC)} °C`;
  const ceiling = ceilingC === undefined ? "its safety ceiling" : `${Math.round(ceilingC)} °C`;
  return `Above ${line} the fan speeds up to bring ${source} back under it; at ${ceiling} it runs at full speed.`;
}

/**
 * The same sentence cut to one line, for when the idle-range warning is drawn under the control
 * (issue #297): the warning carries the "the fans run hard" half, so the note gives back a line
 * and the panel adds the warning's one line and no more (#109, #294, #299).
 */
export function hotRuleShortText(limitC: number | undefined, ceilingC: number | undefined): string {
  const line = limitC === undefined ? "its hot limit" : `${Math.round(limitC)} °C`;
  const ceiling = ceilingC === undefined ? "its safety ceiling" : `${Math.round(ceilingC)} °C`;
  return `Above ${line} the fan speeds up; at ${ceiling} it runs at full speed.`;
}
