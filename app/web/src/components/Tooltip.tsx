"use client";

import type { ReactNode } from "react";

/** A hover and focus label for an icon control. */
export function Tooltip({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="tip">
      {children}
      <span className="tip-bubble" role="tooltip">
        {label}
      </span>
    </span>
  );
}
