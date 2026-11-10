"use client";

import { useEffect, useState } from "react";

/** A load shorter than this shows no placeholder, spinner or skeleton. */
export const LOAD_DELAY_MS = 150;

/**
 * True once `active` has been true for `delay` ms; false the moment it ends. A load that finishes
 * inside the delay never turns it on. Every wait in the editor (the whole-view `LoadFrame`, the
 * busy `Button`, `InlineSkeleton`, the thumbnail placeholder) asks this one hook, so none of them
 * flashes on a fast answer.
 */
export function useLoadDelay(active: boolean, delay: number = LOAD_DELAY_MS): boolean {
  const [elapsed, setElapsed] = useState(false);
  useEffect(() => {
    if (!active) return;
    const timer = setTimeout(() => setElapsed(true), delay);
    return () => {
      clearTimeout(timer);
      setElapsed(false);
    };
  }, [active, delay]);
  return active && elapsed;
}
