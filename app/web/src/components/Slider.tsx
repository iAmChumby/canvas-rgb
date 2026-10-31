"use client";

import { useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";

/**
 * The cap's width in css pixels. The cap is the only thing that travels, and it travels
 * between the two ends of the slot, so the fill always ends under the cap.
 */
const CAP = 14;

/** How many decimals a step needs: 1 -> 0, 0.5 -> 1, 0.1 -> 1. */
function decimalsOf(step: number): number {
  const s = String(step);
  const dot = s.indexOf(".");
  return dot < 0 ? 0 : Math.min(4, s.length - dot - 1);
}

/**
 * A fader: a slot with a cap that travels in it, the part behind the cap filled with the live
 * colour. It is not a native range input - the cap is drawn, so the travel, the fill and the
 * keyboard all follow the editor's own hardware layer.
 *
 * `onChange` runs while dragging; `onCommit` runs on release (and on each key step), so a value
 * that reaches the daemon is committed once.
 * `floor` draws a shaded band from the minimum up to that value.
 *
 * Keyboard: arrows step, PageUp and PageDown step ten times, Home and End go to the ends.
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
  const ref = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState(false);
  // While dragging, the cap and the fill follow the pointer itself, between steps; `value` (and
  // `onChange`) see only the snapped steps. Null when no drag is on: then `value` places the cap.
  const [dragFrac, setDragFrac] = useState<number | null>(null);
  // The last snapped value sent, so `onChange` runs once per step crossed, not once per move.
  const sent = useRef(value);
  const span = max - min || 1;
  const decimals = decimalsOf(step);
  const p = dragFrac ?? Math.min(1, Math.max(0, (value - min) / span));
  const f = floor === undefined ? 0 : Math.min(1, Math.max(0, (floor - min) / span));
  const style = { "--p": String(p), "--f": String(f) } as CSSProperties;

  /** A raw number put on the step grid and inside the range. */
  const quantise = (raw: number): number => {
    const stepped = min + Math.round((raw - min) / step) * step;
    return Number(Math.min(max, Math.max(min, stepped)).toFixed(decimals));
  };

  /** Where the pointer is along the travel, 0..1: the cap's centre is what follows the pointer. */
  const fracAt = (clientX: number): number | null => {
    const box = ref.current?.getBoundingClientRect();
    if (!box) return null;
    const travel = Math.max(0, box.width - CAP);
    return travel > 0 ? Math.min(1, Math.max(0, (clientX - box.left - CAP / 2) / travel)) : 0;
  };

  /** The cap follows the pointer; the snapped value goes out only when it changes. */
  const follow = (clientX: number): number => {
    const frac = fracAt(clientX);
    if (frac === null) return value;
    setDragFrac(frac);
    const next = quantise(min + frac * span);
    if (next !== sent.current) {
      sent.current = next;
      onChange(next);
    }
    return next;
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging(true);
    sent.current = value;
    follow(e.clientX);
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    follow(e.clientX);
  };

  // Release: the cap settles onto the committed step with the eased transition, because the
  // drag's own position is dropped in the same render that turns the transitions back on.
  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    e.currentTarget.releasePointerCapture(e.pointerId);
    const committed = follow(e.clientX);
    setDragging(false);
    setDragFrac(null);
    onCommit?.(committed);
  };

  const onPointerCancel = (e: PointerEvent<HTMLDivElement>) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    setDragging(false);
    setDragFrac(null);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (disabled) return;
    let next: number | null = null;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") next = quantise(value + step);
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") next = quantise(value - step);
    else if (e.key === "PageUp") next = quantise(value + step * 10);
    else if (e.key === "PageDown") next = quantise(value - step * 10);
    else if (e.key === "Home") next = quantise(min);
    else if (e.key === "End") next = quantise(max);
    if (next === null) return;
    e.preventDefault();
    onChange(next);
    onCommit?.(next);
  };

  return (
    <div
      ref={ref}
      className="slider"
      style={style}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-orientation="horizontal"
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-disabled={disabled ? true : undefined}
      data-dragging={dragging ? "true" : "false"}
      data-disabled={disabled ? "true" : undefined}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
      onKeyDown={onKeyDown}
    >
      <div className="slider-track" />
      {floor !== undefined && <div className="slider-floor" />}
      <div className="slider-fill" />
      <div className="slider-cap" />
    </div>
  );
}
