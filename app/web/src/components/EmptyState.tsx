"use client";

import type { ReactNode } from "react";

/**
 * A calm empty state: what is missing, one line about it, and the control that fixes it.
 * Nothing here shouts: no red walls of text, no stack traces.
 */
export function EmptyState({
  icon,
  title,
  children,
  action,
  tone = "mute",
}: {
  icon?: ReactNode;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
  tone?: "mute" | "warn";
}) {
  return (
    <div className="empty-state" data-tone={tone}>
      {icon && (
        <span className="empty-state-icon" aria-hidden="true">
          {icon}
        </span>
      )}
      <h2 className="empty-state-title">{title}</h2>
      {children && <div className="empty-state-body">{children}</div>}
      {action && <div className="empty-state-action">{action}</div>}
    </div>
  );
}
