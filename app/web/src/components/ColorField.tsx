"use client";

import type { CSSProperties } from "react";
import { BLACK_HEX, isHex } from "@/lib/color";

/** A color control: a swatch that opens the native picker, with the hex value in mono. */
export function ColorField({
  value,
  label,
  onChange,
  onCommit,
}: {
  value: string;
  label: string;
  onChange: (hex: string) => void;
  onCommit?: (hex: string) => void;
}) {
  const hex = isHex(value) ? value.toLowerCase() : BLACK_HEX;
  return (
    <label className="color">
      <span className="color-swatch" style={{ "--c": hex } as CSSProperties}>
        <input
          type="color"
          aria-label={label}
          value={hex}
          onChange={(e) => onChange(e.target.value)}
          onBlur={(e) => onCommit?.(e.target.value)}
        />
      </span>
      <span className="color-hex">{hex}</span>
    </label>
  );
}
