"use client";

import type { CSSProperties } from "react";
import { useLoadDelay } from "@/lib/loadDelay";

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

/**
 * One line of placeholder text, sized in `ch` (`<InlineSkeleton width="12ch" />`), for a value, a
 * name or a readout that has not arrived. Part of the element-level loading kit (#258):
 *
 * - It appears only after `LOAD_DELAY_MS`. Before that it holds its width and stays invisible, so
 *   a fast answer neither flashes a block nor moves the line.
 * - It is `aria-hidden` and announces nothing, so a screen reader hears the value when it comes.
 *   It is a drawing, not a status: the element that owns the value (the row, the readout, the
 *   control) sets `aria-busy` while it is unknown. A skeleton cannot do that for its owner, and
 *   must never be a `role="status"` inside a button.
 * - It wears the `Skeleton` tone and its one fade-in; nothing loops.
 *
 * Draw it only while the value is unknown. Put the real value in once it is known; an unknown
 * value is never drawn as a real one (a dash, "0 of 0", a neutral default).
 */
export function InlineSkeleton({
  width = "8ch",
  active = true,
  className,
}: {
  /** A CSS width; `ch` keeps it in step with the text it stands in for. */
  width?: string;
  /** False draws nothing, for a caller that always renders the component. */
  active?: boolean;
  className?: string;
}) {
  const shown = useLoadDelay(active);
  if (!active) return null;
  return (
    <span
      className={className ? `inline-skel ${className}` : "inline-skel"}
      data-shown={shown ? "true" : "false"}
      style={{ width }}
      aria-hidden="true"
    />
  );
}

/**
 * The state of a canvas that has no picture yet: the value for a canvas's `data-wait` attribute.
 * `loaded` is whether the first frame is in. The canvas keeps its box and, once the delay has
 * passed with no frame, wears the `Skeleton` tone (CSS: `.preview-canvas[data-wait="shown"]`).
 * Used by `EffectPreview` and `LoopPreview`, whose root is the canvas itself and cannot hold a child.
 *
 * `failed` is a picture that will not come (the fetch failed, the preview errored): the wait is
 * over, so the canvas stops being a skeleton and stands as a still "no picture" box (`"failed"`),
 * and the caller drops its `aria-busy`.
 */
export function useCanvasWait(loaded: boolean, failed = false): "shown" | "held" | "failed" | undefined {
  const shown = useLoadDelay(!loaded && !failed);
  if (loaded) return undefined;
  if (failed) return "failed";
  return shown ? "shown" : "held";
}

/**
 * A skeleton that fills its positioned parent until the first frame of a thumbnail arrives. It
 * appears after `LOAD_DELAY_MS` and is as large as the canvas that follows it, so the tile never
 * changes size when the picture comes.
 */
export function CanvasSkeleton() {
  const shown = useLoadDelay(true);
  return <span className="skeleton canvas-skel" data-shown={shown ? "true" : "false"} aria-hidden="true" />;
}
