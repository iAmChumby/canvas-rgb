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
import s from "./ErrorState.module.css";

/**
 * The one anatomy for an error the editor shows (#576, item 2): what happened as the title,
 * the next step as the lead, and a key row for the actions that can be taken. The card reserves
 * its height so a late hint never moves the page. An error never lives only in a toast.
 *
 * `message` is a daemon or tray message, already scrubbed before it reaches the editor.
 * `detail` is the local text from `messageOf(e)`; it is never used as the report's `what-happened`.
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
  logsEnabled = true,
}: {
  /** The scrubbed "what happened". */
  message?: string | null;
  /** The scrubbed next step. Falls back to the hint on an `ApiFailure`. */
  hint?: string | null;
  /** A local detail, e.g. `errorText(e)` for a plain `Error`. */
  detail?: string | null;
  /** Convenience: take title, hint and detail from one caught value. */
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
}) {
  const title = message ?? (error !== undefined ? messageOf(error) : "Something went wrong.");
  const lead = hint ?? (error !== undefined ? hintOf(error) : null);
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);

  const copy = async () => {
    const text = [title, lead, version].filter(Boolean).join("\n");
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

  return (
    <div className={s.state} role="alert" data-anatomy="error-state">
      <p className={s.title}>{title}</p>
      {lead ? <p className={s.lead}>{lead}</p> : null}
      {detail ? <p className={s.detail}>{detail}</p> : null}
      <div className={s.keys}>
        {onRetry ? (
          <Button size="sm" icon={<ArrowClockwise size={15} />} onClick={onRetry}>
            Try again
          </Button>
        ) : null}
        <Button size="sm" variant="ghost" icon={<Copy size={15} />} onClick={() => void copy()}>
          {copied ? "Copied" : "Copy error"}
        </Button>
        <KeyRow icon={FolderOpen} label={logsEnabled ? "Open logs folder" : "Tray app only"} onClick={onOpenLogs} disabled={!logsEnabled || !onOpenLogs} />
        {onSaveDiagnostics ? (
          <KeyRow icon={FolderOpen} label="Save diagnostics" onClick={onSaveDiagnostics} />
        ) : null}
        {onReport ? <KeyRow icon={Copy} label="Report issue" onClick={onReport} /> : null}
        {copyFailed ? <span className={s.note}>The clipboard refused the copy.</span> : null}
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
