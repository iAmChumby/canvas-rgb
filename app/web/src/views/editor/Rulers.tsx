"use client";

import { DESK_Y } from "@/lib/caseForms";
import type { LengthUnit } from "@/lib/zones";
import desk from "@/views/desk.module.css";

/** The effect area: the whole logical canvas, 320 x 200 units, 128 x 80 cm at 4 mm to the unit. */
export const EFFECT_W = 320;
export const EFFECT_H = 200;

/** The ruler band, in css pixels: the quiet strip along the top and the left of the frame. */
const BAND_PX = 13;
/** How long a tick is, in css pixels. */
const MINOR_PX = 4;
const MAJOR_PX = 8;

/**
 * The furniture of the canvas that is always there: the desk rule, the frame of the effect area
 * and the rulers along its top and left edges.
 *
 * The rulers are real: the frame is 128 x 80 cm, so the reader can measure a part against them
 * and see that a fan is 12 cm across. They are drawn in canvas units, so they pan and zoom with
 * the desk, but their ticks and their numbers keep their size on the screen, the way a ruler
 * does.
 *
 * They are quiet, but they have to be readable on any picture, so every line and every number
 * is drawn twice: a dark keyline under a light one. On a dark picture the light line shows; on
 * a bright one the keyline shows.
 */
export function CanvasFurniture({
  svgRef,
  gRef,
  unit,
  k,
}: {
  svgRef: React.RefObject<SVGSVGElement | null>;
  gRef: React.RefObject<SVGGElement | null>;
  unit: LengthUnit;
  /** Css pixels per canvas unit. */
  k: number;
}) {
  const marks = rulerMarks(unit);
  const tick = (x1: number, y1: number, x2: number, y2: number, major: boolean) => (
    <>
      <line className={desk.rulerKey} x1={x1} y1={y1} x2={x2} y2={y2} strokeWidth={major ? 3 : 2} />
      <line className={desk.rulerLine} x1={x1} y1={y1} x2={x2} y2={y2} />
    </>
  );
  return (
    <svg
      ref={svgRef}
      className={desk.caseSvg}
      aria-hidden="true"
      style={{ ["--k" as string]: k }}
    >
      <g ref={gRef}>
        {/* The desk: the surface the case stands on and the peripherals sit on. */}
        <line className={desk.deskRule} x1={-2} y1={DESK_Y} x2={322} y2={DESK_Y} />
        {/* The frame of the effect area. */}
        <rect className={desk.effectFrame} x={0} y={0} width={EFFECT_W} height={EFFECT_H} />
        {/* The rulers: along the top edge and along the left edge. */}
        <g className={desk.ruler}>
          {marks
            .filter((m) => m.at <= EFFECT_W + 0.001)
            .map((m) =>
              tick(m.at, 0, m.at, (m.major ? MAJOR_PX : MINOR_PX) / k, m.major),
            )}
          {marks
            .filter((m) => m.at <= EFFECT_W + 0.001 && m.major)
            .map((m) => (
              <text key={`tt-${m.at}`} x={m.at + 2 / k} y={(BAND_PX - 2) / k}>
                {m.label}
              </text>
            ))}
          {marks
            .filter((m) => m.at <= EFFECT_H + 0.001)
            .map((m) => tick(0, m.at, (m.major ? MAJOR_PX : MINOR_PX) / k, m.at, m.major))}
          {marks
            .filter((m) => m.at <= EFFECT_H + 0.001 && m.major)
            .map((m) => (
              <text
                key={`lt-${m.at}`}
                x={(BAND_PX - 2) / k}
                y={m.at - 2 / k}
                className={desk.rulerLeft}
                transform={`rotate(-90 ${(BAND_PX - 2) / k} ${m.at - 2 / k})`}
              >
                {m.label}
              </text>
            ))}
        </g>
      </g>
    </svg>
  );
}

interface Mark {
  /** The place on the canvas, in canvas units. */
  at: number;
  major: boolean;
  label: string;
}

/**
 * The ruler's marks: a tick every 2 cm (or every inch) and a number every 10 cm (or every 5 in).
 * The numbers are the reader's own unit, never canvas units.
 */
function rulerMarks(unit: LengthUnit): Mark[] {
  const out: Mark[] = [];
  const perCm = 2.5; // canvas units to a centimetre
  const perIn = 25.4 / 4; // canvas units to an inch
  const minor = unit === "in" ? perIn : 2 * perCm;
  const count = Math.floor(Math.max(EFFECT_W, EFFECT_H) / minor);
  for (let i = 0; i <= count; i++) {
    const at = i * minor;
    const mm = at * 4;
    out.push({
      at,
      major: i % 5 === 0,
      label:
        unit === "in"
          ? String(Math.round((mm / 25.4) * 100) / 100)
          : String(Math.round(mm / 10)),
    });
  }
  return out;
}
