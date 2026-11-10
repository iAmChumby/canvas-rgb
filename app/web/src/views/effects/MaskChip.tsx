"use client";

// The mask on a layer's row (#257 phase 3): its own thumbnail, which opens the mask picker, and
// a remove "x". Backspace on the focused chip removes the mask, as it does on the inspector's
// Remove key. The chip sits beside the layer's thumbnail in `StackBuilder`'s row.

import { X } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";
import { Thumb } from "@/components/Thumb";

export function MaskChip({
  layerName,
  effect,
  label,
  onChange,
  onRemove,
}: {
  layerName: string;
  /** The mask effect's id: its thumbnail and its name. */
  effect: string;
  label: string;
  /** A press on the thumb: pick the layer and open the mask picker under it. */
  onChange: (anchor: HTMLElement) => void;
  onRemove: () => void;
}) {
  return (
    <span
      className="rd-mask-chip"
      data-mask-chip="true"
      // The chip owns its presses: a click on either part never also selects the row.
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        if (event.key !== "Backspace" && event.key !== "Delete") return;
        event.preventDefault();
        event.stopPropagation();
        onRemove();
      }}
    >
      <button
        type="button"
        className="rd-mask-chip-thumb"
        aria-label={`Change the mask of ${layerName}`}
        title={`Mask: ${label}. Click to change it; Backspace removes it.`}
        onClick={(event) => onChange(event.currentTarget)}
      >
        <Thumb id={effect} size="sm" label={`Mask: ${label}`} />
      </button>
      <IconButton
        size="sm"
        className="rd-mask-chip-x"
        label={`Remove the mask from ${layerName}`}
        icon={<X size={12} />}
        onClick={onRemove}
      />
    </span>
  );
}
