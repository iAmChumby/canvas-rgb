// The desk canvas's view, remembered per layout.
//
// The zoom and the pan are a view transform: they never reach the layout the daemon stores.
// They are kept here, in this viewer's localStorage, so that coming back to a layout shows it
// as it was left — the same way the theme and the motion preference are kept in `shell/prefs.ts`.

export const VIEW_KEY = "canvas-rgb.views";

export interface CanvasView {
  scale: number;
  panX: number;
  panY: number;
}

/** The view as it is when the canvas is shown whole. */
export const FIT_VIEW: CanvasView = { scale: 1, panX: 0, panY: 0 };

function readAll(): Record<string, CanvasView> {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(VIEW_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    return parsed as Record<string, CanvasView>;
  } catch {
    // Storage can be blocked, or hold something else.
    return {};
  }
}

/** The view of one layout, or null when it has never been changed. */
export function readView(layoutId: string): CanvasView | null {
  if (!layoutId) return null;
  const all = readAll();
  const v = all[layoutId];
  if (!v || typeof v !== "object") return null;
  const { scale, panX, panY } = v;
  if (typeof scale !== "number" || typeof panX !== "number" || typeof panY !== "number") return null;
  if (!Number.isFinite(scale) || scale <= 0) return null;
  return { scale, panX, panY };
}

/** Remembers the view of one layout. */
export function writeView(layoutId: string, view: CanvasView): void {
  if (!layoutId || typeof window === "undefined") return;
  try {
    const all = readAll();
    all[layoutId] = { scale: view.scale, panX: view.panX, panY: view.panY };
    window.localStorage.setItem(VIEW_KEY, JSON.stringify(all));
  } catch {
    // Storage can be blocked; the view still applies for this session.
  }
}

/** Forgets the view of one layout: the fit control uses it. */
export function forgetView(layoutId: string): void {
  if (!layoutId || typeof window === "undefined") return;
  try {
    const all = readAll();
    delete all[layoutId];
    window.localStorage.setItem(VIEW_KEY, JSON.stringify(all));
  } catch {
    // Storage can be blocked.
  }
}
