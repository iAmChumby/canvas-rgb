// The one question the shell asks before it changes page.
//
// An editor with unsaved code registers a guard while it has changes. A rail link, a palette
// page, the tray's navigate, a link button or the browser's own Back then asks the guard first;
// the guard answers true when the page may change, and shows its own Save / Discard / Keep
// editing prompt when it may not. The navigation it refused is held here, so Discard or Save
// can carry it out and Keep editing can drop it.
//
// A held navigation outlives the guard's own removal. Save makes the source clean before its
// last await, and the editor's clean-up unregisters the guard then; the page change still has to
// happen once the save lands. Only an unmount that nobody answered, or Keep editing, cancels it.

type Guard = () => boolean;
type Pending = () => void;

let guard: Guard | null = null;
let pending: Pending | null = null;

/**
 * Registers the guard while the edit has changes, and removes it when there is nothing to lose.
 * Removing it leaves a navigation the prompt is still holding: the prompt answers that, not the
 * guard's lifetime.
 */
export function setNavigationGuard(next: Guard | null): void {
  guard = next;
}

/** True while a page has a guard: the shell then asks before it navigates. */
export function hasNavigationGuard(): boolean {
  return guard !== null;
}

/** True while a page change is held for the prompt: the editor tells Back from a click. */
export function hasPendingNavigation(): boolean {
  return pending !== null;
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

/** The guard's prompt said Keep editing, or the editor went away unanswered: drop it. */
export function cancelNavigation(): void {
  pending = null;
}
