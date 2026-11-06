// Motion gates: the document's visibility and the reduced-motion preference. Anything that loops
// (effect thumbnails, the live preview) checks both before it runs.

import { useSyncExternalStore } from "react";

const REDUCED = "(prefers-reduced-motion: reduce)";

function subscribeVisibility(onChange: () => void): () => void {
  document.addEventListener("visibilitychange", onChange);
  return () => document.removeEventListener("visibilitychange", onChange);
}

/** True while the window is visible. */
export function useDocumentVisible(): boolean {
  return useSyncExternalStore(
    subscribeVisibility,
    () => document.visibilityState === "visible",
    () => true,
  );
}

function subscribeReduced(onChange: () => void): () => void {
  const mq = window.matchMedia(REDUCED);
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}

/** True when the user asks for reduced motion. */
export function usePrefersReducedMotion(): boolean {
  return useSyncExternalStore(
    subscribeReduced,
    () => window.matchMedia(REDUCED).matches,
    () => false,
  );
}

/**
 * The `data-motion="reduced"` switch, read synchronously (Settings >
 * Appearance): the in-app switch can only turn motion off, never on.
 */
export function appMotionOff(): boolean {
  try {
    return document.documentElement.getAttribute("data-motion") === "reduced";
  } catch {
    return false;
  }
}

/**
 * Reduced motion, synchronously: the OS preference or the in-app switch.
 * A JS timer that waits for a CSS transition reads this, so under reduced
 * motion the timer is 0 as well and the change is instant.
 */
export function motionOffNow(): boolean {
  if (appMotionOff()) return true;
  try {
    return window.matchMedia(REDUCED).matches;
  } catch {
    return false;
  }
}

/**
 * Reads a `--dur*` design token in milliseconds at runtime, so a JS timer
 * waits exactly as long as the CSS transition it mirrors (`app/web/src/styles/
 * tokens.css`). `fallback` is the same token's value in that file, used only
 * where no stylesheet is loaded (jsdom tests, SSR).
 */
export function readCssMs(token: string, fallback: number): number {
  try {
    const raw = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
    const ms = /^(\d+(?:\.\d+)?)\s*ms$/.exec(raw)?.[1];
    if (ms !== undefined) return Number(ms);
    const s = /^(\d+(?:\.\d+)?)\s*s$/.exec(raw)?.[1];
    if (s !== undefined) return Number(s) * 1000;
  } catch {
    // No document (SSR): the fallback is the token's own value.
  }
  return fallback;
}
