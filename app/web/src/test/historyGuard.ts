// Shared helpers for waiting on the history guards the builder and the editor install (#136).
//
// The stack builder pushes its guard entry and adds its `popstate` listener in a `useEffect`
// keyed on `dirty` (`views/effects/StackBuilder.tsx` ~857-889), which runs after the commit
// that renders the "Unsaved" text. A test that calls `window.history.back()` right after
// seeing that text therefore races the effect: under load Back can land before the guard
// exists, and is then a real navigation with no prompt. Waiting for the pushed state proves
// the listener is in place. Same shape in the effect editor
// (`views/effects/EffectEditor.tsx` ~160-193), which pushes `{ canvasRgbEditor: "guard" }`.

import { waitFor } from "@testing-library/react";

/** Waits until the stack builder's history guard is installed. */
export async function waitForStackGuard(): Promise<void> {
  await waitFor(() => {
    if (window.history.state?.canvasRgbStackGuard !== true)
      throw new Error("stack guard not installed yet");
  });
}

/** Waits until the effect editor's history guard is installed. */
export async function waitForEditorGuard(): Promise<void> {
  await waitFor(() => {
    if (window.history.state?.canvasRgbEditor !== "guard")
      throw new Error("editor guard not installed yet");
  });
}

/** Waits until the pattern editor's history guard is installed. */
export async function waitForPatternGuard(): Promise<void> {
  await waitFor(() => {
    if (window.history.state?.canvasRgbPatternGuard !== true)
      throw new Error("pattern guard not installed yet");
  });
}

/**
 * Leaves the history the next test starts on plain: no guard state on the entry the document is
 * on, and no history move still on its way.
 *
 * The move matters as much as the state. A builder that undoes to clean and edits again pushes a
 * second guard entry while the first one's Back is still queued, so a Back is in flight when the
 * test ends. Clearing the current entry's state does not stop it: it lands later, on the first
 * guard's copy behind, whose state still carries the guard's key, and the next test's builder reads
 * that as a spent entry and refuses to open. jsdom cancels every queued traversal when a page is
 * pushed (`History-impl.js` `pushState`), so the plain entry is pushed over the cleared one.
 *
 * Also: an editor that unmounts with its guard's entry up takes that entry back with a Back
 * (`strictEntry`), and a test's clean-up unmounts a dirty editor. Call this before the clean-up.
 */
export function plainEntry(): void {
  window.history.replaceState({}, "", window.location.href);
  window.history.pushState({}, "", window.location.href);
}

/**
 * Resolves once every history move jsdom has queued has run. A traversal is two queued tasks, the
 * second queued by the first, each one `setImmediate` hop: three hops of the event loop outlast any
 * move queued before the call. No timer, so no wall-clock wait.
 */
export async function historySettled(): Promise<void> {
  for (let hop = 0; hop < 3; hop += 1) await new Promise<void>((resolve) => setImmediate(resolve));
}
