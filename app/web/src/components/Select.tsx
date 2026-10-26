"use client";

import { CaretDown } from "@phosphor-icons/react";

export interface SelectOption {
  value: string;
  label: string;
}

/** A native select, styled. Used for choices with more than four options. */
export function Select({
  value,
  label,
  options,
  onChange,
}: {
  value: string;
  label: string;
  options: SelectOption[];
  onChange: (v: string) => void;
}) {
  return (
    <span className="select">
      <select aria-label={label} value={value} onChange={(e) => onChange(e.target.value)}>
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <CaretDown className="select-caret" size={14} weight="bold" aria-hidden="true" />
    </span>
  );
}
