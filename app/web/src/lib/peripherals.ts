// The peripheral shapes the editor works with, and the words it uses for them.

import type { PeripheralDetail } from "@/proto/PeripheralDetail";
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

/** The daemon's words (`problem`) when reads are on and nothing has been read yet. */
export const NOT_READ_YET_PROBLEM = "the device's settings have not been read yet";

/** Values read longer ago than this are shown as stale (the daemon refreshes them when idle). */
export const STALE_AFTER_MS = 30 * 60 * 1000;

/**
 * True when nothing has been read from the device yet (#127): reads are on, the daemon has no
 * read time for it and keeps no values. The page then shows a frame that says so instead of
 * controls with settings it made up.
 */
export function isUnread(detail: PeripheralDetail): boolean {
  return detail.access.reads && detail.read_at_ms == null && Object.keys(detail.values).length === 0;
}

/**
 * "Read 3 min ago": when the values on the page were read, from the daemon's `read_at_ms` (Unix
 * ms) and the clock. `stale` is true past `STALE_AFTER_MS`.
 */
export function readAgo(readAtMs: number, nowMs: number): { text: string; stale: boolean } {
  const age = Math.max(0, nowMs - readAtMs);
  const minutes = Math.floor(age / 60_000);
  let text: string;
  if (age < 45_000) text = "Read just now";
  else if (minutes < 60) text = `Read ${Math.max(1, minutes)} min ago`;
  else if (minutes < 24 * 60) text = `Read ${Math.floor(minutes / 60)} h ago`;
  else text = `Read ${Math.floor(minutes / (24 * 60))} d ago`;
  return { text, stale: age > STALE_AFTER_MS };
}
