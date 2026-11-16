"use client";

// The slide step's picture of what the hardware shows: a track of LEDs, the trail white, the last
// one orange. Up to 48 LEDs are drawn one by one; a longer run is a solid bar with an orange tick,
// because 300 cells would be noise. The colours are the LEDs' own (`TRAIL_COLOUR`, the first of
// `PART_COLOURS`), the same ones the daemon is told to light.

import type { CSSProperties } from "react";

import { PART_COLOURS } from "@/views/chain/partColours";
import { TRAIL_COLOUR } from "@/views/chain/useChainSetup";
import s from "@/views/chain/chain.module.css";

/** The most LEDs drawn one by one. */
export const DRAWN_CELLS_MAX = 48;

export function RunDrawing({ length }: { length: number }) {
  const end = PART_COLOURS[0] ?? "#ff4d00";
  const style = { "--trail": TRAIL_COLOUR, "--end": end } as CSSProperties;
  if (length > DRAWN_CELLS_MAX) {
    return (
      <div className={s.track} style={style} data-testid="run-drawing" data-solid="true" aria-hidden="true">
        <span className={s.bar} />
        <span className={s.tick} />
      </div>
    );
  }
  return (
    <div className={s.track} style={style} data-testid="run-drawing" data-solid="false" aria-hidden="true">
      {Array.from({ length: Math.max(1, length) }, (_, i) => (
        <span key={i} className={s.cell} data-end={i === length - 1 ? "true" : "false"} />
      ))}
    </div>
  );
}
