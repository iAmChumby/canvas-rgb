// The stage's direction and speed handles, read from an effect's own schema (#257 phase 3).
//
// The spec's rule, derived from the schema with no spine change (§4a): the first `angle` prop is
// the direction; a number whose key is `speed`, `scroll_speed` or `period` is the speed. An effect
// with both gets one vector handle (the angle is the direction and the arrow's length shows the
// speed, 16 px at the minimum and 72 px at the maximum). An effect with only a speed gets a pill
// on the frame's bottom edge. An effect with neither gets no handle.
//
// `period` is inverse: a long period is a slow effect, so the shortest period is the fastest and
// the arrow is longest at its minimum.
//
// Everything here is pure, so the mapping is tested over every built-in schema without a DOM.

import type { PropertyDef } from "@/proto/PropertyDef";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";

/** The arrow's length at the slowest and the fastest setting, in css pixels. */
export const ARROW_MIN_PX = 16;
export const ARROW_MAX_PX = 72;
/** One arrow-key press turns the direction this far; with Shift, this far. */
export const ANGLE_STEP_DEG = 1;
export const ANGLE_SNAP_DEG = 15;
/** A full sweep of the speed range, in css pixels of a scrub. */
export const SPEED_SWEEP_PX = 160;

/** The keys a speed is read from, in the order the mapping prefers them. */
export const SPEED_KEYS = ["speed", "scroll_speed", "period"] as const;

export interface DirectionSpec {
  key: string;
  label: string;
}

export interface SpeedSpec {
  key: string;
  label: string;
  min: number;
  max: number;
  step: number;
  /** `period`: a longer value is a slower effect, so the range reads backwards. */
  inverse: boolean;
}

export type EffectHandle =
  | { kind: "none" }
  | { kind: "vector"; direction: DirectionSpec; speed: SpeedSpec }
  | { kind: "pill"; speed: SpeedSpec }
  /** A direction and no speed (an HTML effect that declares only an `angle`): the arrow at rest. */
  | { kind: "direction"; direction: DirectionSpec };

const NONE: EffectHandle = { kind: "none" };

function speedSpecOf(def: PropertyDef): SpeedSpec | null {
  if (def.kind.type !== "number") return null;
  const { min, max, step } = def.kind;
  // A range that is not two finite ends cannot be drawn or scrubbed: no handle (#257 review).
  if (!Number.isFinite(min) || !Number.isFinite(max)) return null;
  return { key: def.key, label: def.label, min, max, step: step > 0 ? step : 1, inverse: def.key === "period" };
}

/**
 * The handle an effect's schema asks for: one vector when it has a direction and a speed, a pill
 * when it has only a speed, a direction alone when it has an angle and no speed, and none
 * otherwise. Schemas that have not arrived get none.
 *
 * The speed is the first of `SPEED_KEYS` that is a number with a finite range, so a `speed` of
 * another kind (a choice, say) does not hide a `scroll_speed` beside it (#257 review).
 */
export function handleOf(schema: PropertySchema | null | undefined): EffectHandle {
  const props = schema?.properties ?? [];
  const directionDef = props.find((def) => def.kind.type === "angle");
  let speed: SpeedSpec | null = null;
  for (const key of SPEED_KEYS) {
    const def = props.find((entry) => entry.key === key);
    if (!def) continue;
    const spec = speedSpecOf(def);
    if (spec) {
      speed = spec;
      break;
    }
  }
  const direction = directionDef ? { key: directionDef.key, label: directionDef.label } : null;
  if (direction && speed) return { kind: "vector", direction, speed };
  if (speed) return { kind: "pill", speed };
  if (direction) return { kind: "direction", direction };
  return NONE;
}

/** A prop's number: an angle, a hue or a plain number all read as one (older saves used a hue). */
export function numberOf(value: PropValue | undefined, fallback = 0): number {
  if (!value) return fallback;
  if (value.type === "angle" || value.type === "hue" || value.type === "number") {
    return Number.isFinite(value.value) ? value.value : fallback;
  }
  return fallback;
}

const clamp01 = (value: number): number => Math.min(1, Math.max(0, value));

/** A value rounded to its step, with the step's own decimals, held inside the range. */
export function quantise(value: number, min: number, max: number, step: number): number {
  const held = Math.min(max, Math.max(min, value));
  const decimals = Math.min(6, (String(step).split(".")[1] ?? "").length);
  return Number((Math.round(held / step) * step).toFixed(decimals));
}

/** Where a speed value sits in its range, 0 (slowest) to 1 (fastest). */
export function speedFraction(spec: SpeedSpec, raw: number): number {
  if (spec.max === spec.min) return 0;
  const t = (raw - spec.min) / (spec.max - spec.min);
  return clamp01(spec.inverse ? 1 - t : t);
}

/** The value at a speed fraction, rounded to the schema's step. */
export function speedValue(spec: SpeedSpec, fraction: number): number {
  const t = spec.inverse ? 1 - clamp01(fraction) : clamp01(fraction);
  return quantise(spec.min + t * (spec.max - spec.min), spec.min, spec.max, spec.step);
}

/** The vector handle's arrow: 16 px at the slowest, 72 px at the fastest. */
export function arrowLength(fraction: number): number {
  return ARROW_MIN_PX + (ARROW_MAX_PX - ARROW_MIN_PX) * clamp01(fraction);
}

/** An angle in 0..360, the canvas' own: 0 points right, 90 down. */
export function normaliseAngle(deg: number): number {
  const wrapped = deg % 360;
  // `+ 0` folds `-0` to `0`, so a rounded angle never reads as `-0` (`Object.is(-0, 0)` is false).
  return wrapped < 0 ? wrapped + 360 : wrapped + 0;
}

/** The direction from a centre to a point, in the canvas' degrees (0 right, 90 down). */
export function angleFromPoint(centre: { x: number; y: number }, point: { x: number; y: number }): number {
  const raw = (Math.atan2(point.y - centre.y, point.x - centre.x) * 180) / Math.PI;
  // Round first, then wrap: `Math.round(359.6)` is 360, which is 0, not an angle of 360 (#257 review).
  return normaliseAngle(Math.round(raw));
}

/** The direction snapped to 15° steps, as a Shift drag does. */
export function snapAngle(deg: number, step = ANGLE_SNAP_DEG): number {
  if (step <= 0) return normaliseAngle(Math.round(deg));
  return normaliseAngle(Math.round(deg / step) * step);
}

/** A box in client pixels, as `getBoundingClientRect` returns it. */
export interface ClientBox {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/**
 * The arrow's length clamped so its grip's centre stays inside `box` (#257 review): a small frame
 * at the picture's edge can put the grip outside the stage, where `overflow: hidden` clips it and
 * it cannot be grabbed. The angle is unchanged; only the length is shortened.
 */
export function clampArrow(
  dir: number,
  length: number,
  hub: { x: number; y: number } | null,
  box: ClientBox | null,
): number {
  if (!hub || !box) return length;
  const rad = (dir * Math.PI) / 180;
  const x = hub.x + length * Math.cos(rad);
  const y = hub.y + length * Math.sin(rad);
  // Already inside: keep the exact length, so no clamp introduces a float's worth of drift.
  if (x >= box.left && x <= box.right && y >= box.top && y <= box.bottom) return length;
  const cx = Math.min(box.right, Math.max(box.left, x));
  const cy = Math.min(box.bottom, Math.max(box.top, y));
  return Math.hypot(cx - hub.x, cy - hub.y);
}

/** A scrub of `dx` pixels across the pill: a full sweep is `SPEED_SWEEP_PX`. */
export function scrubSpeed(spec: SpeedSpec, start: number, dx: number, sweep = SPEED_SWEEP_PX): number {
  const span = sweep > 0 ? sweep : SPEED_SWEEP_PX;
  return speedValue(spec, speedFraction(spec, start) + dx / span);
}

/** One arrow-key step: `dir` 1 is faster, -1 slower. A period steps backwards. */
export function stepSpeed(spec: SpeedSpec, raw: number, dir: 1 | -1): number {
  const next = raw + dir * spec.step * (spec.inverse ? -1 : 1);
  return quantise(next, spec.min, spec.max, spec.step);
}

/** A speed's value for a readout: whole numbers whole, a step of 0.1 shown as one decimal. */
export function formatSpeed(value: number): string {
  return Number.isInteger(value) ? String(value) : String(Number(value.toFixed(2)));
}
