"use client";

import { CircleDashed, GridFour, LineSegment } from "@phosphor-icons/react";
import { Segmented, type SegOption } from "@/components/Segmented";
import type { Arrangement } from "@/proto/Arrangement";
import type { ZoneKind } from "@/proto/ZoneKind";

/** The shapes the owner picks between. `single` is what a one-LED light is, never a choice. */
export type Shape = Extract<Arrangement, "strip" | "ring" | "grid">;

const OPTIONS: Record<Shape, SegOption<Shape>> = {
  strip: { value: "strip", label: "Line", icon: <LineSegment size={14} weight="bold" aria-hidden="true" /> },
  ring: { value: "ring", label: "Ring", icon: <CircleDashed size={14} weight="bold" aria-hidden="true" /> },
  grid: { value: "grid", label: "Grid", icon: <GridFour size={14} weight="bold" aria-hidden="true" /> },
};

/**
 * The shapes a zone may be laid out as: a line of LEDs is a line or a ring (the hardware cannot
 * tell them apart, so the owner says), a matrix is also a grid, and a zone of one LED is not asked.
 * No zone known (a pattern built on a device that is not here) offers a line and a ring.
 */
export function shapesFor(zone: ZoneKind["type"] | undefined): Shape[] {
  if (zone === "single") return [];
  if (zone === "matrix") return ["strip", "ring", "grid"];
  return ["strip", "ring"];
}

/** The shape a light's arrangement is shown as: a `single` reads as the line it is drawn on. */
export function shapeOf(arrangement: Arrangement): Shape {
  return arrangement === "ring" || arrangement === "grid" ? arrangement : "strip";
}

/**
 * The shape picker (#468): how a light's LEDs are laid out, Ring, Line or Grid where the light is a
 * grid. The Devices row, the pattern editor's header and anywhere else a light's shape is chosen
 * use this one control; what it writes is `Light.arrangement`, through the lights route.
 */
export function ShapePicker({
  label,
  value,
  shapes,
  disabled,
  onChange,
}: {
  /** What a screen reader hears for the group: "What Fan 1 is". */
  label: string;
  value: Arrangement;
  /** Which shapes to offer; see [`shapesFor`]. */
  shapes: Shape[];
  disabled?: boolean;
  onChange: (shape: Shape) => void;
}) {
  if (shapes.length === 0) return null;
  return (
    <Segmented<Shape>
      label={label}
      value={shapeOf(value)}
      disabled={disabled}
      options={shapes.map((shape) => OPTIONS[shape])}
      onChange={onChange}
    />
  );
}
