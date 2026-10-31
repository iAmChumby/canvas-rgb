// The peripheral shapes the editor works with, and the words it uses for them.

import type { PeripheralInfo } from "@/proto/PeripheralInfo";
import type { ReadProgress } from "@/proto/ReadProgress";
import type { LoadStage } from "@/components/Loading";

/**
 * A peripheral as the list shows it. The battery is an extra the daemon may add when the
 * device reports one; a daemon that does not send it leaves the field empty and the editor
 * shows no battery.
 */
export interface PeripheralRow extends PeripheralInfo {
  battery?: number | null;
  charging?: boolean;
}

/** The words the editor uses for a kind of peripheral. */
export function kindWord(kind: PeripheralInfo["kind"]): string {
  if (kind === "mouse") return "Mouse";
  if (kind === "game_controller") return "Game controller";
  return "Keyboard";
}

/**
 * The words for the ids a peripheral's Choice options carry. The id stays the value; these are
 * only what a person reads, in the Select, the Segmented and the read-only rows.
 */
const CHOICE_WORDS: Record<string, string> = {
  none: "None",
  start: "Start",
  select: "Select",
  menu: "Menu / Share",
  home: "Home",
  screenshot: "Screenshot",
  swap: "Swap",
  other: "Other",
  a: "A",
  b: "B",
  x: "X",
  y: "Y",
  "left-stick-click": "Left stick click",
  "right-stick-click": "Right stick click",
  "left-bumper": "Left bumper (LB)",
  "right-bumper": "Right bumper (RB)",
  "left-trigger": "Left trigger",
  "right-trigger": "Right trigger",
  "dpad-up": "D-pad up",
  "dpad-down": "D-pad down",
  "dpad-left": "D-pad left",
  "dpad-right": "D-pad right",
  "paddle-1": "Paddle 1 (P1)",
  "paddle-2": "Paddle 2 (P2)",
  "paddle-3": "Paddle 3 (P3)",
  "paddle-4": "Paddle 4 (P4)",
  "turbo-single": "Turbo (single)",
  "turbo-auto": "Turbo (auto)",
  "left-click": "Left click",
  "right-click": "Right click",
  "middle-click": "Middle click",
  "dpi-cycle": "DPI cycle",
  forward: "Forward",
  back: "Back",
  static: "Static",
  breathing: "Breathing",
  neon: "Neon",
  off: "Off",
  "color-breathing": "Colour breathing",
  "static-dpi": "Static (DPI colour)",
  "breathing-dpi": "Breathing (DPI colour)",
};

/** What a person reads for a choice id: a known word, or the id with its hyphens opened. */
export function choiceWord(id: string): string {
  const known = CHOICE_WORDS[id];
  if (known) return known;
  if (/^[a-z]+(-[a-z]+)+$/.test(id)) {
    const text = id.replace(/-/g, " ");
    return text.charAt(0).toUpperCase() + text.slice(1);
  }
  return id;
}

/** What a peripheral is called in a sentence about reading it: "mouse", "controller". */
export function nounOf(kind: PeripheralInfo["kind"]): string {
  if (kind === "mouse") return "mouse";
  if (kind === "game_controller") return "controller";
  return "keyboard";
}

/**
 * The words for the stage of a device read ("Waking the mouse", "Reading settings · 3 of 5").
 * `idle` is a read that has not reported yet: it reads as the settings being read, which is what
 * the daemon is doing.
 */
export function readStage(progress: ReadProgress | null, noun: string): LoadStage {
  const stage = progress?.stage ?? "idle";
  if (stage === "opening") return { label: `Opening the ${noun}` };
  if (stage === "waking") return { label: `Waking the ${noun}` };
  if (stage === "reading" && progress && progress.total > 0) {
    return { label: "Reading settings", done: progress.done, total: progress.total };
  }
  return { label: "Reading settings" };
}
