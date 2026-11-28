"use client";

import { ErrorState } from "@/components/ErrorState";
import { useSnap } from "@/lib/store";
import s from "./ErrorState.module.css";

/**
 * The note that what a page shows may be old (#576, K1): the last read failed and the page keeps
 * what it had. The row is there with or without the note. `everyMs` is the beat the store reads
 * this area on.
 */
export function StaleNotice({
  message,
  error,
  everyMs,
  onRetry,
}: {
  message: string;
  /** The failed read, or null while the page is up to date. */
  error: string | null;
  everyMs: number;
  onRetry: () => void;
}) {
  // While canvas-rgb itself is not answering the polls back off, so no beat is promised.
  const away = useSnap((s) => s.statusError !== null);
  const beat = everyMs === 1000 ? "every second" : `every ${everyMs / 1000} s`;
  return (
    <div className={s.slot} data-slot="stale-notice">
      {error !== null ? (
        <ErrorState
          compact
          details
          message={message}
          error={error}
          next={away ? "Retrying when canvas-rgb answers." : `Retrying ${beat}.`}
          onRetry={onRetry}
        />
      ) : null}
    </div>
  );
}
