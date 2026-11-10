"use client";

import dynamic from "next/dynamic";

import { Skeleton } from "@/components/Skeleton";
import { useLoadDelay } from "@/lib/loadDelay";

/**
 * What stands in the pane while the editor's chunk loads: a skeleton of the code box, shown only
 * after the load delay (a chunk that is already cached shows nothing). It holds the pane's height,
 * so the box does not jump when the editor arrives.
 */
function CodePending() {
  const shown = useLoadDelay(true);
  return (
    <div className="cm-loading" data-shown={shown ? "true" : "false"} role="status" aria-busy="true">
      <Skeleton className="cm-loading-box" width="100%" height="100%" />
      {/* The words a screen reader hears, as LoadFrame's `label` does: with the placeholder, so a
          chunk that is already cached is never announced. */}
      <span className="sr-only">{shown ? "Loading the editor" : ""}</span>
    </div>
  );
}

/**
 * CodeMirror 6, reached through its own dynamic import so it is a separate chunk: the Effects
 * page loads no editing code until an effect is opened for editing.
 */
export const EffectCode = dynamic(() => import("@/components/CodeEditor"), {
  ssr: false,
  loading: () => <CodePending />,
});
