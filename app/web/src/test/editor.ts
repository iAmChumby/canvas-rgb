// Shared helpers for waiting on the CodeMirror editor the effects tests drive (#120).
//
// The editor is loaded with `next/dynamic` (`views/effects/EffectCode.tsx`), and CodeMirror
// builds `.cm-content` inside a `useEffect` (`components/CodeEditor.tsx`: `new EditorView`).
// The host div `data-testid="effect-code"` exists before that, so a test that finds the host
// and then reads `.cm-content` synchronously races the mount: under load `querySelector`
// returns null and `.focus()` throws `Cannot read properties of null (reading 'focus')`.
// Lint markers (`.cm-lint-marker-error`) come from a later effect still, so they need the same
// wait. Every test goes through here instead of querying `.cm-*` itself.

import { waitFor } from "@testing-library/react";

/** Waits until CodeMirror has mounted and returns its `.cm-content` element. */
export async function editorContent(root: ParentNode = document): Promise<HTMLElement> {
  await waitFor(() => {
    if (!root.querySelector(".cm-content")) throw new Error("editor not mounted yet");
  });
  return root.querySelector(".cm-content") as HTMLElement;
}

/** Waits until the editor draws a lint error marker and returns it. */
export async function lintMarker(root: ParentNode = document): Promise<HTMLElement> {
  await waitFor(() => {
    if (!root.querySelector(".cm-lint-marker-error")) throw new Error("lint marker not yet drawn");
  });
  return root.querySelector(".cm-lint-marker-error") as HTMLElement;
}

/** The source as the editor holds it right now (the `.cm-line` texts, joined), once it has mounted.
 *  A one-shot read: to wait for a change, wrap the comparison in `waitFor`. */
export async function editorSource(root: ParentNode = document): Promise<string> {
  const content = await editorContent(root);
  return Array.from(content.querySelectorAll(".cm-line"))
    .map((line) => line.textContent ?? "")
    .join("\n");
}
