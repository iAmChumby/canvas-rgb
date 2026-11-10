/**
 * Where a tooltip bubble goes, so it never runs off the window (#271).
 *
 * The bubble's CSS says where it would *like* to sit: above its trigger, centred on it (or
 * aligned to one end where a row has no room). This is the collision rule a floating-element
 * library applies (Floating UI's `flip` and `shift`, Radix's collision padding), written out
 * here because the editor takes no dependency for it:
 *
 * - along the trigger's side, the bubble keeps the alignment the CSS asked for, then slides
 *   along that edge until both ends are at least `margin` inside the bounds;
 * - across the gap, the bubble takes the preferred side, and flips to the opposite side when
 *   there is no room for it there and the other side has more room;
 * - a bubble wider or taller than the room it has cannot fit at all; it then keeps its start
 *   edge inside, which is the part a reader needs. The caller wraps text first.
 *
 * The function is pure: trigger box, bubble size and bounds in, a position out. Everything it
 * needs is a number, so the six placement cases are unit-tested without a browser.
 */

/** The least distance a bubble keeps from the edge of its bounds, in px. */
export const TIP_MARGIN = 8;

/** A flip that is within half a pixel of fitting stays where it is: subpixel noise is not room. */
const EPSILON = 0.5;

export type TipSide = "top" | "bottom" | "left" | "right";
/** Physical, not logical: `start` is the left (or top) edge, `end` the right (or bottom). The editor is left to right only. */
export type TipAlign = "start" | "center" | "end";

/** A box in viewport coordinates, as `getBoundingClientRect` reports it. */
export interface TipRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface TipSize {
  width: number;
  height: number;
}

export interface TipPlacementInput {
  /** The trigger (the `.tip` element the bubble is a child of). */
  trigger: TipRect;
  /** The bubble's own size, as laid out. */
  bubble: TipSize;
  /** Where the bubble may sit: the window, cut down by any ancestor that clips it. */
  bounds: TipRect;
  /** The side the CSS prefers. */
  side: TipSide;
  /** The alignment along the trigger's edge that the CSS prefers. */
  align: TipAlign;
  /** The gap the CSS keeps between the trigger and the bubble. */
  gap: number;
  /** The least distance the bubble keeps from the bounds' edges. Defaults to `TIP_MARGIN`. */
  margin?: number;
}

export interface TipPlacement {
  /** The side the bubble ended up on: the preferred one, or its opposite after a flip. */
  side: TipSide;
  /** The bubble's top-left corner, in the same coordinates as the trigger. */
  left: number;
  top: number;
}

/** The window as bounds: what a tooltip is kept inside when nothing clips it sooner. */
export function boundsOf(viewport: TipSize): TipRect {
  return { left: 0, top: 0, width: viewport.width, height: viewport.height };
}

const onX = (side: TipSide) => side === "left" || side === "right";
const across = (side: TipSide): TipSide =>
  side === "top" ? "bottom" : side === "bottom" ? "top" : side === "left" ? "right" : "left";

/** How far the bubble's start edge is from the trigger, on the side it is asked for. */
function startOn(side: TipSide, trigger: TipRect, bubble: TipSize, gap: number): number {
  switch (side) {
    case "top":
      return trigger.top - gap - bubble.height;
    case "bottom":
      return trigger.top + trigger.height + gap;
    case "left":
      return trigger.left - gap - bubble.width;
    case "right":
      return trigger.left + trigger.width + gap;
  }
}

/** The room on that side, between the trigger's edge and the bounds, gap and margin taken out. */
function roomOn(side: TipSide, trigger: TipRect, bounds: TipRect, gap: number, margin: number): number {
  switch (side) {
    case "top":
      return trigger.top - gap - (bounds.top + margin);
    case "bottom":
      return bounds.top + bounds.height - margin - (trigger.top + trigger.height + gap);
    case "left":
      return trigger.left - gap - (bounds.left + margin);
    case "right":
      return bounds.left + bounds.width - margin - (trigger.left + trigger.width + gap);
  }
}

/** The bubble's size along the placement axis (the axis it sits on one side of). */
const depth = (side: TipSide, bubble: TipSize) => (onX(side) ? bubble.width : bubble.height);

const clamp = (n: number, low: number, high: number) => Math.min(Math.max(n, low), Math.max(low, high));

export function placeTooltip(input: TipPlacementInput): TipPlacement {
  const { trigger, bubble, bounds, gap } = input;
  const margin = input.margin ?? TIP_MARGIN;
  const align = input.align;

  // The preferred side, unless it has no room and the other side has more: a flip.
  let side = input.side;
  const roomHere = roomOn(side, trigger, bounds, gap, margin);
  if (roomHere < depth(side, bubble) - EPSILON) {
    const other = across(side);
    if (roomOn(other, trigger, bounds, gap, margin) > roomHere) side = other;
  }

  // Across the gap: the preferred side, then pulled inside the bounds.
  const low = onX(side) ? bounds.left + margin : bounds.top + margin;
  const high =
    onX(side) ? bounds.left + bounds.width - margin - bubble.width : bounds.top + bounds.height - margin - bubble.height;
  const start = clamp(startOn(side, trigger, bubble, gap), low, high);

  // Along the trigger's edge: the preferred alignment, then slid inside the bounds.
  const centre = onX(side)
    ? trigger.top + trigger.height / 2 - bubble.height / 2
    : trigger.left + trigger.width / 2 - bubble.width / 2;
  const aligned =
    align === "start"
      ? onX(side)
        ? trigger.top
        : trigger.left
      : align === "end"
        ? onX(side)
          ? trigger.top + trigger.height - bubble.height
          : trigger.left + trigger.width - bubble.width
        : centre;
  const min = onX(side) ? bounds.top + margin : bounds.left + margin;
  const max = onX(side)
    ? bounds.top + bounds.height - margin - bubble.height
    : bounds.left + bounds.width - margin - bubble.width;
  const along = clamp(aligned, min, max);

  return onX(side) ? { side, left: start, top: along } : { side, left: along, top: start };
}
