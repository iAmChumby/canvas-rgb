// What a part of a chain is called and what colour it is drawn in.
//
// The colours are data, not chrome: the daemon is told to light each part in one of them, and the
// flow's rows draw the same list, so a colour on the screen is the colour on the hardware. They
// are full-saturation hues, which stay apart on an LED (a dimmed or whitened colour washes out
// next to its neighbour), and every one is a long way from white: white is the colour step 1
// draws the chain in, so a part never reads as that. The first colour avoids the editor's blue,
// because it is the one that marks the last LED of the chain. These hex values and the white and
// orange the flow sends are the LEDs' own colours, not the editor's tokens: they are pixels on
// hardware, and the same value must reach the daemon and the screen.
//
// Which colour is which on real fans is checked in the hardware session (Task 12): the owner looks
// at seven fans and answers whether two neighbours look the same.

import { parseHex } from "@/lib/color";
import type { ChainPartKind } from "@/proto/ChainPartKind";
import type { Rgb8 } from "@/proto/Rgb8";

export const PART_COLOURS: readonly string[] = [
  "#ff4d00", // orange: the last LED of step 1, as far from white and the blue chrome as it gets
  "#00b8ff", // azure
  "#9dff00", // lime
  "#ff00a8", // magenta
  "#00e08a", // mint
  "#ffb300", // amber
  "#4d5dff", // indigo
  "#ff2d3d", // red
  "#21c400", // green
  "#d000ff", // violet
  "#f5ff00", // yellow
  "#00e5d0", // turquoise
];

/**
 * A colour as the wire takes it: `#rrggbb`, lower case (canvas-proto's `Rgb8` is that string).
 * A value that is not a colour is a mistake in the code that made it, so it is refused rather
 * than drawn black.
 */
export function hexToRgb8(hex: string): Rgb8 {
  const full = parseHex(hex);
  if (!full) throw new Error(`${hex} is not a #rrggbb colour`);
  return full;
}

/** The word a kind is named with: "Fan 1", "Strip 2", and "Device 3" for what the owner called other. */
function nounOf(kind: ChainPartKind): string {
  if (kind === "fan") return "Fan";
  if (kind === "strip") return "Strip";
  return "Device";
}

/** The name a part is given before the owner types one. `position` counts from 1 among all parts. */
export function defaultName(kind: ChainPartKind, position: number): string {
  return `${nounOf(kind)} ${position}`;
}

/**
 * The name after a row's kind changes. A name the owner typed is theirs and stays; one the app
 * gave ("Fan 1") follows the kind, so a fan turned into a strip reads "Strip 1".
 */
export function renameForKind(
  current: string,
  typedByOwner: boolean,
  kind: ChainPartKind,
  position: number,
): string {
  return typedByOwner ? current : defaultName(kind, position);
}
