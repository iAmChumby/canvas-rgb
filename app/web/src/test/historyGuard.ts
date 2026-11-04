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
