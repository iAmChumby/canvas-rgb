"use client";

// The editor's keys: paint, off, pick and fill all on the left, undo and redo beside them, and the
// colour field at the end. Flat hardware keys, the latched tool carrying a lit LED (the chip's own
// `aria-pressed`), and nothing that moves or resizes under the pointer.

import {
  ArrowClockwise,
  ArrowCounterClockwise,
  Eraser,
  Eyedropper,
  Gradient,
  Minus,
  PaintBrush,
  PaintBucket,
  Plus,
} from "@phosphor-icons/react";
import { ColorField } from "@/components/ColorField";
import { IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { MAX_STOPS, MIN_STOPS } from "@/lib/gradient";
import type { GradientState, Tool } from "@/views/patterns/usePatternEditor";
import s from "@/views/patterns/patterns.module.css";

export function PatternTools({
  tool,
  colour,
  canUndo,
  canRedo,
  gradient,
  onTool,
  onColour,
  onFillAll,
  onUndo,
  onRedo,
  onWrap,
  onAddStop,
  onRemoveStop,
}: {
  tool: Tool;
  colour: string;
  canUndo: boolean;
  canRedo: boolean;
  /** The Gradient tool's stops, once it has been picked. */
  gradient: GradientState | null;
  onTool: (tool: Tool) => void;
  onColour: (colour: string) => void;
  onFillAll: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onWrap: (wrap: boolean) => void;
  onAddStop: () => void;
  onRemoveStop: () => void;
}) {
  const stops = gradient?.stops.length ?? 0;
  const live = tool === "gradient" && gradient !== null;
  return (
    <div className={s.tools}>
      <div className={s.toolKeys} role="group" aria-label="Tools">
        <Chip icon={<PaintBrush size={14} weight="bold" aria-hidden="true" />} label="Paint" pressed={tool === "paint"} onClick={() => onTool("paint")}>
          <span className={s.toolWord}>Paint</span>
        </Chip>
        <Chip icon={<Eraser size={14} weight="bold" aria-hidden="true" />} label="Off" pressed={tool === "off"} onClick={() => onTool("off")}>
          <span className={s.toolWord}>Off</span>
        </Chip>
        <Chip icon={<Eyedropper size={14} weight="bold" aria-hidden="true" />} label="Pick" pressed={tool === "pick"} onClick={() => onTool("pick")}>
          <span className={s.toolWord}>Pick</span>
        </Chip>
        <Chip
          icon={<PaintBucket size={14} weight="bold" aria-hidden="true" />}
          label="Fill all"
          pressed={tool === "fill"}
          onClick={() => {
            onTool("fill");
            onFillAll();
          }}
        >
          <span className={s.toolWord}>Fill all</span>
        </Chip>
        <Chip icon={<Gradient size={14} weight="bold" aria-hidden="true" />} label="Gradient" pressed={tool === "gradient"} onClick={() => onTool("gradient")}>
          <span className={s.toolWord}>Gradient</span>
        </Chip>
      </div>
      {/* The Gradient tool's own keys keep their room when the tool is not in the hand (dimmed,
          never removed), so picking it moves nothing. */}
      <div className={s.gradientKeys} role="group" aria-label="Gradient" data-live={live ? "true" : "false"}>
        <Chip pressed={live && gradient.wrap} disabled={!live} onClick={() => gradient && onWrap(!gradient.wrap)} title="The last stop meets the first across the seam">
          Wrap
        </Chip>
        <IconButton
          label="Add stop"
          icon={<Plus size={16} weight="bold" aria-hidden="true" />}
          disabled={!live || stops >= MAX_STOPS}
          onClick={onAddStop}
        />
        <IconButton
          label="Remove stop"
          icon={<Minus size={16} weight="bold" aria-hidden="true" />}
          disabled={!live || stops <= MIN_STOPS}
          onClick={onRemoveStop}
        />
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
      <ColorField
        value={colour}
        label={live ? `Stop ${(gradient?.selected ?? 0) + 1} colour` : "Colour"}
        onChange={onColour}
      />
    </div>
  );
}
