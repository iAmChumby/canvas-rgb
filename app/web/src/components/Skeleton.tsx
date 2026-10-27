"use client";

import type { CSSProperties } from "react";

/**
 * A placeholder block for content that is still loading. It fades in once and
 * holds still: no looping animation anywhere outside the effect previews.
 */
export function Skeleton({
  width,
  height,
  radius,
  className,
  style,
}: {
  width?: number | string;
  height?: number | string;
  radius?: number | string;
  className?: string;
  style?: CSSProperties;
}) {
  const s: CSSProperties = {
    width: typeof width === "number" ? `${width}px` : width,
    height: typeof height === "number" ? `${height}px` : height,
    borderRadius: typeof radius === "number" ? `${radius}px` : radius,
    ...style,
  };
  return <div className={className ? `skeleton ${className}` : "skeleton"} style={s} aria-hidden="true" />;
}

/** A few lines of text-shaped placeholders. */
export function SkeletonLines({ rows = 3 }: { rows?: number }) {
  return (
    <div aria-hidden="true">
      {Array.from({ length: rows }, (_, i) => (
        <Skeleton key={i} className="skeleton-line" width={i === rows - 1 ? "62%" : "100%"} height={12} />
      ))}
    </div>
  );
}
