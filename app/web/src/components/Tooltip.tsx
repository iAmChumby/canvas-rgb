"use client";

import type { ReactNode } from "react";

/** A hover and focus label for an icon control. */
export function Tooltip({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <span className={className ? `tip ${className}` : "tip"}>
      {children}
      <span className="tip-bubble" role="tooltip">
        {label}
      </span>
    </span>
  );
}
