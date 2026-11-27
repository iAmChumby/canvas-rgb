"use client";

import type { ReactNode } from "react";
import { CaretRight } from "@phosphor-icons/react";

/**
 * The licence text behind one group of third-party notices. The full texts are
 * folded away: a page with 300 packages shows 11 summaries, not 300 expanded
 * blocks. Built on the component layer, styled with the same tokens as the
 * other disclosures, and keyboard reachable without a raw HTML control in a
 * view.
 */
export function LicenceDisclosure({
  label,
  children,
  className,
  caretClassName,
  bodyClassName,
}: {
  label: string;
  children: ReactNode;
  className?: string;
  caretClassName?: string;
  bodyClassName?: string;
}) {
  return (
    <details className={className}>
      <summary>
        <CaretRight className={caretClassName} size={14} aria-hidden="true" />
        <span>{label}</span>
      </summary>
      <div className={bodyClassName}>{children}</div>
    </details>
  );
}
