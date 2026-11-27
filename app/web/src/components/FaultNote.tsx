"use client";

import type { ReactNode } from "react";
import { Warning } from "@phosphor-icons/react";
import { Button } from "@/components/Button";

/** The five parts every hardware note says, in the daemon's words or the adapter's (#580). */
export interface FaultParts {
  affected: string;
  cause?: string | null;
  fix: string;
  ignore_if?: string | null;
}

/**
 * One hardware note: what is affected, then labelled Cause, Fix and Ignore if lines. The tint is
 * the `--tone` one already used by banners, and every colour is a token. At 720 px and below the
 * title sits above the fields; above that it occupies the left column. Names and ids the reader
 * may need to recognise are handed in as children and drawn in mono by the caller.
 */
export function FaultNote({
  parts,
  tone = "warn",
  children,
  action,
  className = "",
}: {
  parts: FaultParts;
  tone?: "warn" | "down";
  children?: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div className={`fault-note ${className}`.trim()} data-tone={tone} role="status">
      <div className="fault-note-head">
        <span className="fault-note-icon" aria-hidden="true">
          <Warning size={18} weight="fill" />
        </span>
        <h3 className="fault-note-title">{parts.affected}</h3>
        {action && <div className="fault-note-action">{action}</div>}
      </div>
      <dl className="fault-note-fields">
        {parts.cause && (
          <>
            <dt>Cause</dt>
            <dd>{parts.cause}</dd>
          </>
        )}
        <dt>Fix</dt>
        <dd>{parts.fix}</dd>
        {parts.ignore_if && (
          <>
            <dt>Ignore if</dt>
            <dd>{parts.ignore_if}</dd>
          </>
        )}
        {children && (
          <>
            <dt>More</dt>
            <dd>{children}</dd>
          </>
        )}
      </dl>
    </div>
  );
}

/** The button a dismissable page note carries. The note stays in the strip's count (R14). */
export function DismissFault({ busy = false, onDismiss }: { busy?: boolean; onDismiss: () => void }) {
  return (
    <Button size="sm" variant="ghost" busy={busy} onClick={onDismiss}>
      Dismiss
    </Button>
  );
}
