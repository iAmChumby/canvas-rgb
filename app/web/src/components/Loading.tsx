"use client";

import { useEffect, useState, type ReactNode } from "react";
import { CaretRight } from "@phosphor-icons/react";
import { Panel } from "@/components/Panel";
import { Button } from "@/components/Button";

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

/** A load shorter than this shows no placeholder. */
export const LOAD_DELAY_MS = 150;

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

/**
 * True once `active` has been true for `delay` ms; false the moment it ends. A load that finishes
 * inside the delay never turns it on.
 */
export function useLoadDelay(active: boolean, delay: number = LOAD_DELAY_MS): boolean {
  const [elapsed, setElapsed] = useState(false);
  useEffect(() => {
    if (!active) return;
    const timer = setTimeout(() => setElapsed(true), delay);
    return () => {
      clearTimeout(timer);
      setElapsed(false);
    };
  }, [active, delay]);
  return active && elapsed;
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

/** The words behind a short message, folded away until asked for: the raw text of an error. */
export function Details({ summary = "Details", children }: { summary?: string; children: ReactNode }) {
  return (
    <details className="load-detail">
      <summary>
        <CaretRight className="load-detail-caret" size={14} aria-hidden="true" />
        <span>{summary}</span>
      </summary>
      <p className="state-detail">{children}</p>
    </details>
  );
}

/**
 * The plain error: a titled panel with what went wrong, and a way to try again.
 *
 * The line under the title is the one the reader sees: one sentence in plain words, with no
 * jargon and no raw answer from the service. The raw words stay behind the `Details`
 * disclosure, where a person who wants them can open it. Pass `onRetry` wherever a load can
 * be asked for again; a panel without one is a dead end.
 */
export function ErrorPanel({
  title,
  message,
  summary = "The answer did not arrive. Nothing was changed.",
  onRetry,
}: {
  title: string;
  /** The raw words, shown only inside the disclosure. */
  message?: ReactNode;
  /** The one line the reader sees. */
  summary?: string;
  onRetry?: () => void;
}) {
  return (
    <Panel title={title}>
      <p className="error-line" role="alert">
        {summary}
      </p>
      {message ? <Details>{message}</Details> : null}
      {onRetry && (
        <div className="load-note-action">
          <Button size="sm" onClick={onRetry}>
            Try again
          </Button>
        </div>
      )}
    </Panel>
  );
}

export function LoadFrame({
  state,
  shape,
  stage,
  refreshing = false,
  error,
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
  /** The error state: an `ErrorPanel`, or any panel. */
  error?: ReactNode;
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
          {state === "error" && <div className="load-note">{error}</div>}
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
