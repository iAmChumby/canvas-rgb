// The error the editor shows. `status` 0 means the daemon did not answer at all.

export class ApiFailure extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiFailure";
    this.status = status;
    this.code = code;
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
