"use client";

import { useState } from "react";

/** An on/off switch with an accessible name. */
export function Toggle({
  checked,
  label,
  disabled,
  onChange,
}: {
  checked: boolean;
  label: string;
  disabled?: boolean;
  onChange: (next: boolean) => void;
}) {
  // The knob smears only once it has been touched: a page that loads does not animate.
  const [touched, setTouched] = useState(false);
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className="switch"
      data-touched={touched ? "true" : undefined}
      disabled={disabled}
      onClick={() => {
        setTouched(true);
        onChange(!checked);
      }}
    />
  );
}
