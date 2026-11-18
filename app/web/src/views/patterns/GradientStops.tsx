"use client";

// The Gradient tool's handles (#468): one square key per stop, drawn inside the ring (the ring fills
// its box, so the room is in the middle; above the dot on a line), laid over the device's own box. Dragging a key moves its stop to the nearest LED;
// the arrow keys move it one LED. The colours are the frame's: this only places the stops, and the
// editor writes their blend. The geometry is `dotLayout`'s, so a handle stands where its LED is drawn.

import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { ledNear, ledPosition } from "@/lib/gradient";
import { layoutDots, type Dot } from "@/views/patterns/dotLayout";
import type { GradientState } from "@/views/patterns/usePatternEditor";
import type { BuiltOn } from "@/proto/BuiltOn";
import s from "@/views/patterns/patterns.module.css";

/** How far off its dot a handle stands, in CSS pixels. */
const STAND_OFF = 18;
/** Half a handle: it stays this far inside the box. */
const HALF = 8;

export function GradientStops({
  builtOn,
  state,
  onSelect,
  onMove,
  onBegin,
  onEnd,
}: {
  builtOn: BuiltOn;
  state: GradientState;
  onSelect: (index: number) => void;
  onMove: (index: number, at: number) => void;
  onBegin: () => void;
  onEnd: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ width: 0, height: 0 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      setBox((was) => (was.width === r.width && was.height === r.height ? was : { width: r.width, height: r.height }));
    };
    measure();
    if (typeof ResizeObserver !== "function") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const dots = useMemo(() => layoutDots(builtOn, box.width, box.height), [builtOn, box.width, box.height]);
  const leds = dots.length;
  const ring = builtOn.arrangement === "ring";

  /** Where a handle stands for the LED it sits on: in from the ring toward its centre, or above a dot. */
  const stand = (dot: Dot): { x: number; y: number } => {
    let x = dot.x;
    let y = dot.y - dot.r - STAND_OFF;
    if (ring) {
      const cx = box.width / 2;
      const cy = box.height / 2;
      const dx = dot.x - cx;
      const dy = dot.y - cy;
      const length = Math.hypot(dx, dy) || 1;
      x = dot.x - (dx / length) * (dot.r + STAND_OFF);
      y = dot.y - (dy / length) * (dot.r + STAND_OFF);
    }
    return {
      x: Math.min(box.width - HALF, Math.max(HALF, x)),
      y: Math.min(box.height - HALF, Math.max(HALF, y)),
    };
  };

  /** The position of the LED nearest a pointer. */
  const positionAt = (event: ReactPointerEvent<HTMLElement>): number | null => {
    const rect = ref.current?.getBoundingClientRect();
    if (!rect || leds === 0) return null;
    const px = event.clientX - rect.left;
    const py = event.clientY - rect.top;
    let best = 0;
    let bestDistance = Infinity;
    for (const dot of dots) {
      const distance = Math.hypot(dot.x - px, dot.y - py);
      if (distance < bestDistance) {
        bestDistance = distance;
        best = dot.index;
      }
    }
    return ledPosition(best, leds, state.wrap);
  };

  const nudge = (event: KeyboardEvent<HTMLElement>, index: number, at: number) => {
    const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
    if (step === 0 || leds === 0) return;
    event.preventDefault();
    // Each key press is its own stroke: open and close the drag window around it so every nudge
    // pushes one undo entry, not just the first of a run (#481).
    onBegin();
    const led = ledNear(at, leds, state.wrap) + step;
    onMove(index, ledPosition(state.wrap ? (led + leds) % leds : Math.min(leds - 1, Math.max(0, led)), leds, state.wrap));
    onEnd();
  };

  // The seam: a hairline at the top of the ring, where the last stop meets the first.
  const top = ring && state.wrap && dots[0] ? dots[0] : null;

  return (
    <div className={s.stops} ref={ref}>
      {top && <span className={s.seam} style={{ left: top.x, top: Math.max(0, top.y - top.r - 12) }} aria-hidden="true" />}
      {leds > 0 &&
        state.stops.map((stop, index) => {
          const dot = dots[ledNear(stop.at, leds, state.wrap)];
          if (!dot) return null;
          const at = stand(dot);
          return (
            <button
              key={index}
              type="button"
              className={s.stop}
              data-selected={index === state.selected ? "true" : "false"}
              style={{ left: at.x, top: at.y, ["--stop" as string]: stop.colour }}
              aria-label={`Stop ${index + 1}, ${stop.colour}`}
              aria-pressed={index === state.selected}
              onPointerDown={(event) => {
                event.currentTarget.setPointerCapture?.(event.pointerId);
                onSelect(index);
                onBegin();
              }}
              onPointerMove={(event) => {
                if (!event.currentTarget.hasPointerCapture?.(event.pointerId)) return;
                const at2 = positionAt(event);
                if (at2 !== null) onMove(index, at2);
              }}
              onPointerUp={(event) => {
                if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
                  event.currentTarget.releasePointerCapture(event.pointerId);
                }
                onEnd();
              }}
              onPointerCancel={onEnd}
              onKeyDown={(event) => nudge(event, index, stop.at)}
              onFocus={() => onSelect(index)}
            />
          );
        })}
    </div>
  );
}
