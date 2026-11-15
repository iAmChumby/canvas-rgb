// The pattern's fit in the editor: which kind of landing a device gets, in words, and the
// document to save.
//
// The daemon owns the colours (`crates/canvas-core/src/pattern_fit.rs`); the editor never
// computes them. What it must agree on is the kind, because that is what the group's flags and the
// picker's marks say, and a mark that disagrees with what the lights do is a lie. The rule is
// `fit_kind` in `crates/canvas-proto/src/pattern.rs`, in its own order:
//
// - the same LED count and arrangement is `exact`;
// - a light of one LED is `one_led`, whatever its arrangement (it takes the first colour that is
//   on);
// - the same arrangement with another count is `stretched`;
// - anything else is `other_shape`, laid along the LED order.
//
// The shared vectors in `__tests__/patternFit.vectors.json` are the Rust tests' own cases, so the
// two agree by test and not by hope.

import type { Arrangement } from "@/proto/Arrangement";
import type { BuiltOn } from "@/proto/BuiltOn";
import type { FitKind } from "@/proto/FitKind";
import type { LightView } from "@/proto/LightView";
import type { Pattern } from "@/proto/Pattern";
import type { Rgb8 } from "@/proto/Rgb8";

/**
 * How a pattern built on `builtOn` lands on a light of `targetLeds` LEDs in `target` arrangement.
 * The daemon's `fit_kind`, rule for rule, in the same order.
 */
export function fitKind(builtOn: BuiltOn, targetLeds: number, target: Arrangement): FitKind {
  if (builtOn.led_count === targetLeds && builtOn.arrangement === target) return "exact";
  if (targetLeds === 1) return "one_led";
  if (builtOn.arrangement === target) return "stretched";
  return "other_shape";
}

/**
 * What a pattern was built on, in the words the editor shows beside it: "ring of 12", "strip of
 * 60", "grid 5 by 4", "single LED". A grid the daemon reports without its cell map says its LED
 * count instead of a shape it does not know.
 */
export function kindWords(builtOn: BuiltOn): string {
  if (builtOn.arrangement === "single") return "single LED";
  if (builtOn.arrangement === "grid") {
    const grid = builtOn.grid;
    return grid ? `grid ${grid.width} by ${grid.height}` : `grid of ${builtOn.led_count}`;
  }
  if (builtOn.arrangement === "ring") return `ring of ${builtOn.led_count}`;
  return `strip of ${builtOn.led_count}`;
}

/**
 * How a pattern lands on the lights of one group: how many take it exactly, how many are flagged
 * (stretched, another shape, or a single LED showing the first colour), and how many there are.
 * Nothing is refused, so `flagged` is the rest of `total`.
 */
export function groupFit(
  builtOn: BuiltOn,
  lights: LightView[],
): { exact: number; flagged: number; total: number } {
  let exact = 0;
  for (const light of lights) {
    if (fitKind(builtOn, light.led_count, light.light.arrangement) === "exact") exact += 1;
  }
  return { exact, flagged: lights.length - exact, total: lights.length };
}

/**
 * The document to save: the loaded pattern with its first frame's colours replaced, and everything
 * else spread as it came. A pattern a later version wrote keeps the fields this one does not know
 * (a second frame, a motion other than none, any later key), because the loaded object is spread
 * rather than rebuilt from the fields named here.
 *
 * The generated `Pattern` type cannot hold those keys (`#[ts(skip)]` on the flattened maps), so the
 * spread is what keeps them: do not replace it with a literal of the fields above.
 */
export function withColors(pattern: Pattern, colors: Rgb8[]): Pattern {
  const [first, ...rest] = pattern.frames;
  const frame = first ?? { colors: [], hold: 0 };
  return { ...pattern, frames: [{ ...frame, colors }, ...rest] };
}
