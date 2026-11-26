"use client";

import { useEffect, useRef } from "react";

/**
 * How often the editor asks again for a list that can change while a page is open. The rail's
 * peripheral count (`shell/peripheralCount.ts`) uses the same beat: a device is plugged in or
 * moves to another connection, not on every frame.
 */
export const POLL_MS = 10_000;

/**
 * Calls `refresh` on every tick while the page is mounted. The first load stays with the page:
 * this is the beat that notices a later change, not a second mount-time request. The newest
 * callback is used, so a refresh that closes over state never goes stale.
 */
export function usePoll(refresh: () => void, everyMs: number = POLL_MS): void {
  const latest = useRef(refresh);
  latest.current = refresh;
  useEffect(() => {
    const timer = setInterval(() => latest.current(), everyMs);
    return () => clearInterval(timer);
  }, [everyMs]);
}
