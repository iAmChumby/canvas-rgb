"use client";

import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import type { CurvePoint } from "@/proto/CurvePoint";
import { HOT_FLOOR, HOT_CPU_FAN_FLOOR, HOT_TEMP_C, CPU_FAN } from "@/lib/fan-rules";
import { clamp } from "@/lib/format";

const TEMP_MIN = 20;
const TEMP_MAX = 100;
const MARGIN = { left: 48, right: 14, top: 14, bottom: 46 };
const ASPECT = 0.62;

/**
 * A 2 to 8 point curve on a temperature × percent grid. The shaded band at the bottom is the
 * floor. Above 50 °C a manual header is raised to its hot floor, so that band is drawn too.
 * Points drag, or take the arrow keys. The coordinates are read from the points list beside it.
 */
export function FanCurveEditor({
  header,
  points,
  floor,
  currentTemp,
  onChange,
  onCommit,
}: {
  header: string;
  points: CurvePoint[];
  floor: number;
  currentTemp: number | null;
  onChange: (points: CurvePoint[]) => void;
  onCommit: (points: CurvePoint[]) => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(360);
  const drag = useRef<number | null>(null);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(Math.max(260, el.clientWidth)));
    ro.observe(el);
    setWidth(Math.max(260, el.clientWidth));
    return () => ro.disconnect();
  }, []);

  const height = Math.round(width * ASPECT);
  const plotW = width - MARGIN.left - MARGIN.right;
  const plotH = height - MARGIN.top - MARGIN.bottom;

  const xOf = (t: number) => MARGIN.left + ((t - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)) * plotW;
  const yOf = (p: number) => MARGIN.top + (1 - p / 100) * plotH;
  const tempOfX = (x: number) => TEMP_MIN + ((x - MARGIN.left) / plotW) * (TEMP_MAX - TEMP_MIN);
  const pctOfY = (y: number) => (1 - (y - MARGIN.top) / plotH) * 100;

  const toData = (e: PointerEvent<Element>) => {
    const box = wrapRef.current?.getBoundingClientRect();
    if (!box) return null;
    const x = e.clientX - box.left;
    const y = e.clientY - box.top;
    return { t: clamp(tempOfX(x), TEMP_MIN, TEMP_MAX), p: clamp(pctOfY(y), floor, 100) };
  };

  // Points, sorted by temperature, drawn as a flat-then-linear line, as the daemon evaluates it.
  const sorted = [...points].sort((a, b) => a.temp_c - b.temp_c);
  const line = sorted.map((p) => `${xOf(p.temp_c).toFixed(1)},${yOf(p.percent).toFixed(1)}`).join(" ");

  // During a drag the array keeps its order, so the dragged index stays valid. The bounds keep
  // every point between its neighbours; the sort happens on commit only.
  const update = (i: number, next: Partial<CurvePoint>, commit: boolean) => {
    const copy = points.map((p, j) => (j === i ? { ...p, ...next } : p));
    if (commit) onCommit([...copy].sort((a, b) => a.temp_c - b.temp_c));
    else onChange(copy);
  };

  const boundsFor = (i: number): { lo: number; hi: number } => {
    const me = points[i];
    let lo = TEMP_MIN;
    let hi = TEMP_MAX;
    points.forEach((p, j) => {
      if (j === i || !me) return;
      if (p.temp_c <= me.temp_c) lo = Math.max(lo, p.temp_c + 1);
      else hi = Math.min(hi, p.temp_c - 1);
    });
    return { lo, hi };
  };

  const onDown = (i: number, e: PointerEvent<SVGCircleElement>) => {
    e.preventDefault();
    drag.current = i;
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onMove = (e: PointerEvent<SVGCircleElement>) => {
    const i = drag.current;
    if (i === null) return;
    const d = toData(e);
    if (!d) return;
    const { lo, hi } = boundsFor(i);
    update(i, { temp_c: clamp(Math.round(d.t), lo, Math.max(lo, hi)), percent: Math.round(d.p) }, false);
  };

  const onUp = (e: PointerEvent<SVGCircleElement>) => {
    const i = drag.current;
    if (i === null) return;
    drag.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    const d = toData(e);
    if (!d) return;
    const { lo, hi } = boundsFor(i);
    update(i, { temp_c: clamp(Math.round(d.t), lo, Math.max(lo, hi)), percent: Math.round(d.p) }, true);
  };

  const onKey = (i: number, e: KeyboardEvent<SVGCircleElement>) => {
    const p = points[i];
    if (!p) return;
    const big = e.shiftKey ? 5 : 1;
    let next: Partial<CurvePoint> | null = null;
    if (e.key === "ArrowUp") next = { percent: clamp(p.percent + big, floor, 100) };
    if (e.key === "ArrowDown") next = { percent: clamp(p.percent - big, floor, 100) };
    if (e.key === "ArrowRight") next = { temp_c: clamp(p.temp_c + big, TEMP_MIN, TEMP_MAX) };
    if (e.key === "ArrowLeft") next = { temp_c: clamp(p.temp_c - big, TEMP_MIN, TEMP_MAX) };
    if (!next) return;
    e.preventDefault();
    update(i, next, true);
  };

  const hot = header === CPU_FAN ? HOT_CPU_FAN_FLOOR : HOT_FLOOR;
  const hotTop = yOf(hot);
  const hotLeft = xOf(HOT_TEMP_C);
  const floorTop = yOf(floor);
  const grid = [20, 40, 60, 80, 100];
  const pctGrid = [0, 20, 40, 60, 80, 100];

  return (
    <div ref={wrapRef} className="curve" style={{ height }}>
      <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="group" aria-label="Fan curve">
        {pctGrid.map((p) => (
          <g key={`p${p}`}>
            <line x1={MARGIN.left} x2={width - MARGIN.right} y1={yOf(p)} y2={yOf(p)} stroke="var(--line)" />
            <text x={MARGIN.left - 8} y={yOf(p) + 5} textAnchor="end" fill="var(--mute)" fontSize={14} fontFamily="var(--fm)">
              {p}%
            </text>
          </g>
        ))}
        {grid.map((t) => (
          <g key={`t${t}`}>
            <line x1={xOf(t)} x2={xOf(t)} y1={MARGIN.top} y2={MARGIN.top + plotH} stroke="var(--line)" />
            <text x={xOf(t)} y={MARGIN.top + plotH + 20} textAnchor="middle" fill="var(--mute)" fontSize={14} fontFamily="var(--fm)">
              {t}
            </text>
          </g>
        ))}
        <text x={MARGIN.left + plotW} y={height - 6} textAnchor="end" fill="var(--mute)" fontSize={14}>
          temperature °C
        </text>

        {/* Floor: shaded from 0% up to the floor, across the whole plot. */}
        <rect
          x={MARGIN.left}
          y={floorTop}
          width={plotW}
          height={MARGIN.top + plotH - floorTop}
          fill="color-mix(in srgb, var(--down) 18%, transparent)"
        />
        {/* Hot floor: from 50 °C to the right, shaded up to the hot floor (G4). */}
        <rect
          x={hotLeft}
          y={hotTop}
          width={MARGIN.left + plotW - hotLeft}
          height={MARGIN.top + plotH - hotTop}
          fill="color-mix(in srgb, var(--warn) 16%, transparent)"
        />
        <line x1={hotLeft} x2={hotLeft} y1={MARGIN.top} y2={MARGIN.top + plotH} stroke="var(--warn)" strokeDasharray="4 4" />
        <text x={hotLeft + 6} y={MARGIN.top + 16} fill="var(--warn)" fontSize={14} fontFamily="var(--fm)">
          {HOT_TEMP_C} °C hot
        </text>

        {currentTemp !== null && (
          <line
            x1={xOf(clamp(currentTemp, TEMP_MIN, TEMP_MAX))}
            x2={xOf(clamp(currentTemp, TEMP_MIN, TEMP_MAX))}
            y1={MARGIN.top}
            y2={MARGIN.top + plotH}
            stroke="var(--ink)"
            strokeDasharray="2 3"
          />
        )}

        <polyline points={line} fill="none" stroke="var(--a2)" strokeWidth={2.5} strokeLinejoin="round" />
        {sorted.map((p, i) => {
          const idx = points.indexOf(p);
          return (
            <circle
              key={`${i}-${p.temp_c}-${p.percent}`}
              cx={xOf(p.temp_c)}
              cy={yOf(p.percent)}
              r={7}
              fill="var(--a2)"
              stroke="var(--panel)"
              strokeWidth={2}
              tabIndex={0}
              role="slider"
              aria-label={`Point ${i + 1}: ${p.temp_c} °C`}
              aria-valuetext={`${p.temp_c} °C, ${p.percent}%`}
              className="curve-point"
              onPointerDown={(e) => onDown(idx, e)}
              onPointerMove={onMove}
              onPointerUp={onUp}
              onKeyDown={(e) => onKey(idx, e)}
            />
          );
        })}
      </svg>
    </div>
  );
}
