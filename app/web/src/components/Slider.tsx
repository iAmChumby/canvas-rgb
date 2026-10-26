"use client";

import type { CSSProperties, KeyboardEvent, PointerEvent } from "react";

/**
 * A range control with a live fill. `onChange` runs while dragging; `onCommit` runs on release
 * (and on each arrow-key step), so a value that reaches the daemon is committed once.
 * `floor` draws a shaded band from the minimum up to that value.
 */
export function Slider({
  value,
  min,
  max,
  step = 1,
  label,
  floor,
  disabled,
  onChange,
  onCommit,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  label: string;
  floor?: number;
  disabled?: boolean;
  onChange: (v: number) => void;
  onCommit?: (v: number) => void;
}) {
  const span = max - min || 1;
  const p = Math.min(1, Math.max(0, (value - min) / span));
  const f = floor === undefined ? 0 : Math.min(1, Math.max(0, (floor - min) / span));
  const style = { "--p": String(p), "--f": String(f) } as CSSProperties;

  const release = (e: PointerEvent<HTMLInputElement> | KeyboardEvent<HTMLInputElement>) => {
    onCommit?.(Number(e.currentTarget.value));
  };

  return (
    <div className="slider" style={style}>
      <div className="slider-track" />
      {floor !== undefined && <div className="slider-floor" />}
      <div className="slider-fill" />
      <input
        type="range"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        onPointerUp={release}
        onKeyUp={(e) => {
          if (e.key.startsWith("Arrow") || e.key === "Home" || e.key === "End") release(e);
        }}
      />
    </div>
  );
}
