"use client";

import { useCallback, useSyncExternalStore } from "react";

/**
 * True while the media query matches. The editor uses it where the layout has to change
 * shape, not just size: the effect detail becomes its own view on a phone, and the fan
 * headers become cards instead of a table.
 */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      const mq = window.matchMedia(query);
      mq.addEventListener("change", onChange);
      return () => mq.removeEventListener("change", onChange);
    },
    [query],
  );
  const snapshot = useCallback(() => window.matchMedia(query).matches, [query]);
  return useSyncExternalStore(subscribe, snapshot, () => false);
}
