"use client";

import type { ReactNode } from "react";

/**
 * Two icons that swap (issue #187, rule 8): play/pause, lock/unlock and the
 * like. Both icons overlap in one cell and cross-fade with a small scale and
 * blur — the outgoing one shrinks and blurs away while the incoming one
 * grows out of the blur — instead of a hard swap. Both stay in the tree for
 * the swap, so there is no blank beat. `at` says which one is on. The motion
 * is CSS on the shared tokens, so reduced motion (and the in-app motion
 * switch) makes the swap instant.
 */
export function IconSwap({ a, b, at, label }: { a: ReactNode; b: ReactNode; at: 0 | 1; label?: string }) {
  return (
    <span className="icon-swap" role="img" aria-label={label}>
      <span className={at === 0 ? "icon-swap-layer icon-swap-in" : "icon-swap-layer icon-swap-out"} aria-hidden="true">
        {a}
      </span>
      <span className={at === 1 ? "icon-swap-layer icon-swap-in" : "icon-swap-layer icon-swap-out"} aria-hidden="true">
        {b}
      </span>
    </span>
  );
}
