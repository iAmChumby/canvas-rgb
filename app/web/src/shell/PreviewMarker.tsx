"use client";

import { Eye } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { leavePreview } from "@/lib/previewMode";
import s from "@/shell/previewMarker.module.css";

/**
 * The mode marker of a preview (#579), in the shell's top row. It is in every page's markup and
 * the stylesheet shows it only under `html[data-preview="on"]`, which a script sets before the
 * first paint: the row is there from the first frame and nothing moves.
 */
export function PreviewMarker() {
  return (
    <div className={s.marker} data-testid="preview-marker">
      <Eye size={18} weight="bold" aria-hidden="true" />
      <p className={s.words}>Preview: sample data, not your PC</p>
      <Button size="sm" className={s.leave} onClick={leavePreview}>
        Leave preview
      </Button>
    </div>
  );
}
