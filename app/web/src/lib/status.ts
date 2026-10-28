"use client";

import type { EffectSummary } from "@/proto/EffectSummary";
import type { LinkState } from "@/proto/LinkState";
import type { Status } from "@/proto/Status";

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
 */
export function links(status: Status | null, offline: boolean): Link[] {
  return [
    {
      name: "The daemon",
      state: status ? "online" : offline ? "offline" : "connecting",
      tone: linkTone(status ? "online" : offline ? "offline" : "connecting"),
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

/**
 * What is not working, worst first. An empty list means everything is fine.
 *
 * A link that is still connecting is not a fault, but it is not "all connected" either, so it
 * is said in words too - in the link's own name, so the reader knows which one is still
 * coming up. The words are short on purpose: the strip has one line, and a phone has one
 * line's worth of room.
 */
export function problems(status: Status | null, offline: boolean): Problem[] {
  const out: Problem[] = [];
  if (offline) {
    out.push({ tone: "down", text: "Daemon offline", short: "Daemon" });
    return out;
  }
  if (!status) {
    out.push({ tone: "work", text: "Daemon connecting", short: "Daemon" });
    return out;
  }
  if (status.openrgb === "down") out.push({ tone: "down", text: "OpenRGB down", short: "OpenRGB" });
  if (status.service === "down") out.push({ tone: "down", text: "Fan control off", short: "Fans" });
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

/**
 * What is playing, in words: the effect's name and how many devices it is on. This is the
 * same words the strip shows.
 */
export function playing(
  status: Status | null,
  effects: EffectSummary[] | null,
): { name: string; detail: string; count: boolean } {
  if (!status) return { name: "—", detail: "", count: false };
  if (!status.active_layout) return { name: "No layout yet", detail: "", count: false };
  const effectName = status.active_effect
    ? (effects?.find((e) => e.id === status.active_effect)?.name ?? status.active_effect)
    : "";
  if (!effectName) return { name: "Nothing is running", detail: "", count: false };
  if (status.power === "paused") return { name: effectName, detail: "paused", count: false };
  if (status.parked) return { name: effectName, detail: "idle", count: false };
  const n = status.devices_connected;
  return { name: effectName, detail: `on ${n} ${n === 1 ? "device" : "devices"}`, count: true };
}

/** The playing line as one sentence: "Spiral on 5 devices". */
export function playingLine(status: Status | null, effects: EffectSummary[] | null): string {
  const p = playing(status, effects);
  return p.detail ? `${p.name} ${p.detail}` : p.name;
}
