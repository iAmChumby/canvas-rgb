"use client";

// The builder's page strip (#257 phase 4), moved out of StackBuilder.tsx unchanged: the back key,
// the stack's name and layer count, Unsaved, Revert and Save.

import { ArrowCounterClockwise, ArrowLeft, FloppyDisk } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { PageActions, PageTitle } from "@/shell/PageHeader";
import type { RefObject } from "react";

export function BuilderHeader({
  name,
  count,
  narrow,
  dirty,
  saveState,
  canSave,
  backRef,
  onBack,
  onRevert,
  onSave,
}: {
  name: string;
  count: number;
  narrow: boolean;
  dirty: boolean;
  saveState: "idle" | "saving";
  canSave: boolean;
  backRef: RefObject<HTMLSpanElement | null>;
  onBack: () => void;
  onRevert: () => void;
  onSave: () => void;
}) {
  return (
    <>
      <PageTitle
        title={name}
        context={
          <span className="rd-stacks-context">
            Stack · {count} {count === 1 ? "layer" : "layers"}
          </span>
        }
        leading={
          <span ref={backRef}>
            <IconButton label="Back to the effects" size="sm" icon={<ArrowLeft size={16} />} onClick={onBack} />
          </span>
        }
      />
      <PageActions>
        {/* On a phone the strip carries one key: Save, as an icon with its name (D95.5.7). The
            word Unsaved and Revert are desktop keys: the phone's way out asks Save or Discard. */}
        {!narrow && dirty && <span className="mute rd-unsaved">Unsaved</span>}
        {!narrow && (
          <Button
            size="sm"
            variant="ghost"
            icon={<ArrowCounterClockwise size={14} />}
            disabled={!dirty || saveState === "saving"}
            onClick={onRevert}
          >
            Revert
          </Button>
        )}
        <Button
          className="rd-stack-strip-save"
          size="sm"
          variant="primary"
          icon={<FloppyDisk size={14} />}
          aria-label={narrow ? "Save" : undefined}
          title={narrow ? "Save the stack" : undefined}
          busy={saveState === "saving"}
          disabled={!canSave}
          onClick={onSave}
        >
          <span className="rd-btn-text">Save</span>
        </Button>
      </PageActions>
    </>
  );
}
