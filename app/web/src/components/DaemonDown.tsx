"use client";

import { useEffect, useState } from "react";
import { ArrowClockwise, Check, CloudSlash, SpinnerGap } from "@phosphor-icons/react";
import { Button } from "@/components/Button";

/**
 * The state the whole editor shows when it cannot reach the daemon. It says what is wrong in
 * one line, what keeps working in two, and offers a retry that says it is retrying.
 *
 * The line next to the button says when the daemon was last tried and when it will be tried
 * again, so it never repeats the heading.
 */
export function DaemonDown({
  detail,
  onRetry,
  lastTryAt,
  nextTryInMs,
}: {
  detail: string;
  onRetry: () => Promise<void>;
  /** When the daemon was last tried, 0 when it has not been tried yet. */
  lastTryAt?: number;
  /** How long until the next try, in milliseconds. */
  nextTryInMs?: number;
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
  // The heading already says the daemon is not answering: the line must not say it again.
  const says = (text: string) => text.replace(/[.\s]+$/, "").toLowerCase();
  const extra = detail && says(detail) !== says("The daemon is not answering") ? ` · ${detail}` : "";

  return (
    <div className="state-page enter">
      <div className="state-page-inner">
        <span className="state-page-icon" aria-hidden="true">
          <CloudSlash size={26} />
        </span>
        <h1 className="state-page-title">The daemon is not answering</h1>
        <p className="state-page-lead">
          canvas-rgb draws the lights through the daemon on 127.0.0.1:9430. It may still be
          starting, or it may have stopped.
        </p>
        <ul className="state-page-keeps">
          <li>
            <Check size={14} weight="bold" aria-hidden="true" />
            The lights keep the frame they had.
          </li>
          <li>
            <Check size={14} weight="bold" aria-hidden="true" />
            The fans stay under BIOS control.
          </li>
        </ul>
        <div className="state-page-actions">
          <Button onClick={() => void retry()} disabled={busy} icon={busy ? <SpinnerGap size={16} className="spin" /> : <ArrowClockwise size={16} />}>
            {busy ? "Retrying…" : "Try again"}
          </Button>
          <span className="state-page-detail" aria-live="polite">
            {lastTryAt ? (
              <>
                tried {ago} s ago · next in {next} s
                {extra}
              </>
            ) : (
              detail || "the daemon has not answered yet"
            )}
          </span>
        </div>
      </div>
    </div>
  );
}
