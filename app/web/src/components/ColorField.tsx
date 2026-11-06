"use client";

import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Popover } from "@/components/Popover";
import { ColorPicker, type ColorSwatch } from "@/components/ColorPicker";
import { motionOffNow, readCssMs } from "@/lib/motion";
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

  // The wipe (issue #187, rule 9): `base` is the colour beneath the swatch,
  // and a wipe overlay carries the incoming colour over it. The overlay is
  // mounted only from the first real change after mount (first mount is
  // static), and it is keyed by a wipe generation, not by the colour: while
  // a wipe runs, further changes update its target without remounting it, so
  // dragging in the picker does not replay the reveal on every intermediate
  // hex. When the wipe ends the incoming colour is promoted to the base.
  const [base, setBase] = useState(hex);
  const [wiping, setWiping] = useState(false);
  const [wipeId, setWipeId] = useState(0);
  useEffect(() => {
    if (hex === base) {
      if (wiping) {
        setWiping(false);
      }
      return;
    }
    if (!wiping) {
      setWipeId((id) => id + 1);
      setWiping(true);
    }
    const ms = motionOffNow() ? 0 : readCssMs("--dur", 180);
    const t = setTimeout(() => {
      setBase(hex);
      setWiping(false);
    }, ms);
    return () => clearTimeout(t);
    // `wiping` above only gates starting a new wipe generation; re-running
    // the timer on a further change is the point (the target moves, the
    // promotion waits for the newest colour).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hex]);

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
        style={{ "--c": base } as CSSProperties}
        aria-label={label}
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => (open ? close() : setOpen(true))}
      >
        {/* The new colour wipes in over the old one (issue #187, rule 9): the
            overlay is keyed by the wipe generation, so a change replays the
            mask reveal from the swatch's centre — the base beneath is the
            previous colour until the wipe ends. */}
        {wiping && <span key={wipeId} className="color-wipe" style={{ "--c": hex } as CSSProperties} data-hex={hex} aria-hidden="true" />}
      </button>
      <span className="color-hex">{hex}</span>
      <Popover anchor={swatchRef} open={open} onClose={close} label={label} className="picker-pop">
        <ColorPicker label={label} value={hex} swatches={swatches} onChange={onChange} />
      </Popover>
    </span>
  );
}
