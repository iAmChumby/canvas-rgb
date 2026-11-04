// The left rail's width: how wide it is, and how wide it may be.
//
// The rail is the shell's first grid column (`styles/app.css`), so its width is a page-level
// fact. It is kept in this viewer's localStorage, applied before the first paint by the inline
// script in `app/layout.tsx` (the same early mechanism the theme uses), and read back here.
//
// The bounds keep the page column whole. The rail is never narrower than its own default
// (200 px), so nothing in it truncates, and it never takes more than a quarter of the window
// (and never more than 320 px), so the page keeps the room the layouts were fitted to (#67).
// When the window shrinks, the rail gives way first: the width on screen is the stored width
// clamped to the window's own maximum, and the stored width is kept, so a wider window gets it
// back.

/** The rail's width when nobody has changed it, and the narrowest it may be. */
export const RAIL_DEFAULT_PX = 200;
export const RAIL_MIN_PX = 200;
/** The widest the rail may ever be. */
export const RAIL_MAX_PX = 320;
/** The share of the window the rail may take. */
export const RAIL_MAX_FRACTION = 0.25;
/** Where the width is kept, per viewer. */
export const RAIL_KEY = "canvas-rgb.rail-w";
/** The keyboard's steps: one step, and the bigger one with Shift held. */
export const RAIL_STEP_PX = 8;
export const RAIL_BIG_STEP_PX = 32;

/** The widest the rail may be in a window of this width, in css pixels. */
export function railMaxPx(windowWidth: number): number {
  if (!Number.isFinite(windowWidth) || windowWidth <= 0) return RAIL_MAX_PX;
  const share = Math.floor(windowWidth * RAIL_MAX_FRACTION);
  return Math.max(RAIL_MIN_PX, Math.min(RAIL_MAX_PX, share));
}

/**
 * The width the rail shows in a window of this width: the width asked for, inside the bounds.
 * A width that is not a number is the default.
 */
export function clampRailWidth(width: number, windowWidth: number): number {
  if (!Number.isFinite(width)) return RAIL_DEFAULT_PX;
  const rounded = Math.round(width);
  return Math.min(railMaxPx(windowWidth), Math.max(RAIL_MIN_PX, rounded));
}

/** The width this viewer asked for, or null when they never chose one. */
export function readStoredRailWidth(): number | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(RAIL_KEY);
    if (raw === null) return null;
    const n = Number.parseFloat(raw);
    return Number.isFinite(n) ? n : null;
  } catch {
    // Storage can be blocked, or hold something else.
    return null;
  }
}

/** Keeps the width for the next time the editor is opened. */
export function storeRailWidth(px: number): void {
  if (typeof window === "undefined" || !Number.isFinite(px)) return;
  try {
    window.localStorage.setItem(RAIL_KEY, String(Math.round(px)));
  } catch {
    // Storage can be blocked; the width still applies for this session.
  }
}

/** Sets the width the shell's grid reads. */
export function applyRailWidth(px: number): void {
  if (typeof document === "undefined" || !Number.isFinite(px)) return;
  document.documentElement.style.setProperty("--rail-w", `${Math.round(px)}px`);
}

/** The width the page is showing right now, read back from the document. */
export function readAppliedRailWidth(): number | null {
  if (typeof document === "undefined") return null;
  const raw = document.documentElement.style.getPropertyValue("--rail-w").trim();
  const n = Number.parseFloat(raw);
  return Number.isFinite(n) ? n : null;
}

/**
 * The width, applied before the first paint.
 *
 * This runs in the page's head, before anything has been painted, so the rail is already the
 * width the viewer chose: no layout shift on load. It is the same early mechanism the theme uses
 * (`app/layout.tsx`), and it takes the same bound the editor takes, so the width the page starts
 * with and the width the editor reads back are the same number.
 *
 * The names are bare globals (`localStorage`, `innerWidth`, `document`), so the script can be run
 * in a test.
 */
export const RAIL_EARLY_SCRIPT = [
  "try{",
  `var w=localStorage.getItem(${JSON.stringify(RAIL_KEY)});`,
  "if(w){w=parseFloat(w);",
  "if(isFinite(w)){",
  `var mx=Math.min(${RAIL_MAX_PX},Math.floor(innerWidth*${RAIL_MAX_FRACTION}));`,
  `if(mx<${RAIL_MIN_PX})mx=${RAIL_MIN_PX};`,
  `if(w<${RAIL_MIN_PX})w=${RAIL_MIN_PX};`,
  "if(w>mx)w=mx;",
  'document.documentElement.style.setProperty("--rail-w",w+"px");',
  "}}",
  "}catch(e){}",
].join("");
