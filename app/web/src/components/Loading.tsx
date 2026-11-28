"use client";

import type { ReactElement, ReactNode } from "react";
import { ErrorState } from "@/components/ErrorState";
import { hintOf } from "@/lib/errors";
import { useLoadDelay } from "@/lib/loadDelay";

// The delay now lives in lib/loadDelay.ts (Button needs it, and Button cannot import this file);
// it is re-exported here, where every caller has always found it.
export { LOAD_DELAY_MS, useLoadDelay } from "@/lib/loadDelay";

/**
 * The one loading component. Every view that waits for the daemon draws its wait with this, so a
 * wait looks and behaves the same everywhere:
 *
 * - The frame keeps the shape of the content that is coming (`shape`), so nothing moves when the
 *   data arrives. The error and the empty state sit in the same frame, over the same shape.
 * - A load that ends inside `LOAD_DELAY_MS` shows nothing: the shape holds its room and stays
 *   invisible until the delay has passed, so a fast answer never flashes a placeholder.
 * - A read that talks to a device says what it is doing (`stage`): "Waking the mouse",
 *   "Reading settings · 3 of 5".
 * - Values the page already knows stay on screen, marked as refreshing (`refreshing`), instead of
 *   blanking out.
 */

/** What a frame is showing. `ready` shows the children; the other three show the frame's own. */
export type LoadState = "loading" | "ready" | "error" | "empty";

/** The step of a device read: a line of words, and how far along it is when that is known. */
export interface LoadStage {
  /** "Waking the mouse", "Reading settings". */
  label: string;
  /** The reports in so far. */
  done?: number;
  /** The reports to come in all; 0 or missing when not known. */
  total?: number;
}

/** The stage as one line: "Reading settings · 3 of 5". */
export function stageLine(stage: LoadStage): string {
  const total = stage.total ?? 0;
  if (total > 0) return `${stage.label} · ${stage.done ?? 0} of ${total}`;
  return stage.label;
}

/** The stage line, with a row of ticks that fill as the reports come in. */
function StageChip({ stage }: { stage: LoadStage }) {
  const total = stage.total ?? 0;
  const done = Math.min(stage.done ?? 0, total);
  return (
    <div className="load-stage" role="status" aria-live="polite">
      <span className="load-stage-text fig">{stageLine(stage)}</span>
      {total > 0 && total <= 12 && (
        <span className="load-ticks" aria-hidden="true">
          {Array.from({ length: total }, (_, i) => (
            <span key={i} className="load-tick" data-on={i < done ? "true" : "false"} />
          ))}
        </span>
      )}
    </div>
  );
}

/**
 * A load that failed: the error anatomy, in the frame the page loads into.
 *
 * The title and the lead are plain words. The lead is the daemon's next step when it sent one,
 * and the view's `summary` otherwise; the error's own words stand under it. Pass `onRetry`
 * wherever a load can be asked for again; an error without one is a dead end.
 */
export function ErrorPanel({
  title,
  error,
  summary = "The answer did not arrive. Nothing was changed.",
  onRetry,
  retryLabel,
  busy,
}: {
  title: string;
  /** The caught value. */
  error?: unknown;
  /** The next step the reader sees when the daemon sent none. */
  summary?: string;
  onRetry?: () => void;
  /** The action's words: "Try again" unless the retry is a different step, like "Start again". */
  retryLabel?: string;
  /** The retry is running: the key shows the wait and takes no second press. */
  busy?: boolean;
}) {
  return (
    <ErrorState
      className="panel"
      message={title}
      hint={hintOf(error) ?? summary}
      error={error ?? undefined}
      onRetry={onRetry}
      retryLabel={retryLabel}
      busy={busy}
    />
  );
}

export function LoadFrame({
  state,
  shape,
  stage,
  refreshing = false,
  error: errorPanel,
  empty,
  label = "Loading",
  className,
  children,
}: {
  state: LoadState;
  /** Placeholders in the shape of the content that is coming. Built from `Skeleton`. */
  shape: ReactNode;
  /** What a device read is doing now. Shown with the placeholder, and with a refresh. */
  stage?: LoadStage | null;
  /** Content the page already knows is on screen (`state="ready"`) while it is read again. */
  refreshing?: boolean;
  /** The error state: an `ErrorPanel` or an element built on `ErrorState`, never loose words. */
  error?: ReactElement | null;
  /** The empty state: a panel holding an `EmptyState`. */
  empty?: ReactNode;
  /** What a screen reader hears while the placeholder stands. */
  label?: string;
  className?: string;
  children?: ReactNode;
}) {
  const loading = state === "loading";
  const shown = useLoadDelay(loading);
  const marked = useLoadDelay(state === "ready" && refreshing);
  const classes = className ? `load-frame ${className}` : "load-frame";
  // The stage line shows with the placeholder, or with the refresh mark: never before the delay.
  const showStage = (loading && shown) || marked;

  return (
    <div
      className={classes}
      data-state={state}
      data-shown={loading ? (shown ? "true" : "false") : undefined}
      data-refreshing={marked ? "true" : undefined}
      aria-busy={loading || (state === "ready" && refreshing) ? true : undefined}
    >
      {state === "ready" ? (
        children
      ) : (
        <>
          {/* The same shape in loading, error and empty: the frame is as tall in all three. */}
          <div className="load-shape" aria-hidden="true">
            {shape}
          </div>
          {state === "error" && <div className="load-note">{errorPanel}</div>}
          {state === "empty" && <div className="load-note">{empty}</div>}
          {/* The region stands while loading; its words come with the placeholder, so a load that
              ends inside the delay is never announced. */}
          {loading && (
            <span className="sr-only" role="status">
              {shown ? label : ""}
            </span>
          )}
        </>
      )}
      {showStage && stage && <StageChip stage={stage} />}
    </div>
  );
}
