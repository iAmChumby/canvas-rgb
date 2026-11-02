"use client";

import dynamic from "next/dynamic";

/**
 * CodeMirror 6, reached through its own dynamic import so it is a separate chunk: the Effects
 * page loads no editing code until an effect is opened for editing. The loading line holds the
 * pane's height, so the box does not jump when the editor arrives.
 */
export const EffectCode = dynamic(() => import("@/components/CodeEditor"), {
  ssr: false,
  loading: () => (
    <div className="cm-loading" role="status">
      Loading the editor&hellip;
    </div>
  ),
});
