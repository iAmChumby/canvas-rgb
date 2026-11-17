"use client";

// The prompt that asks before unsaved work is dropped (#474). The stack builder and the pattern
// editor use the same one, with the same words: Save keeps the work and goes on, Discard throws
// it away and leaves, Keep editing stays.

import type { RefObject } from "react";
import { Button } from "@/components/Button";

export function LeavePrompt({
  promptRef,
  saveState,
  subject = "stack",
  onKeepEditing,
  onDiscard,
  onSave,
}: {
  promptRef: RefObject<HTMLDivElement | null>;
  saveState: "idle" | "saving";
  /** What the work is: a stack, a pattern. The sentence names it. */
  subject?: string;
  onKeepEditing: () => void;
  onDiscard: () => void;
  onSave: () => void;
}) {
  return (
    <div className="rd-scrim" role="presentation">
      <div
        className="rd-picker rd-confirm"
        role="alertdialog"
        aria-modal="true"
        aria-label="Unsaved changes"
        ref={promptRef}
      >
        <div className="rd-picker-head">
          <div>
            <h2>Save your changes?</h2>
            <p>The {subject} has changes that are not saved. Leaving now loses them.</p>
          </div>
        </div>
        <div className="rd-picker-foot">
          <Button size="sm" variant="ghost" onClick={onKeepEditing}>
            Keep editing
          </Button>
          <Button size="sm" variant="danger" disabled={saveState === "saving"} onClick={onDiscard}>
            Discard
          </Button>
          <Button size="sm" variant="primary" busy={saveState === "saving"} onClick={onSave}>
            Save
          </Button>
        </div>
      </div>
    </div>
  );
}
