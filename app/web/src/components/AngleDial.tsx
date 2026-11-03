"use client";

import { useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";
import { cancelPress, refuseDragStart } from "@/lib/pressDrag";

/** Where an angle points, in words, in eighths of the dial: 0 points right, 90 down, as on the canvas. */
function directionWord(deg: number): string {
  const d = ((deg % 360) + 360) % 360;
  if (d < 22.5 || d >= 337.5) return "right";
  if (d < 67.5) return "down-right";
  if (d < 112.5) return "down";
  if (d < 157.5) return "down-left";
  if (d < 202.5) return "left";
  if (d < 247.5) return "up-left";
  if (d < 292.5) return "up";
  return "up-right";
}

/**
 * A direction dial: a neutral ring with a needle that shows which way an effect's direction
 * points. An angle is a direction, not a colour, so this is never a hue wheel.
 *
 * The value is 0..360 degrees; 0 points right and 90 points down, as on the canvas. Drag the
 * ring to turn the needle; the arrow keys step 1° (shift for 15°), Home and End go to 0° and
 * 359°. The exact number lives in the field beside it (PropertyControls).
 */
export function AngleDial({
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
  // The one pointer that owns the drag: a second finger can neither move, end nor commit it.
  const owner = useRef<number | null>(null);
  const [dragging, setDragging] = useState(false);
  const deg = ((value % 360) + 360) % 360;
  // The read-out stays inside [0, 359]: a value a whisker under 360 rounds to 360, which sits
  // past aria-valuemax, so it wraps to 0 — the number the degrees field beside the dial shows.
  const now = Math.round(deg) % 360;

  /** The dial angle under the pointer: 0 at the right, growing clockwise (90 points down). */
  const angleAt = (e: PointerEvent<HTMLDivElement>): number => {
    const box = ref.current?.getBoundingClientRect();
    if (!box) return deg;
    const cx = box.left + box.width / 2;
    const cy = box.top + box.height / 2;
    // Screen coordinates run down, so the angle from +x grows clockwise, which is exactly
    // the canvas' angle: right 0, down 90, left 180, up 270.
    const raw = (Math.atan2(e.clientY - cy, e.clientX - cx) * 180) / Math.PI;
    // The wrap happens after rounding: a pointer a whisker below 0° is 359.7 before it rounds,
    // which rounds to 360 — past the max. Rounding first, then wrapping, keeps every committed
    // value inside 0..359 (−0.3° commits 0, as the read-out shows).
    return ((Math.round(raw) % 360) + 360) % 360;
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    // A disabled dial keeps the old press: a click on it must still blur (and so commit) a
    // field focused elsewhere, and that blur rides on the browser's own press behaviour.
    if (disabled) return;
    // A press never starts a native drag or a text selection (#105, the shared rule); the
    // keyboard keeps working after a mouse press, without claiming the keyboard's focus ring.
    cancelPress(e, { focus: true });
    if (owner.current !== null) return;
    owner.current = e.pointerId;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging(true);
    onChange(angleAt(e));
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled || e.pointerId !== owner.current) return;
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    onChange(angleAt(e));
  };

  /** Ends the drag whatever happened meanwhile, so the needle never stays frozen mid-turn. */
  const end = (e: PointerEvent<HTMLDivElement>) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    owner.current = null;
    setDragging(false);
  };

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    if (e.pointerId !== owner.current) return;
    const at = angleAt(e);
    end(e);
    if (!disabled) onCommit?.(at);
  };

  const onPointerCancel = (e: PointerEvent<HTMLDivElement>) => {
    if (e.pointerId !== owner.current) return;
    end(e);
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (disabled) return;
    const step = e.shiftKey ? 15 : 1;
    let next: number | null = null;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") next = (deg + step) % 360;
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") next = (deg - step + 360) % 360;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = 359;
    if (next === null) return;
    e.preventDefault();
    onChange(next);
    onCommit?.(next);
  };

  // The needle is drawn pointing right (0°) and turned to the value; the turn is eased while
  // it follows the keyboard, and follows the pointer directly while dragging.
  const needleStyle = {
    transform: `rotate(${deg}deg)`,
    transformOrigin: "50px 50px",
    transition: dragging ? "none" : "transform var(--dur-fast) var(--ease)",
  } as CSSProperties;

  return (
    <div
      ref={ref}
      className="angle-dial"
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={359}
      aria-valuenow={now}
      aria-valuetext={`${now}°, pointing ${directionWord(now)}`}
      aria-disabled={disabled ? true : undefined}
      data-dragging={dragging ? "true" : "false"}
      data-disabled={disabled ? "true" : undefined}
      draggable={false}
      onDragStart={refuseDragStart}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
      onLostPointerCapture={onPointerCancel}
      onKeyDown={onKey}
    >
      <svg viewBox="0 0 100 100" aria-hidden="true">
        {/* Cardinal ticks, so the dial reads as a direction, not a clock face. */}
        <line x1={95} y1={50} x2={99} y2={50} />
        <line x1={50} y1={95} x2={50} y2={99} />
        <line x1={5} y1={50} x2={1} y2={50} />
        <line x1={50} y1={1} x2={50} y2={5} />
        {/* The neutral ring. */}
        <circle cx={50} cy={50} r={40} />
        {/* The needle and its arrowhead, with the hub they turn on. */}
        <g className="angle-dial-needle" style={needleStyle}>
          <line x1={54} y1={50} x2={76} y2={50} />
          <polygon points="88,50 77,45.5 77,54.5" />
          <circle cx={50} cy={50} r={3.5} />
        </g>
      </svg>
    </div>
  );
}
