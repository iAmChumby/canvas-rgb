"use client";

import { useTweenNumber } from "@/components/Motion";

/**
 * A number that eases to its new value. Tabular figures, so the digits do not
 * jump sideways while they move.
 */
export function AnimatedNumber({
  value,
  decimals = 0,
  duration = 220,
  className,
}: {
  value: number;
  decimals?: number;
  duration?: number;
  className?: string;
}) {
  const shown = useTweenNumber(value, duration);
  return (
    <span className={className ? `value-tween ${className}` : "value-tween"}>
      {shown.toFixed(decimals)}
    </span>
  );
}
