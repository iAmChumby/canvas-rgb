// The one question the shell asks before it changes page.
//
// An editor with unsaved code registers a guard while it has changes. A rail link, a palette
// page or the tray's navigate then asks the guard first; the guard answers true when the page
// may change, and shows its own Save / Discard / Keep editing prompt when it may not. The
// navigation it refused is held here, so Discard can carry it out and Keep editing can drop it.
//
// A page change that never asks (the browser's own back button) is outside this: leaving the
// window, reloading and closing are covered by the editor's `beforeunload`.

type Guard = () => boolean;
type Pending = () => void;

let guard: Guard | null = null;
let pending: Pending | null = null;

/** Registers the guard while the edit has changes; null when there is nothing to lose. */
export function setNavigationGuard(next: Guard | null): void {
  guard = next;
  if (!next) pending = null;
}

/** True while a page has a guard: the shell then asks before it navigates. */
export function hasNavigationGuard(): boolean {
  return guard !== null;
}

/**
 * Goes now when nothing guards the page or the guard allows it, and otherwise holds the
 * navigation until the guard's prompt says Discard or Save. Returns whether it went.
 */
export function requestNavigation(go: Pending): boolean {
  if (guard && !guard()) {
    pending = go;
    return false;
  }
  go();
  return true;
}

/** The guard's prompt said Discard or Save: the page change it held may happen now. */
export function continueNavigation(): void {
  const go = pending;
  pending = null;
  go?.();
}

/** The guard's prompt said Keep editing: the held page change is dropped. */
export function cancelNavigation(): void {
  pending = null;
}
