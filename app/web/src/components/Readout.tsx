"use client";

import type { CSSProperties, ReactNode } from "react";

/** Sparkline of the last samples. Drawn at a fixed scale; the exact value is shown beside it. */
export function Sparkline({
  values,
  max,
  tone = "a3",
  label,
}: {
  values: number[];
  max: number;
  tone?: "a1" | "a2" | "a3" | "a4" | "a5";
  label: string;
}) {
  const w = 180;
  const h = 44;
  const scaleMax = max > 0 ? max : 1;
  const step = values.length > 1 ? w / (values.length - 1) : w;
  const pts = values.map((v, i) => {
    const x = values.length > 1 ? i * step : w;
    const y = h - 3 - (Math.min(v, scaleMax) / scaleMax) * (h - 6);
    return { x: Number(x.toFixed(1)), y: Number(y.toFixed(1)) };
  });
  const line = pts.map((p, i) => `${i === 0 ? "M" : "L"} ${p.x} ${p.y}`).join(" ");
  const area = pts.length > 1 ? `${line} L ${w} ${h} L 0 ${h} Z` : "";
  const head = pts.length > 0 ? pts[pts.length - 1] : undefined;
  const last = values.length > 0 ? (values[values.length - 1] ?? 0) : 0;
  const peak = values.length > 0 ? Math.max(...values) : 0;

  return (
    <svg
      className="sparkline"
      viewBox={`0 0 ${w} ${h}`}
      role="img"
      aria-label={label}
      style={{ color: `var(--${tone})` } as CSSProperties}
    >
      <title>{`${label}: ${Math.round(last)} RPM, peak ${Math.round(peak)}`}</title>
      <line x1={0} x2={w} y1={h - 0.5} y2={h - 0.5} stroke="var(--line-2)" strokeWidth={1} />
      {area && <path className="spark-area" d={area} />}
      {pts.length > 1 && (
        <path className="spark-line" d={line} fill="none" stroke="currentColor" strokeWidth={1.75} strokeLinejoin="round" />
      )}
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
