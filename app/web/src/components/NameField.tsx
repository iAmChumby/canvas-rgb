"use client";

import { useEffect, useRef } from "react";

/**
 * A name entry for a thing the owner is about to create or rename. It is controlled: Enter asks
 * the parent to commit, Escape asks it to cancel, and it takes the focus (with the text
 * selected) when it appears.
 */
export function NameField({
  value,
  label,
  placeholder,
  maxLength = 40,
  disabled,
  invalid,
  describedBy,
  onChange,
  onEnter,
  onEscape,
}: {
  value: string;
  label: string;
  placeholder?: string;
  maxLength?: number;
  disabled?: boolean;
  invalid?: boolean;
  /** The id of the line that says why the entry is refused. */
  describedBy?: string;
  onChange: (v: string) => void;
  onEnter: () => void;
  onEscape: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  return (
    <input
      ref={ref}
      type="text"
      className="text-in name-field"
      aria-label={label}
      aria-invalid={invalid || undefined}
      aria-describedby={describedBy}
      placeholder={placeholder}
      maxLength={maxLength}
      disabled={disabled}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onEnter();
        } else if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onEscape();
        }
      }}
    />
  );
}
