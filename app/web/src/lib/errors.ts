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

export function messageOf(e: unknown): string {
  if (e instanceof ApiFailure) return e.message;
  if (e instanceof Error) return e.message;
  return "unknown error";
}
