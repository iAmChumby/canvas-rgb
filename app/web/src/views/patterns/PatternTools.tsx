"use client";

// The editor's keys: paint, off, pick and fill all on the left, undo and redo beside them, and the
// colour field at the end. Flat hardware keys, the latched tool carrying a lit LED (the chip's own
// `aria-pressed`), and nothing that moves or resizes under the pointer.

import {
  ArrowClockwise,
  ArrowCounterClockwise,
  Eraser,
  Eyedropper,
  PaintBrush,
  PaintBucket,
} from "@phosphor-icons/react";
import { ColorField } from "@/components/ColorField";
import { IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import type { Tool } from "@/views/patterns/usePatternEditor";
import s from "@/views/patterns/patterns.module.css";

export function PatternTools({
  tool,
  colour,
  canUndo,
  canRedo,
  onTool,
  onColour,
  onFillAll,
  onUndo,
  onRedo,
}: {
  tool: Tool;
  colour: string;
  canUndo: boolean;
  canRedo: boolean;
  onTool: (tool: Tool) => void;
  onColour: (colour: string) => void;
  onFillAll: () => void;
  onUndo: () => void;
  onRedo: () => void;
}) {
  return (
    <div className={s.tools}>
      <div className={s.toolKeys} role="group" aria-label="Tools">
        <Chip icon={<PaintBrush size={14} weight="bold" aria-hidden="true" />} pressed={tool === "paint"} onClick={() => onTool("paint")}>
          Paint
        </Chip>
        <Chip icon={<Eraser size={14} weight="bold" aria-hidden="true" />} pressed={tool === "off"} onClick={() => onTool("off")}>
          Off
        </Chip>
        <Chip icon={<Eyedropper size={14} weight="bold" aria-hidden="true" />} pressed={tool === "pick"} onClick={() => onTool("pick")}>
          Pick
        </Chip>
        <Chip
          icon={<PaintBucket size={14} weight="bold" aria-hidden="true" />}
          pressed={tool === "fill"}
          onClick={() => {
            onTool("fill");
            onFillAll();
          }}
        >
          Fill all
        </Chip>
      </div>
      <div className={s.historyKeys}>
        <IconButton
          label="Undo"
          icon={<ArrowCounterClockwise size={16} weight="bold" aria-hidden="true" />}
          disabled={!canUndo}
          onClick={onUndo}
        />
        <IconButton
          label="Redo"
          icon={<ArrowClockwise size={16} weight="bold" aria-hidden="true" />}
          disabled={!canRedo}
          onClick={onRedo}
        />
      </div>
      <ColorField value={colour} label="Colour" onChange={onColour} />
    </div>
  );
}
