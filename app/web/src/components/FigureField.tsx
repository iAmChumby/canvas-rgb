"use client";

// A figure that opens its field (#257 phase 4, spec §3): the number beside a slider is a readout,
// and a click or tap turns it into the number's own input. Enter commits (one change, one undo
// step), Escape puts the value back and leaves, blur commits as a number field does. A number the
// field cannot take (outside its range, or not a number) is never clamped or dropped quietly: the
// field stays open and says so, as NumberField's `num-invalid` does. A blank field cancels. Arrow
// keys step the value and keep the field open, one commit per press, as NumberField does, so a held
// arrow is as many commits as the caller's own undo run folds into one step.

import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
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
  /** The field closed by a key: focus goes back to the readout, where the keyboard was. */
  const refocus = useRef(false);
  const readout = useRef<HTMLButtonElement>(null);
  const hintId = useId();
  const shown = value.toFixed(decimals);
  const spoken = `${shown}${unit ?? ""}`;
  const range = `${min.toFixed(decimals)} to ${max.toFixed(decimals)}`;

  const blank = draft.trim() === "";
  const parsed = blank ? null : Number(draft);
  const invalid = parsed !== null && (Number.isNaN(parsed) || parsed < min || parsed > max);

  useEffect(() => {
    if (open || !refocus.current) return;
    refocus.current = false;
    readout.current?.focus();
  }, [open]);

  const openField = () => {
    settled.current = false;
    setDraft(shown);
    setOpen(true);
  };

  const close = (byKey: boolean) => {
    settled.current = true;
    refocus.current = byKey;
    setOpen(false);
    setDraft("");
  };

  const commit = (byKey: boolean) => {
    if (settled.current) return;
    // A blank field is a cancel: Number("") is 0, which would set the minimum.
    if (parsed === null) {
      close(byKey);
      return;
    }
    // The field stays open and marked: the number was not taken, and the person is told.
    if (invalid) return;
    const next = Number(parsed.toFixed(decimals));
    close(byKey);
    if (next !== value) onCommit(next);
  };

  const onKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      commit(true);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close(true);
      return;
    }
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      const direction = event.key === "ArrowUp" ? 1 : -1;
      const next = Number(clamp(Number((value + direction * step).toFixed(decimals)), min, max).toFixed(decimals));
      setDraft(next.toFixed(decimals));
      if (next !== value) onCommit(next);
    }
  };

  if (!open) {
    return (
      <button
        ref={readout}
        type="button"
        className="fig-field"
        aria-label={`Edit ${label}, ${spoken}`}
        title={`${label}: ${spoken}. Click to type a value.`}
        disabled={disabled}
        onClick={openField}
      >
        <span className="num fig">{shown}</span>
        {unit && <span className="unit">{unit}</span>}
      </button>
    );
  }

  return (
    <>
      <input
        type="text"
        inputMode="decimal"
        aria-label={`${label} value`}
        aria-invalid={invalid}
        aria-describedby={invalid ? hintId : undefined}
        title={invalid ? `${label} takes a number from ${range}.` : undefined}
        className={invalid ? "num fig-field-input num-invalid" : "num fig-field-input"}
        value={draft}
        disabled={disabled}
        autoFocus
        onChange={(event) => setDraft(event.target.value)}
        onFocus={(event) => event.currentTarget.select()}
        onBlur={() => commit(false)}
        onKeyDown={onKey}
      />
      {invalid && (
        <span className="sr-only" id={hintId} role="alert">
          {label} takes a number from {range}.
        </span>
      )}
    </>
  );
}
