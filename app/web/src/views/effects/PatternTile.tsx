"use client";

// The library tile of a pattern: the device drawn as its own LEDs, in the pattern's colours.
//
// A pattern has no picture, so it never goes through the canvas thumbnail renderer: the dots are
// the picture, laid out by the editor's own `dotLayout` and drawn as one circle per LED. Nothing
// is requested: the tile is drawn from the document it is given, so a gallery of patterns makes
// no network call at all.
//
// The colours are the pattern's data, not the editor's chrome, so they are the hex values the
// daemon holds. An LED that is off is an unlit socket in the theme's own token, never a black
// circle: an off LED and a very dark one must not look the same.

import { useMemo } from "react";
import { BLACK_HEX, parseHex } from "@/lib/color";
import { kindWords } from "@/lib/patternFit";
import { layoutDots } from "@/views/patterns/dotLayout";
import type { BuiltOn } from "@/proto/BuiltOn";
import type { Pattern } from "@/proto/Pattern";
import s from "@/views/effects/newEffect.module.css";

/**
 * The box the dots are laid out in, in the drawing's own units: 16:10, the shape of the gallery
 * tile and of the inspector's preview. A larger tile scales the same drawing up.
 */
const BOX = { width: 168, height: 105 };

/** The smallest dot drawn: below this a circle is invisible, and the LED is still counted. */
const MIN_DRAWN_R = 0.6;

/** A pattern of that device, every LED off: the shape of a device before anything is painted. */
export function emptyPattern(builtOn: BuiltOn): Pattern {
  return {
    id: "pattern:",
    built_on: { ...builtOn },
    frames: [{ colors: Array.from({ length: Math.max(1, builtOn.led_count) }, () => BLACK_HEX), hold: 0 }],
    motion: { type: "none" },
  };
}

/** The name a pattern's id carries: `pattern:` and after it. */
export function patternName(id: string): string {
  return id.startsWith("pattern:") ? id.slice("pattern:".length) : id;
}

export function PatternTile({
  pattern,
  size = "md",
}: {
  /** The whole document: the tile is drawn from its frame's colours. */
  pattern: Pattern;
  size?: "sm" | "md" | "lg";
}) {
  const { built_on: builtOn } = pattern;
  const dots = useMemo(() => layoutDots(builtOn, BOX.width, BOX.height), [builtOn]);
  const colours = pattern.frames[0]?.colors ?? [];
  const lit = colours.filter((colour) => {
    const hex = parseHex(colour);
    return hex !== null && hex !== BLACK_HEX;
  }).length;
  const name = patternName(pattern.id);

  return (
    <svg
      className={s.tile}
      data-size={size}
      viewBox={`0 0 ${BOX.width} ${BOX.height}`}
      preserveAspectRatio="xMidYMid meet"
      role="img"
      aria-label={`${name}, ${kindWords(builtOn)}, ${lit} of ${builtOn.led_count} on`}
    >
      {dots.map((dot) => {
        const ink = parseHex(colours[dot.index] ?? BLACK_HEX);
        const on = ink !== null && ink !== BLACK_HEX;
        return (
          <circle
            key={dot.index}
            cx={dot.x}
            cy={dot.y}
            r={Math.max(MIN_DRAWN_R, dot.r - 0.35)}
            fill={on && ink !== null ? ink : "var(--led-off)"}
            // A socket big enough to see gets an edge, as the editor's own canvas draws it.
            {...(on || dot.r < 5 ? {} : { stroke: "var(--line-2)", strokeWidth: 0.5 })}
          />
        );
      })}
    </svg>
  );
}
