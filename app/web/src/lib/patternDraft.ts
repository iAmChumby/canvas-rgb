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
