"use client";

import type { CSSProperties, ReactNode } from "react";
import { X } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";

export type Tone = "warn" | "down" | "work" | "ok";

/** A full-width notice. `down` is the red service banner; `warn` the conflict notice. */
export function Banner({
  tone,
  icon,
  title,
  children,
  actions,
  dismissLabel,
  onDismiss,
}: {
  tone: Tone;
  icon: ReactNode;
  title: string;
  children?: ReactNode;
  actions?: ReactNode;
  dismissLabel?: string;
  onDismiss?: () => void;
}) {
  const style = { "--tone": `var(--${tone})` } as CSSProperties;
  return (
    <div className="banner" style={style} role={tone === "down" ? "alert" : "status"}>
      <span aria-hidden="true" style={{ color: "var(--tone)", display: "inline-flex", paddingTop: 2 }}>
        {icon}
      </span>
      <div className="banner-body">
        <div className="banner-title">{title}</div>
        {children && <div className="banner-text">{children}</div>}
        {actions && <div className="banner-actions">{actions}</div>}
      </div>
      {onDismiss && (
        <IconButton label={dismissLabel ?? "Dismiss"} icon={<X size={16} />} size="sm" onClick={onDismiss} />
      )}
    </div>
  );
}
