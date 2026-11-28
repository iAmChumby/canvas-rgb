"use client";

import { useEffect, useState, type ReactNode } from "react";
import { ArrowClockwise, Check } from "@phosphor-icons/react";
import { Button } from "@/components/Button";

/**
 * The state the whole editor shows when it cannot show live data.
 *
 * It says what is wrong in one line, what keeps working in two, and offers a retry that
 * says it is retrying. The daemon's own words, when it gave any, are shown as they are.
 *
 * The line next to the button says when the daemon was last tried and when it will be
 * tried again, so it never repeats the heading.
 */
export function StatePage({
  icon,
  title,
  lead,
  keeps,
  message,
  onRetry,
  lastTryAt,
  nextTryInMs,
  keys,
}: {
  /** The mark at the top of the page. */
  icon: ReactNode;
  /** The heading: what is wrong, in one line. */
  title: string;
  /** The line under the heading. */
  lead: string;
  /** What keeps working while the page is up. */
  keeps: string[];
  /** The daemon's own words, when it gave any. */
  message?: string | null;
  onRetry: () => Promise<void>;
  /** When the daemon was last tried, 0 when it has not been tried yet. */
  lastTryAt?: number;
  /** How long until the next try, in milliseconds. */
  nextTryInMs?: number;
  /** The recovery keys: logs, diagnostics, report, details. Reserved room, no layout shift. */
  keys?: ReactNode;
}) {
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  // The countdown moves on its own, so the line stays true.
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(id);
  }, []);

  const retry = async () => {
    setBusy(true);
    const started = Date.now();
    try {
      await onRetry();
    } finally {
      // A retry that fails in a blink is still a retry: it stays on screen long enough to read.
      const rest = 600 - (Date.now() - started);
      if (rest > 0) await new Promise((r) => setTimeout(r, rest));
      setBusy(false);
    }
  };

  const ago = lastTryAt ? Math.max(0, Math.round((now - lastTryAt) / 1000)) : 0;
  const next = lastTryAt && nextTryInMs ? Math.max(0, Math.ceil((lastTryAt + nextTryInMs - now) / 1000)) : 0;
  return (
    <div className="state-page enter">
      <div className="state-page-inner">
        <span className="state-page-icon" aria-hidden="true">
          {icon}
        </span>
        <h1 className="state-page-title">{title}</h1>
        <p className="state-page-lead">{lead}</p>
        {message ? (
          <p className="state-page-message" data-from="daemon">
            {message}
          </p>
        ) : null}
        <ul className="state-page-keeps">
          {keeps.map((line) => (
            <li key={line}>
              <Check size={14} weight="bold" aria-hidden="true" />
              {line}
            </li>
          ))}
        </ul>
        <div className="state-page-actions">
          <Button onClick={() => void retry()} busy={busy} icon={<ArrowClockwise size={16} />}>
            Try again
          </Button>
          {/* One line of timing. The heading already says what is wrong, and the daemon's own
              words are on their own line, so neither is repeated here. */}
          <span className="state-page-detail" aria-live="polite">
            {lastTryAt
              ? next === 0
                ? `tried ${ago} s ago · Retrying now`
                : `tried ${ago} s ago · next in ${next} s`
              : "not tried yet"}
          </span>
        </div>
        {keys ? <div className="state-page-keys">{keys}</div> : null}
      </div>
    </div>
  );
}
