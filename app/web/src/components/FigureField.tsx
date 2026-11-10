"use client";

// A figure that opens its field (#257 phase 4, spec §3): the number beside a slider is a readout,
// and a click or tap turns it into the number's own input. Enter commits (one change, one undo
// step), Escape puts the value back and leaves, blur commits as a number field does.

import { useRef, useState, type KeyboardEvent } from "react";
import { clamp } from "@/lib/format";

export function FigureField({
  value,
  min,
  max,
  step = 1,
  decimals = 0,
  label,
  unit,
  disabled,
  onCommit,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  decimals?: number;
  label: string;
  unit?: string;
  disabled?: boolean;
  onCommit: (value: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  /** One field one commit: Enter and the blur that follows it must not both write. */
  const settled = useRef(false);
  const shown = value.toFixed(decimals);

  const openField = () => {
    settled.current = false;
    setDraft(shown);
    setOpen(true);
  };

  const commit = () => {
    if (settled.current) return;
    settled.current = true;
    const parsed = Number(draft);
    setOpen(false);
    if (Number.isNaN(parsed)) return;
    const next = Number(clamp(parsed, min, max).toFixed(decimals));
    if (next !== value) onCommit(next);
  };

  const cancel = () => {
    settled.current = true;
    setOpen(false);
    setDraft("");
  };

  const onKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      commit();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      cancel();
      return;
    }
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      const direction = event.key === "ArrowUp" ? 1 : -1;
      const next = Number(clamp(Number((value + direction * step).toFixed(decimals)), min, max).toFixed(decimals));
      setDraft(next.toFixed(decimals));
      settled.current = true;
      onCommit(next);
      setOpen(false);
    }
  };

  if (!open) {
    return (
      <button
        type="button"
        className="fig-field"
        aria-label={`Edit ${label}`}
        title={`${label}: ${shown}${unit ?? ""}. Click to type a value.`}
        disabled={disabled}
        onClick={openField}
      >
        <span className="num fig">{shown}</span>
        {unit && <span className="unit">{unit}</span>}
      </button>
    );
  }

  return (
    <input
      type="text"
      inputMode="decimal"
      aria-label={`${label} value`}
      className="num fig-field-input"
      value={draft}
      disabled={disabled}
      autoFocus
      onChange={(event) => setDraft(event.target.value)}
      onFocus={(event) => event.currentTarget.select()}
      onBlur={commit}
      onKeyDown={onKey}
    />
  );
}



