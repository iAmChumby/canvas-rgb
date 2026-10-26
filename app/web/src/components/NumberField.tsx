"use client";

import { useState, type KeyboardEvent } from "react";
import { clamp } from "@/lib/format";

/**
 * A mono number entry. The value is clamped to [min, max] when it is committed (Enter or blur),
 * so a value below a floor cannot be entered. Arrow keys step it.
 */
export function NumberField({
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
  onCommit: (v: number) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const shown = draft ?? value.toFixed(decimals);
  const parsed = draft === null ? null : Number(draft);
  const invalid = parsed !== null && (Number.isNaN(parsed) || parsed < min || parsed > max);

  const commit = () => {
    if (draft === null) return;
    const n = Number(draft);
    setDraft(null);
    if (Number.isNaN(n)) return;
    onCommit(Number(clamp(n, min, max).toFixed(decimals)));
  };

  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.currentTarget.blur();
      return;
    }
    if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      const dir = e.key === "ArrowUp" ? 1 : -1;
      const next = clamp(Number((value + dir * step).toFixed(decimals)), min, max);
      setDraft(null);
      onCommit(next);
    }
  };

  return (
    <span className="row-wrap" style={{ gap: 6 }}>
      <input
        type="text"
        inputMode="decimal"
        aria-label={label}
        aria-invalid={invalid}
        className={invalid ? "num num-invalid" : "num"}
        value={shown}
        disabled={disabled}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={onKey}
        onFocus={(e) => e.currentTarget.select()}
      />
      {unit && <span className="unit">{unit}</span>}
    </span>
  );
}

/** A single-line text entry, committed on Enter or blur. */
export function TextField({
  value,
  label,
  placeholder,
  onCommit,
}: {
  value: string;
  label: string;
  placeholder?: string;
  onCommit: (v: string) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft === null) return;
    const v = draft.trim();
    setDraft(null);
    if (v.length > 0 && v !== value) onCommit(v);
  };
  return (
    <input
      type="text"
      aria-label={label}
      placeholder={placeholder}
      className="text-in"
      value={draft ?? value}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
      }}
    />
  );
}
