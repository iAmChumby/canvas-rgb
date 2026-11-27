// The error the editor shows. `status` 0 means the daemon did not answer at all.

export class ApiFailure extends Error {
  readonly status: number;
  readonly code: string;
  /**
   * The wait the daemon asked for with `Retry-After`, in milliseconds. Only the routes that
   * say how long to wait carry one (the effect preview while both render slots are busy).
   */
  readonly retryAfterMs?: number;
  /**
   * The next step in plain words (#576). The daemon scrubs `message` and `hint` together; a
   * local error has no hint until the caller supplies one.
   */
  readonly hint?: string;

  constructor(status: number, code: string, message: string, retryAfterMs?: number, hint?: string) {
    super(message);
    this.name = "ApiFailure";
    this.status = status;
    this.code = code;
    this.retryAfterMs = retryAfterMs;
    this.hint = hint;
  }
}

/** True when the daemon could not be reached at all. */
export function isUnreachable(e: unknown): boolean {
  return e instanceof ApiFailure && e.status === 0;
}

/**
 * Why the editor could not read canvas-rgb's status.
 *
 * `down` is canvas-rgb not answering at all: it is not running, or the port is gone.
 * `engine` is canvas-rgb answering and saying its render engine is not answering (#241):
 * the daemon is up, and starting it again would not help.
 */
export type DaemonFault = "down" | "engine";

/**
 * The fault a failed status read is.
 *
 * The status route's only 503 is the render engine not answering
 * (`crates/canvas-daemon/src/api/status.rs`), so a 503 there says the engine is the
 * problem and not that canvas-rgb is gone. Every other failure is canvas-rgb being
 * gone, which is what the editor said before this.
 */
export function daemonFault(e: unknown): DaemonFault {
  if (e instanceof ApiFailure && e.status === 503) return "engine";
  return "down";
}

/**
 * The text of an error, in one place. A daemon message is already scrubbed at the API edge;
 * a plain `Error` is a local detail, never the text of the prefilled report (#576 R6).
 */
/** The device-id shapes the daemon and the bundle use (P6), mirrored in Rust. */
const ADAPTER_PREFIXES = [
  "openrgb", "ring", "chione", "royuan", "attack-shark-x6", "8bitdo-ultimate-2", "8bitdo", "hid", "usb",
];
const DEVICE_ID = new RegExp(`\\b(?:${ADAPTER_PREFIXES.join("|")}):[A-Za-z0-9#&_.\\\\-]{2,}`, "g");

/** Replaces every device fingerprint in text the editor copies or puts in a URL (P6). */
export function scrubDeviceIds(text: string): string {
  return text.replace(DEVICE_ID, "the device");
}

export function messageOf(e: unknown): string {
  const message = e instanceof ApiFailure || e instanceof Error ? e.message : "unknown error";
  // The daemon scrubs at the edge, but a tray or local message can still carry a device id.
  return scrubDeviceIds(message);
}

/**
 * The next step a failure carries, if any. `ErrorState` is the only component that reads it.
 */
export function hintOf(e: unknown): string | null {
  return e instanceof ApiFailure && e.hint ? e.hint : null;
}

/**
 * The name views use for a piece of error text. The anatomy guard (`test/errorAnatomyGuard.test.ts`)
 * keeps `messageOf` inside `ErrorState`; views take their strings through this alias and render
 * them with `ErrorState` or its compact `ErrorLine`, never as a bare paragraph.
 */
export const errorText = messageOf;

/** True when the error came from the daemon or a scrubbed tray command, safe for the report. */
export function isScrubbedMessage(e: unknown): boolean {
  return e instanceof ApiFailure;
}
