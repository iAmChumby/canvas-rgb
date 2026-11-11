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
 * Issue #297: a CPU hot limit at or below this sits in the part's idle range, so the closed-loop
 * "Don't exceed" hold is engaged almost all the time and runs the fans hard to keep it. The code's
 * own hot line is `HOT_TEMP_C` (50 °C): the fixed line every board sensor still uses, and the line
 * G4 used for every source before the limit was movable. A Ryzen 7000/9000 X3D idles within a few
 * degrees of it (the issue's 7800X3D sits at 48-53 °C), so the rule is that line plus the 10 °C a
 * real idle varies by: under 60 °C, warn.
 */
export const CPU_IDLE_LIMIT_C = HOT_TEMP_C + 10;

/** Whether a user-set hot limit lies in the CPU's idle range and will run the fans hard (#297). */
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
 * (issue #297): the warning carries the "the fans will run hard" half, so the note keeps the
 * height it had and the Fans right panel does not start to scroll (#109, #294).
 */
export function hotRuleShortText(limitC: number | undefined, ceilingC: number | undefined): string {
  const line = limitC === undefined ? "its hot limit" : `${Math.round(limitC)} °C`;
  const ceiling = ceilingC === undefined ? "its safety ceiling" : `${Math.round(ceilingC)} °C`;
  return `Above ${line} the fan speeds up; at ${ceiling} it runs at full speed.`;
}
