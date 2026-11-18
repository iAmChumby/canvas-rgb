// What the editor puts on the lights (#481): the selected frame while the owner paints, the whole
// timeline while the owner plays it.
//
// The daemon plays whatever pattern the draft carries from the moment it lands: a new document
// installs a new effect, which starts its clock over and fits every frame of it. So a document of
// many frames sent on every stroke would animate the lights while the owner paints frame 2, and
// would fit every frame again for every stroke. A one-frame pattern stands still and costs one
// fit, which is what painting needs; Play is the owner asking to see the frames in time.

import type { Pattern } from "@/proto/Pattern";

/**
 * The document to put on the lights. Playing: the whole pattern, as it will be saved. Painting: the
 * selected frame alone, with the pattern's shape and motion (so a spin still turns), and with no
 * transition and no Loop, which a one-frame pattern has nothing to use them for. A pattern of one
 * frame is sent as it is.
 */
export function draftDocument(pattern: Pattern, selected: number, playing: boolean): Pattern {
  if (playing || pattern.frames.length <= 1) return pattern;
  const index = Math.min(Math.max(0, selected), pattern.frames.length - 1);
  const frame = pattern.frames[index];
  if (!frame) return pattern;
  const { transition: _transition, ...still } = frame;
  const { playback: _playback, ...rest } = pattern;
  return { ...rest, frames: [still] };
}

/** JSON with the keys of every object in order, so two documents with the same content read alike. */
function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, v) =>
    v !== null && typeof v === "object" && !Array.isArray(v)
      ? Object.fromEntries(Object.entries(v as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
      : v,
  );
}

/**
 * What decides when a pattern's picture moves: the same fields the engine compares across an install
 * (`PatternTiming` in `canvas-core`), the pattern, its motion, how its frames play and each frame's hold
 * and transition. Not the colours. A document with the key of the one running keeps its clock on the
 * lights, so a spinning ring painted stroke by stroke does not start over with every stroke.
 */
export function timingKey(pattern: Pattern): string {
  return canonical({
    id: pattern.id,
    motion: pattern.motion,
    playback: pattern.playback ?? null,
    frames: pattern.frames.map((f) => [f.hold, f.transition ?? null]),
  });
}
