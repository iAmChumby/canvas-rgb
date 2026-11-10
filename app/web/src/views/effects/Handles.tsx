"use client";

// The picked frame's handles, read from the effect's own schema (#257 phase 3, spec §4a):
//
//   - Direction and speed (Gradient, Rainbow Wave): one vector. The arrow points the way the
//     effect runs and its length shows the speed, 16 px at the minimum and 72 px at the maximum.
//     Drag the tip around the frame's centre; Shift snaps to 15°. The grip is 44 px, and it is
//     focusable: the arrows turn it 1°, with Shift 15°.
//   - Speed only (Aurora, Spiral, Rain, Color Cycle, Breathing): a pill on the frame's bottom
//     edge. Scrub it left and right; Left and Right step it. A long `period` is slow, so the
//     pill's range reads backwards.
//
// The inspector's AngleDial and slider stay and mirror these: both write the same prop, and one
// drag or scrub is one undo step (the builder holds the run open until the gesture ends).

import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";
import { cancelPress, refuseDragStart } from "@/lib/pressDrag";
import {
  ANGLE_SNAP_DEG,
  ANGLE_STEP_DEG,
  ARROW_MIN_PX,
  angleFromPoint,
  arrowLength,
  clampArrow,
  formatSpeed,
  handleOf,
  normaliseAngle,
  numberOf,
  scrubSpeed,
  snapAngle,
  speedFraction,
  stepSpeed,
  type ClientBox,
  type SpeedSpec,
} from "@/lib/handles";
import { layerValues } from "@/views/effects/stackDraft";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { StackLayer } from "@/proto/StackLayer";

/** A press that owns a handle: one pointer only, so a second finger cannot move it. */
interface Grip {
  pointerId: number;
  /** The element holding the pointer capture, so an unmount can let it go. */
  el: HTMLElement;
  run: string;
  key: string;
  startX: number;
  startRaw: number;
  /** The pointer that owns the handle is still down. */
  down: boolean;
}

export function FrameHandles({
  layer,
  schema,
  centre,
  stage,
  onChange,
  onChangeEnd,
}: {
  layer: StackLayer;
  /** The effect's schema: without it there is nothing to read, so no handle is drawn. */
  schema: PropertySchema | null | undefined;
  /** The frame's centre in client pixels, read when a drag needs it. */
  centre: () => { x: number; y: number } | null;
  /** The stage's box in client pixels: the grip is clamped inside it, so it stays grabbable. */
  stage?: () => ClientBox | null;
  /** A prop changed inside `run`; `hold` keeps the step open until `onChangeEnd`. */
  onChange: (key: string, value: PropValue, run: string, hold: boolean) => void;
  onChangeEnd?: () => void;
}) {
  const handle = handleOf(schema);
  const values = layerValues(schema, layer.props);
  const grip = useRef<Grip | null>(null);
  // The arrow key (or the pill's arrow key) being held: its auto-repeats are one undo step.
  const held = useRef<{ key: string; run: string } | null>(null);
  const [dragging, setDragging] = useState(false);
  const runs = useRef(0);
  // The latest `onChangeEnd`, for the unmount cleanup (which must not depend on it).
  const endRef = useRef(onChangeEnd);
  useEffect(() => {
    endRef.current = onChangeEnd;
  });
  // A gesture cut off by an unmount (the picked layer removed, or the pick moved) still ends its
  // undo run: the next edit is its own step, and the capture is let go (#257 review).
  useEffect(() => {
    return () => {
      const current = grip.current;
      if (!current) return;
      grip.current = null;
      held.current = null;
      if (current.el.hasPointerCapture?.(current.pointerId)) {
        try {
          current.el.releasePointerCapture(current.pointerId);
        } catch {
          // The pointer was already gone; the capture went with it.
        }
      }
      endRef.current?.();
    };
  }, []);

  if (handle.kind === "none") return null;

  const layerId = layer.id;
  const beginRun = (key: string) => `props:${layerId}:${key}:${++runs.current}`;
  const end = () => {
    grip.current = null;
    held.current = null;
    setDragging(false);
    onChangeEnd?.();
  };

  if (handle.kind === "vector" || handle.kind === "direction") {
    const { direction } = handle;
    const speed = handle.kind === "vector" ? handle.speed : null;
    const dir = normaliseAngle(numberOf(values[direction.key]));
    const raw = speed ? numberOf(values[speed.key], speed.min) : null;
    const reach = clampArrow(dir, speed === null || raw === null ? ARROW_MIN_PX : arrowLength(speedFraction(speed, raw)), centre(), stage?.() ?? null);
    const readout = Math.round(dir) % 360;
    const word = directionWord(readout);

    const onDown = (e: PointerEvent<HTMLDivElement>) => {
      if (e.button > 0) return;
      e.stopPropagation();
      cancelPress(e, { focus: true });
      if (grip.current) return;
      try {
        e.currentTarget.setPointerCapture(e.pointerId);
      } catch {
        // A pointer already gone has nothing to capture.
      }
      grip.current = {
        pointerId: e.pointerId,
        el: e.currentTarget,
        run: beginRun(direction.key),
        key: direction.key,
        startX: e.clientX,
        startRaw: raw ?? 0,
        down: true,
      };
      setDragging(true);
      point(e.clientX, e.clientY, e.shiftKey);
    };

    /** The arrow tip follows the pointer around the frame's centre. */
    const point = (clientX: number, clientY: number, shift: boolean) => {
      const current = grip.current;
      if (!current) return;
      const box = centre();
      if (!box) return;
      const at = angleFromPoint(box, { x: clientX, y: clientY });
      const next = shift ? snapAngle(at) : at;
      onChange(direction.key, { type: "angle", value: next }, current.run, true);
    };

    const onMove = (e: PointerEvent<HTMLDivElement>) => {
      const current = grip.current;
      if (!current || current.pointerId !== e.pointerId) return;
      if (e.pointerType === "mouse" && e.buttons === 0) {
        end();
        return;
      }
      point(e.clientX, e.clientY, e.shiftKey);
    };

    const onUp = (e: PointerEvent<HTMLDivElement>) => {
      if (grip.current?.pointerId !== e.pointerId) return;
      end();
    };

    const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const step = e.shiftKey ? ANGLE_SNAP_DEG : ANGLE_STEP_DEG;
      let next: number | null = null;
      if (e.key === "ArrowRight" || e.key === "ArrowUp") next = normaliseAngle(dir + step);
      else if (e.key === "ArrowLeft" || e.key === "ArrowDown") next = normaliseAngle(dir - step);
      else if (e.key === "Home") next = 0;
      else if (e.key === "End") next = 359;
      if (next === null) return;
      e.preventDefault();
      e.stopPropagation();
      const again = e.repeat && held.current?.key === e.key;
      const run = again ? held.current!.run : beginRun(direction.key);
      held.current = { key: e.key, run };
      onChange(direction.key, { type: "angle", value: Math.round(next) % 360 }, run, true);
    };

    const onKeyUp = (e: KeyboardEvent<HTMLDivElement>) => {
      if (held.current?.key !== e.key) return;
      end();
    };

    return (
      <div
        className="rd-angle-handle"
        data-dragging={dragging ? "true" : "false"}
        style={{ transform: `rotate(${dir}deg)` }}
        onDragStart={refuseDragStart}
      >
        <span className="rd-angle-arm" style={{ width: `${reach}px` }} />
        <div
          className="rd-angle-grip"
          data-handle="direction"
          role="slider"
          tabIndex={0}
          style={{ left: `${reach}px` }}
          aria-label={`Direction of ${layer.name}`}
          aria-valuemin={0}
          aria-valuemax={359}
          aria-valuenow={readout}
          aria-valuetext={`${readout}°, pointing ${word}`}
          title="Drag to point the effect; Shift snaps to 15°. Arrow keys nudge."
          draggable={false}
          onPointerDown={onDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          onPointerCancel={onUp}
          onLostPointerCapture={onUp}
          onKeyDown={onKey}
          onKeyUp={onKeyUp}
          onBlur={() => held.current && end()}
        />
      </div>
    );
  }

  // A speed with no direction: the pill on the frame's bottom edge.
  const speed: SpeedSpec = handle.speed;
  const raw = numberOf(values[speed.key], speed.min);

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button > 0) return;
    e.stopPropagation();
    cancelPress(e, { focus: true });
    if (grip.current) return;
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // A pointer already gone has nothing to capture.
    }
    grip.current = { pointerId: e.pointerId, el: e.currentTarget, run: beginRun(speed.key), key: speed.key, startX: e.clientX, startRaw: raw, down: true };
    setDragging(true);
  };

  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    const current = grip.current;
    if (!current || current.pointerId !== e.pointerId) return;
    if (e.pointerType === "mouse" && e.buttons === 0) {
      end();
      return;
    }
    const next = scrubSpeed(speed, current.startRaw, e.clientX - current.startX);
    onChange(speed.key, { type: "number", value: next }, current.run, true);
  };

  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    if (grip.current?.pointerId !== e.pointerId) return;
    end();
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    e.stopPropagation();
    const faster = e.key === "ArrowRight";
    const again = e.repeat && held.current?.key === e.key;
    const run = again ? held.current!.run : beginRun(speed.key);
    held.current = { key: e.key, run };
    onChange(speed.key, { type: "number", value: stepSpeed(speed, raw, faster ? 1 : -1) }, run, true);
  };

  const onKeyUp = (e: KeyboardEvent<HTMLDivElement>) => {
    if (held.current?.key !== e.key) return;
    end();
  };

  const fraction = speedFraction(speed, raw);
  // A period is said as a speed, so the reader hears which way is faster: a shorter period is a
  // faster effect, and ArrowRight steps it down (#257 review).
  const valueText = speed.inverse
    ? `Speed: ${fraction > 0.5 ? "faster" : fraction < 0.5 ? "slower" : "middling"} (${speed.label} ${formatSpeed(raw)})`
    : `${speed.label} ${formatSpeed(raw)}`;
  return (
    <div
      className="rd-speed-pill"
      data-handle="speed"
      data-dragging={dragging ? "true" : "false"}
      role="slider"
      tabIndex={0}
      aria-label={`${speed.label} of ${layer.name}`}
      aria-valuemin={speed.min}
      aria-valuemax={speed.max}
      aria-valuenow={raw}
      aria-valuetext={valueText}
      title={`Scrub left and right to change the ${speed.label.toLowerCase()}; Left and Right step it.`}
      draggable={false}
      style={{ "--rd-speed-fill": `${Math.round(fraction * 100)}%` } as CSSProperties}
      onDragStart={refuseDragStart}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
      onLostPointerCapture={onUp}
      onKeyDown={onKey}
      onKeyUp={onKeyUp}
      onBlur={() => held.current && end()}
    >
      <span className="rd-speed-pill-inner">
        <span className="rd-speed-pill-fill" aria-hidden="true" />
        <span className="rd-speed-pill-label">{speed.label}</span>
        <span className="rd-speed-pill-value fig">{formatSpeed(raw)}</span>
      </span>
    </div>
  );
}

/** The dial's own words for a direction, so the handle and the inspector say the same thing. */
function directionWord(deg: number): string {
  const d = normaliseAngle(deg);
  if (d < 22.5 || d >= 337.5) return "right";
  if (d < 67.5) return "down-right";
  if (d < 112.5) return "down";
  if (d < 157.5) return "down-left";
  if (d < 202.5) return "left";
  if (d < 247.5) return "up-left";
  if (d < 292.5) return "up";
  return "up-right";
}
