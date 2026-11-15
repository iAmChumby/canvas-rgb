"use client";

// The length step's fader: the chain's length, with the figure the owner watches count.
//
// It is the component layer's Slider, not a native range: a slot with a flat cap, which takes the
// pointer over the whole 44 px of its height while the cap itself is 14 x 24. The arrows move the
// length by one; Shift with an arrow moves it by ten, caught before the fader sees the key so the
// value never passes through the one step in between.

import type { KeyboardEvent } from "react";

import { AnimatedNumber } from "@/components/AnimatedNumber";
import { Slider } from "@/components/Slider";
import s from "@/views/chain/chain.module.css";

/** How far Shift and an arrow move the length. */
const SHIFT_STEP = 10;

export function LengthSlider({
  value,
  max,
  onChange,
  onCommit,
}: {
  value: number;
  max: number;
  onChange: (n: number) => void;
  onCommit?: (n: number) => void;
}) {
  const shiftMove = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!e.shiftKey) return;
    const dir =
      e.key === "ArrowRight" || e.key === "ArrowUp"
        ? 1
        : e.key === "ArrowLeft" || e.key === "ArrowDown"
          ? -1
          : 0;
    if (dir === 0) return;
    e.preventDefault();
    e.stopPropagation();
    const next = Math.min(max, Math.max(1, value + dir * SHIFT_STEP));
    onChange(next);
    onCommit?.(next);
  };

  return (
    <div className={s.fader} onKeyDownCapture={shiftMove}>
      <div className={s.reading}>
        {/* As wide as the longest length's digits: the count grows in place, "LEDs" never moves. */}
        <span className={s.count} data-testid="chain-length" style={{ minWidth: `${String(max).length}ch` }}>
          <AnimatedNumber value={value} />
        </span>
        <span className={s.countWord}>LEDs</span>
      </div>
      <Slider
        value={value}
        min={1}
        max={max}
        label="Chain length"
        onChange={onChange}
        onCommit={onCommit}
      />
      <div className={s.ends} aria-hidden="true">
        <span>1</span>
        <span>{max}</span>
      </div>
    </div>
  );
}
