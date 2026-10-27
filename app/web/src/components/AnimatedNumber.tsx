"use client";

import type { CSSProperties } from "react";
import { useCounter } from "@/components/Motion";

/**
 * A number that counts to its new value, through every step of `decimals`. Tabular figures,
 * and the width is held at the widest the count reaches (right-aligned), so the digits never
 * move what sits beside them while they run.
 */
export function AnimatedNumber({
  value,
  decimals = 0,
  duration,
  className,
}: {
  value: number;
  decimals?: number;
  duration?: number;
  className?: string;
}) {
  const { shown, chars } = useCounter(value, decimals, duration);
  const style: CSSProperties = { minWidth: `${chars}ch` };
  return (
    <span className={className ? `value-tween ${className}` : "value-tween"} style={style}>
      {shown.toFixed(decimals)}
    </span>
  );
}
