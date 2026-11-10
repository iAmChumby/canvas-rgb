"use client";

import type { CSSProperties, ReactNode } from "react";

/** The highest reading in a history, or `floor` when it is higher. A slot with no reading (`null`)
 *  is not a 0: it is skipped, so a gap never pulls the peak down. */
export function peakOf(values: readonly (number | null)[], floor = 0): number {
  let peak = floor;
  for (const v of values) if (v !== null && v > peak) peak = v;
  return peak;
}

interface SparkPoint {
  x: number;
  y: number;
}

/**
 * Sparkline of the last samples. Drawn at a fixed scale; the exact value is shown beside it.
 * One slot per sample, evenly spaced, so the x axis is time. A slot with no reading (`null`) is a
 * gap: the line breaks there (one path per run of readings) and never bridges it (#279).
 */
export function Sparkline({
  values,
  max,
  tone = "a3",
  label,
}: {
  values: readonly (number | null)[];
  max: number;
  tone?: "a1" | "a2" | "a3" | "a4" | "a5";
  label: string;
}) {
  const w = 180;
  const h = 44;
  const scaleMax = max > 0 ? max : 1;
  const step = values.length > 1 ? w / (values.length - 1) : w;
  // The runs of consecutive readings, split at every gap.
  const runs: SparkPoint[][] = [];
  let run: SparkPoint[] | null = null;
  values.forEach((v, i) => {
    if (v === null) {
      run = null;
      return;
    }
    const x = values.length > 1 ? i * step : w;
    const y = h - 3 - (Math.min(v, scaleMax) / scaleMax) * (h - 6);
    const pt = { x: Number(x.toFixed(1)), y: Number(y.toFixed(1)) };
    if (run) run.push(pt);
    else {
      run = [pt];
      runs.push(run);
    }
  });
  // One reading and nothing else is a flat line across the window, not a lone dot: the speed has
  // been the same since the first sample, and a dot reads as a broken chart.
  const only = values.length === 1 ? runs[0]?.[0] : undefined;
  const drawn: SparkPoint[][] = only ? [[{ x: 0, y: only.y }, only]] : runs;
  const read = values.filter((v): v is number => v !== null);
  // A history that never moved is a rule, not a filled block.
  const flat = read.length > 0 && read.every((v) => v === read[0]);
  const lines = drawn.filter((r) => r.length > 1);
  // A reading with a gap on both sides is a dot: a one-point line would draw nothing.
  const dots = drawn.filter((r) => r.length === 1).map((r) => r[0] as SparkPoint);
  const trace = (r: SparkPoint[]) => r.map((p, i) => `${i === 0 ? "M" : "L"} ${p.x} ${p.y}`).join(" ");
  const areaOf = (r: SparkPoint[]) => {
    const first = r[0] as SparkPoint;
    const end = r[r.length - 1] as SparkPoint;
    return `${trace(r)} L ${end.x} ${h} L ${first.x} ${h} Z`;
  };
  const last = values[values.length - 1] ?? null;
  const peak = read.length > 0 ? Math.max(...read) : null;
  // The head marks the newest sample, and only when that sample was read.
  const tail = drawn[drawn.length - 1];
  const head = last !== null && tail ? tail[tail.length - 1] : undefined;

  return (
    <svg
      className="sparkline"
      viewBox={`0 0 ${w} ${h}`}
      role="img"
      aria-label={label}
      style={{ color: `var(--${tone})` } as CSSProperties}
    >
      <title>
        {last === null ? `${label}: no reading` : `${label}: ${Math.round(last)} RPM, peak ${Math.round(peak ?? 0)}`}
      </title>
      <line x1={0} x2={w} y1={h - 0.5} y2={h - 0.5} stroke="var(--line-2)" strokeWidth={1} />
      {!flat && lines.map((r, i) => <path key={`a${i}`} className="spark-area" d={areaOf(r)} />)}
      {lines.map((r, i) => (
        <path key={`l${i}`} className="spark-line" d={trace(r)} fill="none" stroke="currentColor" strokeWidth={1.75} strokeLinejoin="round" />
      ))}
      {dots.map((p, i) => (
        <circle key={`d${i}`} className="spark-dot" cx={p.x} cy={p.y} r={1.5} fill="currentColor" />
      ))}
      {head && <circle className="spark-head" cx={head.x} cy={head.y} r={2.5} />}
    </svg>
  );
}

/** A labelled figure: the exact value in mono, with its unit. */
export function Figure({ children, unit }: { children: ReactNode; unit?: string }) {
  return (
    <span className="fig" style={{ whiteSpace: "nowrap" }}>
      {children}
      {unit && <span className="unit" style={{ marginLeft: 4 }}>{unit}</span>}
    </span>
  );
}
