"use client";

import type { CSSProperties, ReactNode } from "react";

export function Panel({
  title,
  actions,
  children,
  className,
}: {
  title?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={className ? `panel ${className}` : "panel"}>
      {(title || actions) && (
        <div className="panel-head">
          {title && <h2 className="panel-title">{title}</h2>}
          {actions}
        </div>
      )}
      {children}
    </section>
  );
}

/** One setting: its name on the left, its control on the right. */
export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="field">
      <div className="field-label">
        <span>{label}</span>
        {hint && <span className="field-hint">{hint}</span>}
      </div>
      <div className="field-control">{children}</div>
    </div>
  );
}

export type DotTone = "ok" | "work" | "warn" | "down" | "mute";

/** A status dot with its words beside it. The words carry the meaning; the colour agrees. */
export function StateDot({ tone, children }: { tone: DotTone; children: ReactNode }) {
  const color = tone === "mute" ? "var(--mute)" : `var(--${tone})`;
  return (
    <span className="state" style={{ "--tone": color } as CSSProperties}>
      <span className="state-dot" aria-hidden="true" />
      {children}
    </span>
  );
}
