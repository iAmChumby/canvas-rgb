"use client";

// The LED-order view (#468, phase 3): the run drawn as a ring or a line, a notch on the physical
// LED that is position 0, and an arrow for the direction. The notch is a slider: drag it to another
// LED, or focus it and use the arrow keys (one LED a press) and Home (back to LED 0). The arrow
// toggles the direction and the crosshair identifies the first LED on the real device.

import { useRef, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { cancelPress } from "@/lib/pressDrag";
import { nearestLed, notchDot, stepFirstLed } from "@/lib/ledOrder";
import type { Dot } from "@/views/patterns/dotLayout";
import s from "@/components/LedOrderControl.module.css";

/** Where the notch and arrow sit on one dot. */
interface Marker {
  tip: [number, number];
  baseA: [number, number];
  baseB: [number, number];
  arrowFrom: [number, number];
  arrowTo: [number, number];
}

/** The ring/line geometry of the marker, from the same dots the LEDs are drawn at. */
function markerOf(dot: Dot, dots: Dot[], reversed: boolean, ring: boolean): Marker {
  const sign = reversed ? -1 : 1;
  if (ring) {
    // Outward from the ring's centre, and the tangent the run follows from the notch.
    const cx = dots.reduce((n, d) => n + d.x, 0) / dots.length;
    const cy = dots.reduce((n, d) => n + d.y, 0) / dots.length;
    const len = Math.hypot(dot.x - cx, dot.y - cy) || 1;
    const ux = (dot.x - cx) / len;
    const uy = (dot.y - cy) / len;
    const tx = -uy * sign;
    const ty = ux * sign;
    const baseX = dot.x + ux * (dot.r + 11);
    const baseY = dot.y + uy * (dot.r + 11);
    return {
      tip: [dot.x + ux * (dot.r + 2), dot.y + uy * (dot.r + 2)],
      baseA: [baseX - uy * 5, baseY + ux * 5],
      baseB: [baseX + uy * 5, baseY - ux * 5],
      arrowFrom: [baseX + ux * 5 - tx * 9, baseY + uy * 5 - ty * 9],
      arrowTo: [baseX + ux * 5 + tx * 9, baseY + uy * 5 + ty * 9],
    };
  }
  const y = dot.y - dot.r - 3;
  return {
    tip: [dot.x, y],
    baseA: [dot.x - 6, y - 11],
    baseB: [dot.x + 6, y - 11],
    arrowFrom: [dot.x - sign * 10, y - 11],
    arrowTo: [dot.x + sign * 10, y - 11],
  };
}

/** LEDs as dots when no larger view supplied them (the Devices row's own small view). */
function miniDots(count: number, ring: boolean, width: number, height: number): Dot[] {
  const n = Math.max(1, count);
  if (ring) {
    const r = Math.max(6, Math.min(width, height) / 2 - 16);
    const cx = width / 2;
    const cy = height / 2;
    return Array.from({ length: n }, (_, j) => {
      const theta = ((-90 + (360 * j) / n) * Math.PI) / 180;
      return { index: j, x: cx + r * Math.cos(theta), y: cy + r * Math.sin(theta), r: 4 };
    });
  }
  return Array.from({ length: n }, (_, j) => ({
    index: j,
    x: ((j + 0.5) * width) / n,
    y: height / 2 + 6,
    r: 4,
  }));
}

export function LedOrderControl({
  ledCount,
  firstLed,
  reversed,
  ring,
  label,
  dots,
  width,
  height,
  onFirstLed,
  onReversed,
  onIdentify,
}: {
  ledCount: number;
  firstLed: number;
  reversed: boolean;
  ring: boolean;
  label: string;
  /** Dots of a larger view to align with; mini views lay themselves out when left out. */
  dots?: Dot[];
  width?: number;
  height?: number;
  onFirstLed: (led: number) => void;
  onReversed?: (next: boolean) => void;
  onIdentify?: () => void;
}) {
  const svg = useRef<SVGSVGElement>(null);
  const w = width ?? (ring ? 132 : 220);
  const h = height ?? (ring ? 132 : 44);
  const placed = dots && dots.length > 0 ? dots : miniDots(ledCount, ring, w, h);
  const dot = notchDot(placed, firstLed);
  const marker = dot ? markerOf(dot, placed, reversed, ring) : null;

  const local = (event: ReactPointerEvent<SVGCircleElement>) => {
    const rect = svg.current?.getBoundingClientRect();
    return rect
      ? { x: ((event.clientX - rect.left) / rect.width) * w, y: ((event.clientY - rect.top) / rect.height) * h }
      : { x: 0, y: 0 };
  };

  const move = (event: ReactPointerEvent<SVGCircleElement>) => {
    if (!event.currentTarget.hasPointerCapture?.(event.pointerId) || !svg.current) return;
    const at = local(event);
    const next = nearestLed(placed, at.x, at.y);
    if (next !== null && next !== firstLed) onFirstLed(next);
  };

  const key = (event: ReactKeyboardEvent<SVGCircleElement>) => {
    const next = stepFirstLed(firstLed, event.key, ledCount);
    if (next === null) return;
    event.preventDefault();
    onFirstLed(next);
  };

  const toggle = (event: ReactKeyboardEvent<SVGGElement>) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onReversed?.(!reversed);
    }
  };

  return (
    <svg
      ref={svg}
      className={s.view}
      viewBox={`0 0 ${w} ${h}`}
      width="100%"
      height="100%"
      role="group"
      aria-label={`${label}: LED 1 is ${firstLed + 1}, running ${reversed ? "backwards" : "forwards"}`}
      data-testid="led-order"
      data-first-led={firstLed}
      data-reversed={reversed ? "true" : "false"}
    >
      {!dots &&
        placed.map((d) => (
          <circle key={d.index} cx={d.x} cy={d.y} r={d.r} className={s.led} />
        ))}
      {marker && dot && (
        <>
          <polygon
            className={s.notch}
            points={`${marker.tip.join(",")} ${marker.baseA.join(",")} ${marker.baseB.join(",")}`}
          />
          <line
            className={s.arrow}
            x1={marker.arrowFrom[0]}
            y1={marker.arrowFrom[1]}
            x2={marker.arrowTo[0]}
            y2={marker.arrowTo[1]}
            markerEnd="url(#led-order-tip)"
          />
          <defs>
            <marker id="led-order-tip" markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto">
              <path d="M0,0 L6,3 L0,6 Z" className={s.arrow} />
            </marker>
          </defs>
          <circle
            className={s.handle}
            cx={dot.x}
            cy={dot.y}
            r={Math.max(12, dot.r + 4)}
            role="slider"
            tabIndex={0}
            aria-label={`${label}: first LED`}
            aria-valuemin={1}
            aria-valuemax={ledCount}
            aria-valuenow={firstLed + 1}
            data-testid="first-led-notch"
            onPointerDown={(event) => {
              cancelPress(event, { focus: true });
              event.currentTarget.setPointerCapture?.(event.pointerId);
            }}
            onPointerMove={move}
            onPointerUp={(event) => {
              if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
                event.currentTarget.releasePointerCapture(event.pointerId);
              }
            }}
            onKeyDown={key}
          />
          {onReversed && (
            <g
              className={s.key}
              role="button"
              tabIndex={0}
              aria-label="Reverse the run"
              data-testid="led-order-direction"
              onClick={() => onReversed(!reversed)}
              onKeyDown={toggle}
              transform={`translate(${w - 16}, 12)`}
            >
              <circle r="11" className={s.keyBg} />
              <path d="M -5 0 A 5 5 0 1 1 2 5" fill="none" className={s.keyInk} />
              <path d="M 2 5 L 5 3 L 3 0 Z" className={s.keyInk} />
            </g>
          )}
          {onIdentify && (
            <g
              className={s.key}
              role="button"
              tabIndex={0}
              aria-label="Identify LED 1"
              data-testid="led-order-identify"
              onClick={onIdentify}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  onIdentify();
                }
              }}
              transform={`translate(16, 12)`}
            >
              <circle r="11" className={s.keyBg} />
              <circle r="4" fill="none" className={s.keyInk} />
              <path d="M 0 -7 L 0 -5 M 0 5 L 0 7 M -7 0 L -5 0 M 5 0 L 7 0" className={s.keyInk} />
            </g>
          )}
        </>
      )}
    </svg>
  );
}
