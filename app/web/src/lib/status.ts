"use client";

import type { EffectSummary } from "@/proto/EffectSummary";
import type { LinkState } from "@/proto/LinkState";
import type { Status } from "@/proto/Status";
import type { DaemonFault } from "@/lib/errors";

/**
 * What the app knows about itself, in plain words, in one place.
 *
 * The status strip and the command palette both say what is playing and what each link is
 * doing, so the words come from here: one answer, and the two places that show it cannot
 * disagree.
 */

export type Tone = "ok" | "warn" | "down" | "work";

export interface Problem {
  tone: Tone;
  text: string;
  /**
   * The words for a phone, where the row has room for the link's name but not for the whole
   * sentence. The mark's icon says the rest.
   */
  short?: string;
  /** #580: this entry is a hardware note. The strip counts every one, dismissed or not. */
  fault?: boolean;
  /** The note's key, so the strip can offer to show a hidden note again. */
  key?: string;
  /** True when this note was hidden by the owner. */
  dismissed?: boolean;
}

/** One link between the app and something else, and what it is doing. */
export interface Link {
  /** The link's own name: "OpenRGB", "Fan service", "The daemon". */
  name: string;
  /** Its state, in the daemon's own words. */
  state: string;
  tone: Tone;
}

/** The state of a link, in one word and one colour. */
function linkTone(state: LinkState | "online" | "offline"): Tone {
  if (state === "connected" || state === "online") return "ok";
  if (state === "down" || state === "offline") return "down";
  return "work";
}

/**
 * The three links the app has: the daemon it talks to, the OpenRGB bridge and the fan
 * service. Each one is named, so "connecting" always says which link is still coming up.
 *
 * The daemon that answered and said its render engine is not answering (#241) is
 * online: it is the engine that is not answering, and the link says so.
 */
export function links(status: Status | null, fault: DaemonFault | null): Link[] {
  const daemon = status
    ? "online"
    : fault === "engine"
      ? "online"
      : fault === "down"
        ? "offline"
        : "connecting";
  return [
    {
      name: "The daemon",
      state: daemon,
      tone: linkTone(daemon),
    },
    {
      name: "OpenRGB",
      state: status?.openrgb ?? "connecting",
      tone: linkTone(status?.openrgb ?? "connecting"),
    },
    {
      name: "Fan service",
      state: status?.service ?? "connecting",
      tone: linkTone(status?.service ?? "connecting"),
    },
  ];
}

/** What the scenes are doing wrong, for the status mark: groups that cannot light, and a read that failed. */
export interface SceneFaults {
  /** Groups of the active scene whose last write failed (a missing effect, a refused value). */
  dark: number;
  /** The scenes could not be read at all. */
  unread: boolean;
}

export const NO_FAULTS: SceneFaults = { dark: 0, unread: false };

/**
 * What is not working, worst first. An empty list means everything is fine.
 *
 * A link that is still connecting is not a fault, but it is not "all connected" either, so it
 * is said in words too - in the link's own name, so the reader knows which one is still
 * coming up. The words are short on purpose: the strip has one line, and a phone has one
 * line's worth of room.
 */
export function problems(
  status: Status | null,
  fault: DaemonFault | null,
  scenes: SceneFaults = NO_FAULTS,
): Problem[] {
  const out: Problem[] = [];
  if (fault === "engine") {
    // The daemon is up and its render engine is not answering (#241): it is the engine
    // that is not answering, and not the daemon.
    out.push({ tone: "warn", text: "Engine not answering", short: "Engine" });
    return out;
  }
  if (fault === "down" && !status) {
    out.push({ tone: "down", text: "Daemon offline", short: "Daemon" });
    return out;
  }
  if (!status) {
    out.push({ tone: "work", text: "Daemon connecting", short: "Daemon" });
    return out;
  }
  // #577: safe start is first: it changes what the app is doing to every light and fan.
  if (status.safe_start?.active) out.push({ tone: "warn", text: "Safe start", short: "Safe" });
  // #580 (R2, R15, C4): a fault array replaces the old OpenRGB entry only when the daemon sent
  // one. An older daemon has no field, so its old line stays. "Fan control off" always stays: it
  // means the service is down, which is a different thing. Dismissed faults stay here too.
  const faults = Array.isArray(status.faults) ? status.faults : null;
  if (faults) {
    for (const fault of faults) {
      out.push({
        tone: fault.kind === "open_rgb_down" ? "down" : "warn",
        text: fault.affected,
        short: fault.kind === "open_rgb_down" ? "OpenRGB" : "Fan chip",
        fault: true,
        key: fault.key,
        dismissed: fault.dismissed,
      });
    }
  } else if (status.openrgb === "down") {
    out.push({ tone: "down", text: "OpenRGB down", short: "OpenRGB" });
  }
  if (status.service === "down") out.push({ tone: "down", text: "Fan control off", short: "Fans" });
  if (scenes.dark > 0) {
    // A group whose effect cannot run is dark: the lights it holds sit unlit, so it is said in words.
    out.push({
      tone: "warn",
      text: scenes.dark === 1 ? "1 group dark" : `${scenes.dark} groups dark`,
      short: scenes.dark === 1 ? "Dark" : `${scenes.dark} dark`,
    });
  }
  if (scenes.unread) out.push({ tone: "warn", text: "Scenes not read", short: "Not read" });
  // The lights are running, but not at the rate they were asked for.
  if (status.power === "running" && !status.parked && status.fps_measured < status.fps_target - 1) {
    out.push({ tone: "warn", text: "Running slow" });
  }
  if (out.length === 0) {
    // Still coming up: the link is named, so "connecting" is never a mystery.
    if (status.openrgb === "connecting") {
      out.push({ tone: "work", text: "OpenRGB connecting", short: "OpenRGB" });
    } else if (status.service === "connecting") {
      out.push({ tone: "work", text: "Fan service connecting", short: "Fan service" });
    }
  }
  return out;
}

/** What a group's effect is called when the effects list does not hold it. */
export const MISSING_EFFECT = "Missing effect";

/** "html:glow-remo" as a reading name: the kind prefix off, words split, each capitalised. */
export function readableEffectId(id: string): string {
  const bare = id.replace(/^[a-z]+:/i, "").replace(/[-_]+/g, " ").trim();
  if (!bare) return MISSING_EFFECT;
  return bare
    .split(/\s+/)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

/**
 * An effect's name for a reader. The effects list names it; a list that has not arrived yet gives
 * a readable form of the id (so nothing flashes "missing" while it loads); a list that arrived
 * without it means the effect is gone, and the words say so. The raw id is never the answer: it
 * belongs in a tooltip.
 */
export function effectNameOf(effects: EffectSummary[] | null, id: string): string {
  const found = effects?.find((e) => e.id === id);
  if (found) return found.name;
  return effects === null ? readableEffectId(id) : MISSING_EFFECT;
}

/** The effects list has arrived and does not hold `id`: the effect is no longer installed. */
export function effectMissing(effects: EffectSummary[] | null, id: string): boolean {
  return effects !== null && !effects.some((e) => e.id === id);
}

/**
 * What is playing, in words. Scenes are the only render path (#310), so it is the active
 * scene's name and how many devices are lit. This is the same words the strip shows.
 */
export function playing(
  status: Status | null,
  effects: EffectSummary[] | null,
): { name: string; detail: string; count: boolean } {
  if (!status) return { name: "—", detail: "", count: false };
  if (!status.active_scene) return { name: "No scene yet", detail: "", count: false };
  const scene = status.active_scene.name;
  const effectName = (status.active_effects ?? []).map((id) => effectNameOf(effects, id)).join(", ");
  // A scene is named even when every group of it is off.
  const name = scene;
  if (!name) return { name: "Nothing is running", detail: "", count: false };
  if (!effectName) return { name: scene, detail: "all off", count: false };
  if (status.power === "paused") return { name, detail: "paused", count: false };
  if (status.parked) return { name, detail: "idle", count: false };
  const n = status.devices_connected;
  return { name, detail: `on ${n} ${n === 1 ? "device" : "devices"}`, count: true };
}

/** The playing line as one sentence: "Spiral on 5 devices". */
export function playingLine(status: Status | null, effects: EffectSummary[] | null): string {
  const p = playing(status, effects);
  return p.detail ? `${p.name} ${p.detail}` : p.name;
}
