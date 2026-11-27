"use client";

import { useState, type CSSProperties, type ReactNode } from "react";
import {
  ArrowClockwise,
  Copy,
  FolderOpen,
  type Icon,
} from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { hintOf, messageOf } from "@/lib/errors";
import { openLogsFolder, tauriHost } from "@/lib/host";
import s from "./ErrorState.module.css";

/**
 * The one anatomy for an error the editor shows (#576, item 2): what happened as the title,
 * the next step as the lead, and a key row for the actions that can be taken. The card reserves
 * its height so a late hint never moves the page. An error never lives only in a toast.
 *
 * Views hand over the caught value as `error`; the words are formed here. `message` is the title
 * the view wants over it, and the error's own words then stand under it as the detail. A daemon or
 * tray message is already scrubbed before it reaches the editor; `detail` is never used as the
 * report's `what-happened`.
 */
export function ErrorState({
  message,
  hint,
  detail,
  error,
  version,
  onRetry,
  onCopy,
  onOpenLogs,
  onSaveDiagnostics,
  onReport,
  logsEnabled,
  retryLabel = "Try again",
  busy = false,
  compact = false,
  context,
  tooltip = false,
  className,
  style,
  id,
}: {
  /** The title, when the view words it. Without one, the error's own words are the title. */
  message?: ReactNode | null;
  /** The scrubbed next step. Falls back to the hint on an `ApiFailure`. */
  hint?: string | null;
  /** A local detail. Defaults to the error's words when `message` is the title. */
  detail?: string | null;
  /** The caught value (a `Failure`): the title or detail, and the hint. */
  error?: unknown;
  /** The version appended by Copy error. */
  version?: string | null;
  onRetry?: () => void;
  onCopy?: () => void;
  onOpenLogs?: () => void;
  onSaveDiagnostics?: () => void;
  onReport?: () => void;
  /** False outside the tray: the key says "Tray app only" and is disabled. */
  logsEnabled?: boolean;
  /** The retry key's words, when the retry is a different step ("Start again"). */
  retryLabel?: string;
  /** The retry is running: the key shows the wait and takes no second press. */
  busy?: boolean;
  /** An inline error in a row or a modal: the same anatomy, one line. */
  compact?: boolean;
  /** Compact only: the words in front of the error's own ("Rescan failed:"). */
  context?: string;
  /** Compact only: the whole line is also the tooltip, for a row that cuts it short. */
  tooltip?: boolean;
  className?: string;
  style?: CSSProperties;
  id?: string;
}) {
  const said = error !== undefined && error !== null ? messageOf(error) : null;
  const title = message ?? said ?? "Something went wrong.";
  // A repeated sentence is dropped: the detail never says the title again (U5).
  const words = detail ?? (message != null ? said : null);
  const lead = hint ?? (error !== undefined ? hintOf(error) : null);
  const under = words && words !== title && words !== lead ? words : null;
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const [logsRefused, setLogsRefused] = useState<unknown>(null);
  const logs = logsEnabled ?? tauriHost() !== null;
  const openLogs =
    onOpenLogs ??
    (() => {
      setLogsRefused(null);
      void openLogsFolder().catch((e: unknown) => setLogsRefused(e));
    });

  const copy = async () => {
    const text = [title, lead, under, version].filter(Boolean).join("\n");
    setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      onCopy?.();
    } catch {
      // A refused clipboard shows on the key row, never as a toast (#576).
      setCopyFailed(true);
    }
  };

  if (compact) {
    // A row-sized error still goes through this component; ErrorLine lives here, so no view
    // writes a bare error paragraph (F8/F9). With nothing to say the line stays, empty, so a
    // reserved row keeps its room.
    const body = message ?? said;
    const line = body !== null && context ? `${context} ${String(body)}` : body;
    return (
      <ErrorLine
        className={className}
        style={style}
        id={id}
        role="alert"
        title={tooltip && typeof line === "string" ? line : undefined}
      >
        {line ?? ""}
      </ErrorLine>
    );
  }

  return (
    <div
      className={[s.state, className].filter(Boolean).join(" ")}
      style={style}
      id={id}
      role="alert"
      data-anatomy="error-state"
    >
      <p className={s.title}>{title}</p>
      {lead ? <p className={s.lead}>{lead}</p> : null}
      {under ? <p className={s.detail}>{under}</p> : null}
      <div className={s.keys}>
        {onRetry ? (
          <Button size="sm" icon={<ArrowClockwise size={15} />} onClick={onRetry} busy={busy}>
            {retryLabel}
          </Button>
        ) : null}
        <Button size="sm" variant="ghost" icon={<Copy size={15} />} onClick={() => void copy()}>
          {copied ? "Copied" : "Copy error"}
        </Button>
        <KeyRow icon={FolderOpen} label={logs ? "Open logs folder" : "Tray app only"} onClick={openLogs} disabled={!logs} />
        {onSaveDiagnostics ? (
          <KeyRow icon={FolderOpen} label="Save diagnostics" onClick={onSaveDiagnostics} />
        ) : null}
        {onReport ? <KeyRow icon={Copy} label="Report issue" onClick={onReport} /> : null}
        {copyFailed ? <span className={s.note}>The clipboard refused the copy.</span> : null}
        {logsRefused !== null ? <span className={s.note}>{messageOf(logsRefused)}</span> : null}
      </div>
    </div>
  );
}

/** One key on an error's key row: a compact ghost button, disabled when it cannot act. */
export function KeyRow({
  icon: Glyph,
  label,
  onClick,
  disabled = false,
  title,
}: {
  icon: Icon;
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <Button
      size="sm"
      variant="ghost"
      icon={<Glyph size={15} />}
      onClick={onClick}
      disabled={disabled || !onClick}
      title={title}
    >
      {label}
    </Button>
  );
}

/**
 * The compact anatomy for an inline error line. It renders the same class the old bare
 * paragraphs used, so the look and the colour are unchanged; it exists so a view never writes
 * an error paragraph by hand.
 */
export function ErrorLine({
  children,
  id,
  role,
  title,
  style,
  className,
}: {
  children: ReactNode;
  id?: string;
  role?: string;
  title?: string;
  style?: CSSProperties;
  className?: string;
}) {
  return (
    <p
      className={["error-line", s.line, className].filter(Boolean).join(" ")}
      id={id}
      role={role}
      title={title}
      style={style}
    >
      {children}
    </p>
  );
}
