"use client";

// The builder's page-level notes (#257 phase 4), moved out of StackBuilder.tsx unchanged: the
// draft's own note and the taken-name prompt.

import { Warning } from "@phosphor-icons/react";
import { Button } from "@/components/Button";

export function BuilderNotices({
  draftNote,
  clash,
  saving,
  onReplace,
  onCancelClash,
}: {
  draftNote: string | null;
  clash: string | null;
  saving: boolean;
  onReplace: () => void;
  onCancelClash: () => void;
}) {
  return (
    <>
      {draftNote && (
        <p className="eff-note" role="status">
          <Warning size={14} aria-hidden="true" /> {draftNote}
        </p>
      )}

      {clash && (
        <div className="import-note" data-tone="ask" role="alert">
          <span>
            <span className="eff-clash-tag">name exists</span> A stack named <span className="fig">{clash}</span>{" "}
            already exists. Replace it with this stack? Its layers are overwritten; its presets and favourite stay. If
            it is running, the lights change to this stack.
          </span>
          <div className="row-wrap">
            <Button size="sm" variant="danger" busy={saving} onClick={onReplace}>
              Replace
            </Button>
            <Button size="sm" variant="ghost" onClick={onCancelClash}>
              Cancel
            </Button>
          </div>
        </div>
      )}
    </>
  );
}
