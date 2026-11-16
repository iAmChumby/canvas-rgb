// The demo's group preview (#378): the picture one group draws, in the daemon's own format
// (80x50 RGB, at most 15 a second). There is no daemon in a plain browser, so the demo draws
// the group's own look from the demo shades, with the group's brightness on it, as the LEDs
// get it.
//
// The rules the daemon keeps: a group set to Off is black, a group playing a pattern has no
// picture at all, and a group the scene does not hold sends nothing.

import { getSnap } from "@/lib/store";
import { demoFrameFor } from "@/lib/thumb-demo";
import { GROUP_PREVIEW_H, GROUP_PREVIEW_W } from "@/lib/groupPreview";

/** The id prefix of a pattern (`crates/canvas-proto/src/pattern.rs`). */
const PATTERN_ID_PREFIX = "pattern:";

/**
 * The demo's picture of one group, at the time `seconds`. Null when the group sends nothing:
 * a pattern has no canvas, and a group the scene does not hold is not there.
 */
export function demoGroupFrame(groupId: string, seconds: number): Uint8Array | null {
  const scenes = getSnap().scenes;
  const active = scenes?.scenes.find((s) => s.id === scenes.active);
  const group = active?.groups.find((g) => g.id === groupId);
  if (!group) return null;
  const rgb = new Uint8Array(GROUP_PREVIEW_W * GROUP_PREVIEW_H * 3);
  // A group set to Off is black, as the daemon sends it.
  if (group.plays.type !== "effect") return rgb;
  // A pattern is drawn on the lights, not on a canvas: it has no frame.
  if (group.plays.effect.startsWith(PATTERN_ID_PREFIX)) return null;
  const frame = demoFrameFor(group.plays.effect, seconds, GROUP_PREVIEW_W, GROUP_PREVIEW_H);
  // The group's brightness, as the LEDs get it.
  const level = group.brightness / 100;
  for (let i = 0; i < rgb.length; i += 1) rgb[i] = Math.round((frame[i] ?? 0) * level);
  return rgb;
}
