// The setup checklist's rows (#579). Every tick is read from the latest `Status` and `FansStatus`;
// nothing here is stored, so a light that is unplugged or a scene that is deleted unticks itself.

import type { EffectSummary } from "@/proto/EffectSummary";
import type { FansStatus } from "@/proto/FansStatus";
import type { SafeStart } from "@/proto/SafeStart";
import type { Status } from "@/proto/Status";

/** The effect a first run plays by itself (`FIRST_RUN_EFFECT` in the daemon's `first_run.rs`). */
export const FIRST_RUN_EFFECT = "aurora";

/**
 * `done`: the daemon says yes. `todo`: it answered and says not yet. `unknown`: nothing answered,
 * so the row cannot be checked. `absent`: this PC cannot do it (#580's chip problem). Only `done`
 * lights the LED, and only `todo` holds "Done" back.
 */
export type RowState = "done" | "todo" | "unknown" | "absent";

export type RowId = "lights" | "scene" | "effect" | "fans";

export interface SetupRow {
  id: RowId;
  name: string;
  why: string;
  state: RowState;
  /** What was read, or why nothing could be. */
  value: string;
}

export interface SetupFacts {
  status: Status | null;
  fans: FansStatus | null;
  fansError: string | null;
  effects: EffectSummary[] | null;
}

type Read = Pick<SetupRow, "state" | "value">;

const NOT_READ: Read = { state: "unknown", value: "—" };

/** The characters one line of a row's monospace value holds in the narrowest row. */
export const VALUE_LINE_CHARS = 28;

/** A character's width in monospace columns: the wide scripts and emoji take two. */
function columns(word: string): number {
  let n = 0;
  for (const c of word) n += (c.codePointAt(0) ?? 0) >= 0x2e80 ? 2 : 1;
  return n;
}

/** True when `text` wraps by whole words into the two lines a row has for its value. */
export function fitsTwoLines(text: string): boolean {
  let lines = 1;
  let used = 0;
  for (const word of text.split(/\s+/)) {
    const wide = columns(word);
    if (wide > VALUE_LINE_CHARS) return false;
    if (used > 0 && used + 1 + wide > VALUE_LINE_CHARS) {
      lines += 1;
      used = wide;
    } else {
      used += (used > 0 ? 1 : 0) + wide;
    }
  }
  return lines <= 2;
}

/**
 * A value with a name in it, or `otherwise` when the name is too long to show whole. Nothing is
 * cut off: the name itself stands on the Scenes page.
 */
function named(value: string, otherwise: string): string {
  return fitsTwoLines(value) ? value : otherwise;
}

function lights(status: Status | null): Read {
  if (!status) return NOT_READ;
  if ((status.conflicts?.length ?? 0) > 0) {
    return { state: "todo", value: "SignalRGB has your lights. Quit it from the tray." };
  }
  // An engine that is not answering has no fresh device count: a number it still holds from
  // before the fake server went quiet must not tick the row (#579).
  if (status.openrgb === "connecting") return { state: "unknown", value: "Waking up your lights" };
  if (status.openrgb !== "connected") return { state: "unknown", value: "The light engine is not answering" };
  const found = status.devices_connected;
  if (typeof found !== "number") return NOT_READ;
  if (found >= 1) {
    const total = status.devices_total;
    return { state: "done", value: total >= found ? `${found} of ${total}` : `${found} found` };
  }
  return { state: "todo", value: "No lights found" };
}

/** The same sentences as the safe start banner (#577). */
function safeReason(safe: SafeStart): string {
  if (safe.reason === "user_chose") return "You chose safe start.";
  if (safe.reason === "unclean_once") return "The last run did not end cleanly.";
  if (safe.reason === "crash_loop") return `The last ${safe.unclean_streak} runs did not end cleanly.`;
  return "Safe start is on.";
}

function scene(status: Status | null): Read {
  if (!status) return NOT_READ;
  if (status.safe_start?.active) return { state: "todo", value: safeReason(status.safe_start) };
  const active = status.active_scene;
  if (active === undefined || !status.active_effects) return NOT_READ;
  if (active === null) return { state: "todo", value: "No scene is on" };
  if (status.active_effects.length === 0) {
    return { state: "todo", value: named(`${active.name}: nothing playing`, "Your scene: nothing playing") };
  }
  return { state: "done", value: named(active.name, "Your scene is playing") };
}

function effect(status: Status | null, effects: EffectSummary[] | null): Read {
  if (!status?.active_effects) return NOT_READ;
  // The onboarding's own effects are the app's, never a choice.
  const chosen = status.active_effects.find((id) => id !== FIRST_RUN_EFFECT && !id.startsWith("onboard-"));
  if (chosen) {
    const name = effects?.find((e) => e.id === chosen)?.name ?? chosen;
    return { state: "done", value: named(name, "Your pick is playing") };
  }
  if (status.active_effects.includes(FIRST_RUN_EFFECT)) {
    return { state: "todo", value: "Aurora, the first-run effect" };
  }
  return { state: "todo", value: "Nothing playing" };
}

function fanRead(status: Status | null, fans: FansStatus | null, fansError: string | null): Read {
  if (!status) return NOT_READ;
  if (status.service !== "connected") return { state: "unknown", value: "The fan service is not answering" };
  if (fansError || !fans) return { state: "unknown", value: "The fans are not read" };
  // An older service sends no field: a missing answer never reads as healthy.
  if (!fans.chip_checked) return { state: "unknown", value: "The fan chip is not checked" };
  if (fans.chip_problem === "pawn_io_unavailable") {
    return { state: "absent", value: "Not on this PC: PawnIO is unavailable" };
  }
  if (fans.chip_problem) return { state: "absent", value: "Not on this PC: no supported fan chip" };
  if (!fans.chip) return { state: "todo", value: "No fan chip yet" };
  const answering = fans.headers.filter((h) => h.available && h.rpm !== null).length;
  if (answering === 0) return { state: "todo", value: "No header reports a speed" };
  return { state: "done", value: `${answering} ${answering === 1 ? "header" : "headers"} answering` };
}

/** The four rows, always in this order. */
export function setupRows({ status, fans, fansError, effects }: SetupFacts): SetupRow[] {
  const sceneRead = scene(status);
  const effectRead = effect(status, effects);
  // Aurora alone is "not yet": the row says what would tick it, so a kept Aurora explains itself.
  const onAurora = effectRead.state === "todo" && status?.active_effects?.includes(FIRST_RUN_EFFECT) === true;
  return [
    { id: "lights", name: "Lights found", why: "The lights are what the app is for.", ...lights(status) },
    {
      id: "scene",
      name: "Your first scene",
      why:
        sceneRead.state === "done"
          ? "canvas-rgb made a scene of every light and it is playing."
          : "A scene is what makes the whole PC react.",
      ...sceneRead,
    },
    {
      id: "effect",
      name: "An effect you chose",
      why: onAurora
        ? "Pick any other effect on Scenes to tick this."
        : "Pick one for a group on Scenes. It is your PC.",
      ...effectRead,
    },
    {
      id: "fans",
      name: "Fans answering",
      why: "Fans are the other half of the app.",
      ...fanRead(status, fans, fansError),
    },
  ];
}

export function doneCount(rows: readonly SetupRow[]): number {
  return rows.filter((r) => r.state === "done").length;
}

/** Every row that can be done is done. A row that cannot be checked or done does not block. */
export function setupFinished(rows: readonly SetupRow[]): boolean {
  return doneCount(rows) > 0 && rows.every((r) => r.state !== "todo");
}
