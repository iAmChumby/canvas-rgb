"use client";

import { SkeletonLines } from "@/components/Skeleton";

/**
 * A read-only receipt: the support report as the daemon sent it. It is a component, not a raw
 * `<pre>` (`ui-taste.md` rule 13), and it shows the stored string unchanged. The editor never
 * formats, reflows or rebuilds the text here: what Copy puts on the clipboard is exactly what this
 * draws, and the daemon owns the scrubbing.
 *
 * Before the answer arrives the receipt holds its real height with skeleton lines, so the panels
 * below it do not move when the text comes (#574).
 */
export function Receipt({
  text,
  label,
  loading = false,
}: {
  text: string;
  label: string;
  loading?: boolean;
}) {
  return (
    <div className="receipt" role="group" aria-label={label}>
      <div
        className="receipt-text"
        tabIndex={0}
        aria-label={`${label}, read only`}
        aria-busy={loading || undefined}
        data-testid={loading ? undefined : "receipt-text"}
      >
        {loading ? <SkeletonLines rows={15} /> : text}
      </div>
    </div>
  );
}
