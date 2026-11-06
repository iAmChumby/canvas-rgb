"use client";

import { useState, type ReactNode } from "react";

/**
 * Two icons that swap (issue #187, rule 8): play/pause, lock/unlock and the
 * like. Both icons overlap in one cell and cross-fade with a small scale and
 * blur — the outgoing one shrinks and blurs away while the incoming one
 * grows out of the blur — instead of a hard swap. Both stay in the tree for
 * the swap, so there is no blank beat. `at` says which one is on. The motion
 * is CSS on the shared tokens, so reduced motion (and the in-app motion
 * switch) makes the swap instant.
 *
 * First mount is static: each layer carries its on/off state (`data-on`) and
 * the cross-fade runs only on a real change after mount, never on the mount
 * itself (the render-phase adjustment below arms the animation).
 */
export function IconSwap({ a, b, at, label }: { a: ReactNode; b: ReactNode; at: 0 | 1; label?: string }) {
  const [live, setLive] = useState(false);
  const [last, setLast] = useState(at);
  if (at !== last) {
    setLast(at);
    setLive(true);
  }
  return (
    <span className="icon-swap" {...(label !== undefined ? { role: "img" as const, "aria-label": label } : {})}>
      <span
        className={live ? (at === 0 ? "icon-swap-layer icon-swap-in" : "icon-swap-layer icon-swap-out") : "icon-swap-layer"}
        data-on={at === 0 ? "true" : "false"}
        aria-hidden="true"
      >
        {a}
      </span>
      <span
        className={live ? (at === 1 ? "icon-swap-layer icon-swap-in" : "icon-swap-layer icon-swap-out") : "icon-swap-layer"}
        data-on={at === 1 ? "true" : "false"}
        aria-hidden="true"
      >
        {b}
      </span>
    </span>
  );
}
