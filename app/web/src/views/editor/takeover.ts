// The takeover: while the onboarding is open it owns the whole window. The shell reads this to
// hide the rail, the strip and the banners; the onboarding sets it while it is on screen.
//
// It is a plain external store (the same shape as `useSnap`) so the shell does not have to
// import the Layout page to know whether the onboarding is open.

import { useSyncExternalStore } from "react";

let open = false;
const listeners = new Set<() => void>();

/** Says whether the onboarding is on screen. Called by the onboarding, on mount and unmount. */
export function setTakeover(on: boolean): void {
  if (open === on) return;
  open = on;
  for (const l of listeners) l();
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

/** True while the onboarding has the window. */
export function useTakeover(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => open,
    () => false,
  );
}
