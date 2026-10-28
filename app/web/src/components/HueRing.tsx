"use client";

import { useRef, type KeyboardEvent, type PointerEvent } from "react";

const SEGMENTS = 36;

function polar(cx: number, cy: number, r: number, deg: number): [number, number] {
  const rad = ((deg - 90) * Math.PI) / 180;
  return [cx + r * Math.cos(rad), cy + r * Math.sin(rad)];
}

function arcPath(cx: number, cy: number, rOuter: number, rInner: number, a0: number, a1: number): string {
  const [x0, y0] = polar(cx, cy, rOuter, a0);
  const [x1, y1] = polar(cx, cy, rOuter, a1);
  const [x2, y2] = polar(cx, cy, rInner, a1);
  const [x3, y3] = polar(cx, cy, rInner, a0);
  return `M ${x0} ${y0} A ${rOuter} ${rOuter} 0 0 1 ${x1} ${y1} L ${x2} ${y2} A ${rInner} ${rInner} 0 0 0 ${x3} ${y3} Z`;
}

/**
 * A hue picker: a ring of the full spectrum; drag or use the arrow keys. The value is 0..360
 * degrees, and the number beside it is the exact value.
 */
export function HueRing({
  value,
  label,
  disabled,
  onChange,
  onCommit,
}: {
  value: number;
  label: string;
  disabled?: boolean;
  onChange: (deg: number) => void;
  onCommit?: (deg: number) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const hue = ((value % 360) + 360) % 360;

  const angleAt = (e: PointerEvent<HTMLDivElement>): number => {
    const box = ref.current?.getBoundingClientRect();
    if (!box) return hue;
    const cx = box.left + box.width / 2;
    const cy = box.top + box.height / 2;
    const deg = (Math.atan2(e.clientY - cy, e.clientX - cx) * 180) / Math.PI + 90;
    return Math.round((deg + 360) % 360);
  };

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    onChange(angleAt(e));
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) onChange(angleAt(e));
  };
  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
      onCommit?.(angleAt(e));
    }
  };
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (disabled) return;
    const step = e.shiftKey ? 15 : 3;
    let next: number | null = null;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") next = (hue + step) % 360;
    if (e.key === "ArrowLeft" || e.key === "ArrowDown") next = (hue - step + 360) % 360;
    if (next === null) return;
    e.preventDefault();
    onChange(next);
    onCommit?.(next);
  };

  const segs = [];
  for (let i = 0; i < SEGMENTS; i++) {
    const a0 = (i * 360) / SEGMENTS;
    const a1 = ((i + 1) * 360) / SEGMENTS + 0.6;
    segs.push(
      <path
        key={i}
        d={arcPath(50, 50, 48, 36, a0, a1)}
        fill={`hsl(${(i * 360) / SEGMENTS} 80% 62%)`}
      />,
    );
  }
  const [mx, my] = polar(50, 50, 42, hue);

  return (
    <div
      ref={ref}
      className="hue-ring"
      role="slider"
      tabIndex={0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={360}
      aria-valuenow={Math.round(hue)}
      aria-disabled={disabled ? true : undefined}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onKeyDown={onKey}
    >
      <svg viewBox="0 0 100 100" aria-hidden="true">
        {segs}
        <circle cx={mx} cy={my} r={5.5} fill="var(--ink)" stroke="var(--panel)" strokeWidth={2} />
      </svg>
    </div>
  );
}
