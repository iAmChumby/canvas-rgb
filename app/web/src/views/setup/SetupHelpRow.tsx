"use client";

import { Eye, ListChecks } from "@phosphor-icons/react";
import { Button, ButtonLink } from "@/components/Button";
import { Panel } from "@/components/Panel";
import { enterPreview, previewActive } from "@/lib/previewMode";
import { offerSetupAgain } from "@/views/editor/flow";
import s from "@/views/setup/setup.module.css";

/** Help & About's way back to the checklist, and into the preview (#579). */
export function SetupHelpRow() {
  return (
    <Panel title="Setup checklist">
      <div className={s.helpKeys}>
        <ButtonLink href="/" icon={<ListChecks size={16} />} onClick={offerSetupAgain}>
          Open the checklist
        </ButtonLink>
        <Button icon={<Eye size={16} />} disabled={previewActive()} onClick={enterPreview}>
          See it without hardware
        </Button>
      </div>
    </Panel>
  );
}
