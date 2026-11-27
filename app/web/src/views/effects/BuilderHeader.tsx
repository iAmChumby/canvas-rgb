"use client";

// The builder's page strip (#257 phase 4), moved out of StackBuilder.tsx unchanged: the back key,
// the stack's name and layer count, Unsaved, Revert and Save.

import { ArrowCounterClockwise, ArrowLeft, DotsThreeOutline, FilmSlate, FloppyDisk } from "@phosphor-icons/react";
import { Button, ButtonLink, IconButton } from "@/components/Button";
import { Popover } from "@/components/Popover";
import { BarContextMirror, PageActions, PageTitle, useContextInBody, useStripCompact } from "@/shell/PageHeader";
import { useRef, useState, type RefObject } from "react";

export function BuilderHeader({
  name,
  count,
  narrow,
  dirty,
  saveState,
  canSave,
  groupHref,
  groupName,
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
  /** The Scenes page's open group, when the builder was opened from it. */
  groupHref?: string | null;
  groupName?: string | null;
  backRef: RefObject<HTMLSpanElement | null>;
  onBack: () => void;
  onRevert: () => void;
  onSave: () => void;
}) {
  // B10: when the bar drops the back and revert keys they live in the one overflow menu.
  const stripCompact = useStripCompact();
  const contextInBody = useContextInBody();
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLSpanElement>(null);
  return (
    <>
      <PageTitle
        title={name}
        userTitle
        context={
          <span className="rd-stacks-context">
            Stack · {count} {count === 1 ? "layer" : "layers"}
          </span>
        }
        leading={
          <span ref={backRef} className="rd-stacks-leading">
            <IconButton label="Back to the effects" size="sm" icon={<ArrowLeft size={16} />} onClick={onBack} />
            {narrow && groupHref && (
              <ButtonLink
                href={groupHref}
                variant="ghost"
                size="sm"
                icon={<FilmSlate size={16} />}
                label={groupName ? `Back to ${groupName}` : "Back to the scene"}
              >
                <span className="rd-btn-text">{groupName ? `Back to ${groupName}` : "Back to the scene"}</span>
              </ButtonLink>
            )}
          </span>
        }
      />
      {contextInBody && (
        <BarContextMirror className="fig">Stack · {count} {count === 1 ? "layer" : "layers"}</BarContextMirror>
      )}
      <PageActions>
        {!narrow && groupHref && (
          <ButtonLink
            className="rd-secondary"
            href={groupHref}
            variant="ghost"
            size="sm"
            icon={<ArrowLeft size={14} />}
            label={groupName ? `Back to ${groupName}` : "Back to the scene"}
          >
            <span className="rd-btn-text">{groupName ? `Back to ${groupName}` : "Back to the scene"}</span>
          </ButtonLink>
        )}
        {/* R7: Save is always in the bar and is disabled when there is nothing to save. R2: the
            Unsaved words become a dot on Save, with those words kept in its accessible name. */}
        {!narrow && (
          <Button
            className="rd-secondary"
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
          data-bar-primary=""
          data-dirty={dirty ? "true" : undefined}
          size="sm"
          variant="primary"
          icon={<FloppyDisk size={14} />}
          aria-label={dirty ? "Save the stack, unsaved changes" : "Save the stack"}
          title="Save the stack"
          busy={saveState === "saving"}
          disabled={!canSave}
          onClick={onSave}
        >
          <span className="rd-btn-text" data-short="Save">Save</span>
          {dirty && <span className="sr-only">Unsaved</span>}
        </Button>
        {stripCompact && (
          <>
            <span ref={moreRef}>
              <IconButton label="More page actions" size="sm" icon={<DotsThreeOutline size={16} />} onClick={() => setMoreOpen((v) => !v)} />
            </span>
            <Popover anchor={moreRef} open={moreOpen} onClose={() => setMoreOpen(false)} label="More page actions">
              {groupHref && (
                <ButtonLink href={groupHref} variant="ghost" size="sm" icon={<ArrowLeft size={14} />} label={groupName ? `Back to ${groupName}` : "Back to the scene"}>
                  <span className="rd-btn-text">{groupName ? `Back to ${groupName}` : "Back to the scene"}</span>
                </ButtonLink>
              )}
              <Button variant="ghost" size="sm" icon={<ArrowCounterClockwise size={14} />} disabled={!dirty || saveState === "saving"} onClick={onRevert}>
                Revert
              </Button>
            </Popover>
          </>
        )}
      </PageActions>
    </>
  );
}
