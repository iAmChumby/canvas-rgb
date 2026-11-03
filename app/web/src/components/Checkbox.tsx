"use client";

import { Check, Minus } from "@phosphor-icons/react";

/**
 * A tick box with an accessible name. A button with role checkbox, so it sits in the component
 * layer with the switch. `blocked` keeps it in the tab order but refuses the tick, so the reason
 * in `title` can still be read; `mixed` is the header box of a partly ticked column.
 */
export function Checkbox({
  checked,
  mixed,
  blocked,
  label,
  title,
  onToggle,
}: {
  checked: boolean;
  mixed?: boolean;
  blocked?: boolean;
  label: string;
  title?: string;
  onToggle: (shift: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      className="tick"
      aria-checked={mixed ? "mixed" : checked}
      aria-disabled={blocked || undefined}
      aria-label={label}
      title={title}
      onClick={(e) => {
        e.stopPropagation();
        if (!blocked) onToggle(e.shiftKey);
      }}
      onKeyDown={(e) => {
        // Space on the box ticks it; the row around it must not hear it a second time.
        if (e.key === " ") e.stopPropagation();
      }}
    >
      <span className="tick-box" aria-hidden="true">
        {mixed || (blocked && !checked) ? <Minus size={12} weight="bold" /> : checked ? <Check size={12} weight="bold" /> : null}
      </span>
    </button>
  );
}
