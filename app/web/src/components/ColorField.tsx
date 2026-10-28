"use client";

import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Popover } from "@/components/Popover";
import { ColorPicker, type ColorSwatch } from "@/components/ColorPicker";
import { BLACK_HEX, isHex } from "@/lib/color";

/**
 * A colour control: a swatch that opens the editor's own picker, with the hex value in mono
 * beside it.
 *
 * `onChange` runs while the colour is being chosen; `onCommit` runs when the picker closes,
 * with the colour that was chosen.
 */
export function ColorField({
  value,
  label,
  swatches,
  disabled,
  onChange,
  onCommit,
}: {
  value: string;
  label: string;
  /** Other colours of the same effect, offered as one-click choices inside the picker. */
  swatches?: ColorSwatch[];
  disabled?: boolean;
  onChange: (hex: string) => void;
  onCommit?: (hex: string) => void;
}) {
  const hex = isHex(value) ? value.toLowerCase() : BLACK_HEX;
  const [open, setOpen] = useState(false);
  const swatchRef = useRef<HTMLButtonElement>(null);
  const lastRef = useRef(hex);
  const escapedRef = useRef(false);

  // The colour that is on screen now: what the picker commits when it closes.
  useEffect(() => {
    lastRef.current = hex;
  }, [hex]);

  // Escape is caught before the popover sees it, so the swatch can take the focus back.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") escapedRef.current = true;
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [open]);

  const close = () => {
    setOpen(false);
    if (escapedRef.current) {
      escapedRef.current = false;
      swatchRef.current?.focus();
    }
    onCommit?.(lastRef.current);
  };

  return (
    <span className="color">
      <button
        ref={swatchRef}
        type="button"
        className="color-swatch"
        style={{ "--c": hex } as CSSProperties}
        aria-label={label}
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => (open ? close() : setOpen(true))}
      />
      <span className="color-hex">{hex}</span>
      <Popover anchor={swatchRef} open={open} onClose={close} label={label} className="picker-pop">
        <ColorPicker label={label} value={hex} swatches={swatches} onChange={onChange} />
      </Popover>
    </span>
  );
}
