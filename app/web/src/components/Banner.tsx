"use client";

import type { CSSProperties, ReactNode } from "react";
import { X } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";

export type Tone = "warn" | "down" | "work" | "ok";

/** A full-width notice. `down` is the red service banner; `warn` the conflict notice.
 *
 * It is a standing notice: it takes its own place in the page and stays while the condition holds
 * or until the person dismisses it. Nothing here comes and goes on its own. A message that does
 * (a success, an undo, the "Updated to" greeting) is a toast, drawn by `components/Notify.tsx`,
 * and `src/test/toastGuard.test.ts` fails on a second handmade one (#366).
 */
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
      <span aria-hidden="true" className="banner-icon">
        {icon}
      </span>
      <div className="banner-body">
        <div className="banner-head">
          <div className="banner-title">{title}</div>
          {actions && <div className="banner-actions">{actions}</div>}
        </div>
        {children && <div className="banner-text">{children}</div>}
      </div>
      {onDismiss && (
        <IconButton label={dismissLabel ?? "Dismiss"} icon={<X size={16} />} size="sm" onClick={onDismiss} />
      )}
    </div>
  );
}
