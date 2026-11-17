// Frames in time (#468): which frame of a pattern shows at a moment, and how one fades into the next,
// as the editor's preview draws it.
//
// The daemon plays a pattern with `Timeline::at` in `crates/canvas-core/src/pattern_play.rs`; this is
// the same function in TypeScript, checked against the same vectors (`__tests__/pattern.vectors.json`).
// A pattern is a pure function of the clock: `tau = t mod cycle` when it loops (`min(t, end)` when it
// does not), the frame is a binary search of the frame start times, and inside the frame's `hold` it
// shows solid. After the hold a Fade blends it into the next frame (the first, for the last frame of a
// looping pattern) over the transition's `ms`, paced by an easing, every LED mixed in OKLab. A Cut
// jumps at the end of the hold. The arithmetic is doubles, polynomial or `%`, in the Rust's own order.

import { fromOklab, toOklab, type Lab } from "@/lib/oklab";
import { spinColours } from "@/lib/spin";
import type { Easing } from "@/proto/Easing";
import type { Frame } from "@/proto/Frame";
import type { Pattern } from "@/proto/Pattern";

/** `PATTERN_FRAMES_MAX` from the spine. */
export const PATTERN_FRAMES_MAX = 256;
/** `PATTERN_HOLD_MS_MAX` from the spine: the longest one frame holds. */
export const HOLD_MS_MAX = 600_000;
/** `TRANSITION_MS_MAX` from the spine: the longest one transition takes. */
export const TRANSITION_MS_MAX = 60_000;

/** The pace of a fade, as the daemon's `ease`: `u` is how far through it, the result how far the colours have gone. */
export function ease(easing: Easing, u: number): number {
  const x = Number.isFinite(u) ? Math.min(1, Math.max(0, u)) : 0;
  switch (easing) {
    case "ease_in":
      return x * x;
    case "ease_out":
      return 1 - (1 - x) * (1 - x);
    case "ease_in_out":
      return x * x * (3 - 2 * x);
    default:
      return x;
  }
}

/** What a pattern shows at one moment: a frame solid, or one frame blending into the next. */
export type Beat = { kind: "hold"; frame: number } | { kind: "fade"; from: number; to: number; u: number };

/** When each frame starts, built once per document and read every animation frame. */
export interface Timeline {
  starts: number[];
  holds: number[];
  fades: number[];
  easings: Easing[];
  cycle: number;
  looping: boolean;
}

/** Whether the frames loop: true unless `playback` says otherwise. */
export function isLooping(pattern: Pick<Pattern, "playback">): boolean {
  return pattern.playback?.loop !== false;
}

/** The timeline of a pattern's frames: the daemon's `Timeline::of`. */
export function timelineOf(pattern: Pick<Pattern, "frames" | "playback">): Timeline {
  const n = pattern.frames.length;
  const looping = isLooping(pattern);
  const starts: number[] = [];
  const holds: number[] = [];
  const fades: number[] = [];
  const easings: Easing[] = [];
  let at = 0;
  pattern.frames.forEach((frame, i) => {
    const t = frame.transition;
    const fade = t !== undefined && t.kind === "fade" && (i + 1 < n || looping) ? t.ms : 0;
    starts.push(at);
    holds.push(frame.hold);
    fades.push(fade);
    easings.push(t?.easing ?? "linear");
    at += frame.hold + fade;
  });
  return { starts, holds, fades, easings, cycle: at, looping };
}

/** One pass through the frames, in ms: every hold plus every fade (the last one only when looping). */
export function cycleMs(pattern: Pick<Pattern, "frames" | "playback">): number {
  return timelineOf(pattern).cycle;
}

/** One frame, or no time in the cycle: the pattern shows its first frame and nothing else, ever. */
export function isStill(timeline: Timeline): boolean {
  return timeline.starts.length < 2 || timeline.cycle <= 0;
}

/** What shows `tMs` milliseconds into the effect. A time before 0 or not a number is 0. */
export function beatAt(timeline: Timeline, tMs: number): Beat {
  if (isStill(timeline)) return { kind: "hold", frame: 0 };
  const t = Number.isFinite(tMs) && tMs > 0 ? tMs : 0;
  let tau: number;
  if (timeline.looping) tau = t % timeline.cycle;
  else if (t >= timeline.cycle) return { kind: "hold", frame: timeline.starts.length - 1 };
  else tau = t;
  // The last frame that has started: a frame of no span shares its start with the next and is never found.
  let lo = 0;
  let hi = timeline.starts.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if ((timeline.starts[mid] as number) <= tau) lo = mid + 1;
    else hi = mid;
  }
  const i = Math.max(0, lo - 1);
  const into = tau - (timeline.starts[i] as number);
  const hold = timeline.holds[i] as number;
  const fade = timeline.fades[i] as number;
  if (into < hold || fade <= 0) return { kind: "hold", frame: i };
  const to = (i + 1) % timeline.starts.length;
  return { kind: "fade", from: i, to, u: ease(timeline.easings[i] as Easing, (into - hold) / fade) };
}

/** The OKLab of every colour of every frame, for a preview that blends the same frames every animation frame. */
export function frameLabs(frames: readonly Frame[]): Lab[][] {
  return frames.map((frame) => frame.colors.map((c) => toOklab(c)));
}

/** A picture and its OKLab, for a turn that blends the neighbours of the true colours (the daemon's `SpinSource`). */
export interface Picture {
  colours: string[];
  labs: Lab[];
}

/**
 * The picture a pattern shows `tMs` into its clock: the frame solid, or the blend of two with the
 * blended OKLab kept (so a spin of it blends the neighbours of the true colours, as the daemon's
 * `blend_from` does). `labs` (from [`frameLabs`]) saves the conversion on every call.
 */
export function pictureAt(
  pattern: Pick<Pattern, "frames" | "playback">,
  tMs: number,
  timeline: Timeline = timelineOf(pattern),
  labs?: readonly (readonly Lab[])[],
): Picture {
  const beat = beatAt(timeline, tMs);
  if (beat.kind === "hold") {
    const colours = (pattern.frames[beat.frame]?.colors ?? []).map((c) => c.toLowerCase());
    return { colours, labs: labs?.[beat.frame] ? [...(labs[beat.frame] as Lab[])] : colours.map((c) => toOklab(c)) };
  }
  const from = pattern.frames[beat.from]?.colors ?? [];
  const to = pattern.frames[beat.to]?.colors ?? [];
  const len = Math.min(from.length, to.length);
  const colours: string[] = [];
  const out: Lab[] = [];
  const u = beat.u;
  for (let i = 0; i < len; i++) {
    const a = from[i] as string;
    const b = to[i] as string;
    const la = labs?.[beat.from]?.[i] ?? toOklab(a);
    const lb = labs?.[beat.to]?.[i] ?? toOklab(b);
    if (a.toLowerCase() === b.toLowerCase() || u <= 0) {
      colours.push(a.toLowerCase());
      out.push(la);
    } else if (u >= 1) {
      colours.push(b.toLowerCase());
      out.push(lb);
    } else {
      const lab: Lab = [la[0] + (lb[0] - la[0]) * u, la[1] + (lb[1] - la[1]) * u, la[2] + (lb[2] - la[2]) * u];
      colours.push(fromOklab(lab));
      out.push(lab);
    }
  }
  return { colours, labs: out };
}

/**
 * The colours of a pattern `tMs` into its clock, one per LED, turned by `turnsPerS` laps a second
 * when it spins: the daemon's `eval`. A time is milliseconds; the spin's clock is the same time in
 * seconds, as the daemon reads one clock for both.
 */
export function evalPattern(
  pattern: Pick<Pattern, "frames" | "playback">,
  tMs: number,
  turnsPerS: number | null = null,
  timeline: Timeline = timelineOf(pattern),
  labs?: readonly (readonly Lab[])[],
): string[] {
  const picture = pictureAt(pattern, tMs, timeline, labs);
  return turnsPerS ? spinColours(picture.colours, turnsPerS, tMs / 1000, picture.labs) : picture.colours;
}
