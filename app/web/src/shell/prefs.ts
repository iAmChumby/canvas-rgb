"use client";

import { previewActive } from "@/lib/previewMode";
import { storeLocal } from "@/lib/storage";

/**
 * The two preferences the editor keeps for this viewer: the theme and how much
 * motion the interface uses. Both live in localStorage and are applied to the
 * document before the first paint (the theme) or as soon as they change (motion).
 */

export type Theme = "dark" | "light" | "system";
export const THEME_KEY = "canvas-rgb.theme";

export type MotionPref = "full" | "reduced";
export const MOTION_KEY = "canvas-rgb.motion";

/** A preview's own choices (#579): it writes nothing, so they live with the page. */
const previewChoice: { theme?: Theme; motion?: MotionPref } = {};

export function readTheme(): Theme {
  if (typeof window === "undefined") return "system";
  if (previewChoice.theme) return previewChoice.theme;
  try {
    const v = window.localStorage.getItem(THEME_KEY);
    if (v === "light" || v === "dark") return v;
  } catch {
    // Storage can be blocked; the default is to follow Windows.
  }
  return "system";
}

export function readMotion(): MotionPref {
  if (typeof window === "undefined") return "full";
  if (previewChoice.motion) return previewChoice.motion;
  try {
    const v = window.localStorage.getItem(MOTION_KEY);
    if (v === "reduced") return "reduced";
  } catch {
    // Storage can be blocked; the default is full motion.
  }
  return "full";
}

function prefersReduced(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
}

/**
 * Applies the theme. When the browser can cross-fade (View Transitions), the
 * change is animated; otherwise it is instant.
 */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  const set = () => {
    if (theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", theme);
  };
  const view = (document as Document & { startViewTransition?: (cb: () => void) => unknown }).startViewTransition;
  if (!prefersReduced() && typeof view === "function") {
    try {
      view.call(document, set);
    } catch {
      set();
    }
  } else {
    set();
  }
  if (previewActive()) previewChoice.theme = theme;
  try {
    storeLocal(THEME_KEY, theme);
  } catch {
    // The choice still applies for this session.
  }
}

/** Applies the motion preference. "Reduced" can only take motion away. */
export function applyMotion(pref: MotionPref): void {
  const root = document.documentElement;
  if (pref === "reduced") root.setAttribute("data-motion", "reduced");
  else root.removeAttribute("data-motion");
  if (previewActive()) previewChoice.motion = pref;
  try {
    storeLocal(MOTION_KEY, pref);
  } catch {
    // The choice still applies for this session.
  }
}

/** The theme that follows the one that is set, for the command palette. */
export function nextTheme(theme: Theme): Theme {
  if (theme === "dark") return "light";
  if (theme === "light") return "system";
  return "dark";
}
