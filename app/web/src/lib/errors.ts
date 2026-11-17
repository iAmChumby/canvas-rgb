// The error the editor shows. `status` 0 means the daemon did not answer at all.

export class ApiFailure extends Error {
  readonly status: number;
  readonly code: string;
  /**
   * The wait the daemon asked for with `Retry-After`, in milliseconds. Only the routes that
   * say how long to wait carry one (the effect preview while both render slots are busy).
   */
  readonly retryAfterMs?: number;

  constructor(status: number, code: string, message: string, retryAfterMs?: number) {
    super(message);
    this.name = "ApiFailure";
    this.status = status;
    this.code = code;
    this.retryAfterMs = retryAfterMs;
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

export function messageOf(e: unknown): string {
  if (e instanceof ApiFailure) return e.message;
  if (e instanceof Error) return e.message;
  return "unknown error";
}
