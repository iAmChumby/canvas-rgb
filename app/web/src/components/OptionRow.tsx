"use client";

import type { ReactNode } from "react";

/**
 * One selectable row in a popover list: a thumbnail, a name and one line of description, with an
 * optional mark at the end. It is the component layer's row for a `listbox` (#256): the mask
 * picker draws its effects with it, so no caller hand-rolls a `<button role="option">`.
 */
export function OptionRow({
  selected,
  disabled,
  title,
  onSelect,
  leading,
  name,
  description,
  trailing,
}: {
  selected?: boolean;
  disabled?: boolean;
  title?: string;
  onSelect: () => void;
  /** The row's picture: a `Thumb`, usually. */
  leading: ReactNode;
  name: ReactNode;
  description: ReactNode;
  /** A mark after the text: a Chip, usually. */
  trailing?: ReactNode;
}) {
  return (
    <button
      type="button"
      className="rd-add-row"
      role="option"
      aria-selected={selected}
      disabled={disabled}
      title={title}
      onClick={onSelect}
    >
      {leading}
      <span className="rd-add-text">
        <span className="rd-add-name">{name}</span>
        <span className="rd-add-desc">{description}</span>
      </span>
      {trailing}
    </button>
  );
}
