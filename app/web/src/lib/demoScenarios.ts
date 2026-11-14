// Demo scenarios the editor's own pages need. The demo backend reads this file; no view ever
// reads it, so the editor's pages cannot tell a scenario from a real answer.
//
// The daemon sends the same shapes the scenarios produce, so a view that renders a state needs
// no demo knowledge at all.
//
// To wire this in, the demo backend needs these lines (and the import):
//
//   import {
//     answerDelayMs, applyDeviceScenarios, applyFanScenarios,
//     applyStatusScenarios, daemonDown, saveError,
//   } from "@/lib/demoScenarios";
//
//   // in initialState():
//
//   // in statusNow():
//   openrgb: openRgbDown() ? "disconnected" : "connected",
//
//   // in demoRequest(), before the router:
//   await new Promise((resolve) => setTimeout(resolve, answerDelayMs()));
//   if (daemonDown()) fail(503, "service_unavailable", "the daemon is not answering");
//   if (method !== "GET" && saveError()) fail(500, "internal", "the daemon refused the change");
//
//   // in the fans route:
//   return applyFanScenarios(fansStatus());

import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { FansStatus } from "@/proto/FansStatus";
import type { Group } from "@/proto/Group";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
import type { ScenesResponse } from "@/proto/ScenesResponse";
import type { Status } from "@/proto/Status";
import { SEEN_LIGHTS_KEY } from "@/lib/seenLights";

/** Why a header was handed back: the chip's own safety, not a request. */
export const HANDBACK_REASON = "the daemon's safety handback: the chip was too hot to trust";

/** The scenario tokens this file understands, next to the demo's own ones. */
export const DAEMON_DOWN = "daemon-down";
export const OPENRGB_DOWN = "openrgb-down";
export const DEVICE_EMPTY = "device-empty";
export const SLOW = "slow";
export const SLOW_FPS = "slow-fps";
export const SAVE_ERROR = "save-error";
export const LINKS_CONNECTING = "links-connecting";
export const FAN_CONNECTING = "fan-connecting";
/** The onboarding's own states, for the screenshots and the walk. */
export const OB_CONNECTING = "connecting"; // OpenRGB never finishes starting
export const OB_WAKING = "waking"; // OpenRGB is starting for the first 4 s
export const OB_NO_DEVICES = "no-devices"; // OpenRGB is up and sees nothing
export const OB_FOUND_ON_RESCAN = "found-on-rescan"; // ...until the owner asks again
export const OB_CONFLICT_CLEARS = "conflict-clears"; // SignalRGB quits 6 s after the page opens
export const OB_COMPLETE = "complete"; // the onboarding was finished already

/** The scenario tokens in the query string, the same way the demo reads them. */
function tokens(): string[] {
  if (typeof window === "undefined") return [];
  const raw = new URLSearchParams(window.location.search).get("state") ?? "";
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

/** True when the token is one of the demo's scenarios. */
function has(name: string): boolean {
  return tokens().includes(name);
}

/** The daemon does not answer at all: the editor cannot reach it. */
export function daemonDown(): boolean {
  return has(DAEMON_DOWN);
}

/** The daemon is up, OpenRGB is not connected. */
export function openRgbDown(): boolean {
  return has(OPENRGB_DOWN);
}

/** There is no layout yet: nothing can run. */
export function deviceEmpty(): boolean {
  return has(DEVICE_EMPTY);
}

/** Every answer takes long enough for the editor to show its skeletons. */
export function slow(): boolean {
  return has(SLOW);
}

/** The lights are running, but not at the rate they were asked for. */
export function slowFps(): boolean {
  return has(SLOW_FPS);
}

/** Every write is refused. */
export function saveError(): boolean {
  return has(SAVE_ERROR);
}

/** The links are still coming up: the OpenRGB bridge and the fan service. */
export function linksConnecting(): boolean {
  return has(LINKS_CONNECTING);
}

/** The fan service alone is still coming up. */
export function fanConnecting(): boolean {
  return has(FAN_CONNECTING);
}

/** OpenRGB stays "connecting" for good. */
export function openRgbConnecting(): boolean {
  return has(OB_CONNECTING);
}

/** OpenRGB is "connecting" for this many ms after the page opened, then connected. 0 when off. */
export function wakingMs(): number {
  return has(OB_WAKING) ? 4000 : 0;
}

/** OpenRGB is up and no device is found. */
export function noDevices(): boolean {
  return has(OB_NO_DEVICES) || has(OB_FOUND_ON_RESCAN);
}

/** The devices appear once the owner has asked for a rescan. */
export function foundOnRescan(): boolean {
  return has(OB_FOUND_ON_RESCAN);
}

/** SignalRGB quits this many ms after the page opened. 0 when it stays. */
export function conflictClearsMs(): number {
  return has(OB_CONFLICT_CLEARS) ? 6000 : 0;
}

/** The onboarding was finished already. */
export function onboardingDone(): boolean {
  return has(OB_COMPLETE);
}

/**
 * The loading states, one per area: `hold:<area>[:<ms>]` makes the GETs of that area wait (60 s
 * when no time is given, which is "still loading" for a screenshot), `fail:<area>` makes them
 * answer 500, `empty:<area>` makes them answer with nothing in them. The areas are the first
 * path segment (`fans`, `layouts`, `settings`, `displays`, `devices`) and `effects`, `effect`,
 * `peripherals`, `peripheral` for the list and the detail of the last two, and `thumbnail` for the
 * effect pictures alone.
 */
export function holdMs(area: string): number {
  for (const t of tokens()) {
    const [kind, name, ms] = t.split(":");
    if (kind === "hold" && name === area) return ms ? Number(ms) || 0 : 60000;
  }
  return 0;
}

/** True when the GETs of this area are made to fail. */
export function failsArea(area: string): boolean {
  return has(`fail:${area}`);
}

/** True when the GETs of this area answer with nothing in them. */
export function emptyArea(area: string): boolean {
  return has(`empty:${area}`);
}

/** A peripheral read that takes long enough to see its stages: opening, waking, five reports. */
export const SLOW_READ = "slow-read";

/** True when reads of a peripheral run through their stages. */
export function slowRead(): boolean {
  return has(SLOW_READ);
}

/**
 * `?slow=1500`: every answer of the demo takes this many milliseconds (capped at 10 s), so a
 * screenshot or a walk catches each wait in the loading kit. Demo only: the real daemon host never
 * reads the query string. `?slow` with no number is 1500.
 */
export function slowQueryMs(): number | null {
  if (typeof window === "undefined") return null;
  const raw = new URLSearchParams(window.location.search).get("slow");
  if (raw === null) return null;
  const ms = raw === "" ? 1500 : Number(raw);
  return Number.isFinite(ms) && ms >= 0 ? Math.min(ms, 10_000) : null;
}

/** How long the demo waits before it answers. */
export function answerDelayMs(): number {
  return slowQueryMs() ?? (slow() ? 700 : 2);
}

/** The header ids the demo should report as handed back to BIOS: "handback:<header id>". */
export function handbackHeaders(): string[] {
  return tokens()
    .filter((t) => t.startsWith("handback:"))
    .map((t) => t.slice("handback:".length))
    .filter(Boolean);
}

/** The reason a handed-back header gives: "handback-reason:<text>" (the text has no comma). */
export function handbackReason(): string {
  const t = tokens().find((x) => x.startsWith("handback-reason:"));
  return t ? t.slice("handback-reason:".length) : HANDBACK_REASON;
}

/** The header ids the demo reports as not on this board: "unavailable:<header id>". */
export function unavailableHeaders(): string[] {
  return tokens()
    .filter((t) => t.startsWith("unavailable:"))
    .map((t) => t.slice("unavailable:".length))
    .filter(Boolean);
}

/** `?state=fan-unread`: the CPU fan's speed and duty come back with no reading (#279). */
export const FAN_UNREAD = "fan-unread";
/** `?state=fan-stopped`: the CPU fan is read at 0 RPM and 0% duty (#279). */
export const FAN_STOPPED = "fan-stopped";

/** The header the two fan-reading scenarios change. */
const SCENARIO_HEADER = "cpu_fan";

/** Applies the fan scenarios to a status. */
export function applyFanScenarios(status: FansStatus): FansStatus {
  const ids = handbackHeaders();
  const unavailable = unavailableHeaders();
  let next = status;
  if (ids.length > 0 || unavailable.length > 0) {
    next = {
      ...status,
      headers: status.headers.map((h) => {
        // Issue #146: the owner's word on a header wins over the table: a header the owner
        // marked "show anyway" is on the board, whatever the table says.
        if (unavailable.includes(h.id) && h.override_available === null) {
          // Issue #128: the read register stays 0 for an unrouted channel, rpm 0, on BIOS.
          return { ...h, available: false, on_bios: true, override_reason: null, rpm: 0, pwm_percent: 0 };
        }
        if (ids.includes(h.id)) {
          return { ...h, on_bios: true, override_reason: h.override_reason ?? handbackReason() };
        }
        return h;
      }),
    };
  }
  // Issue #279: a header the chip did not report comes back with no reading, and one that was
  // read at 0 comes back as 0. The two must stay apart all the way to the page.
  if (has(FAN_UNREAD) || has(FAN_STOPPED)) {
    next = {
      ...next,
      headers: next.headers.map((h) =>
        h.id === SCENARIO_HEADER
          ? { ...h, rpm: has(FAN_UNREAD) ? null : 0, pwm_percent: has(FAN_UNREAD) ? null : 0 }
          : h,
      ),
    };
  }
  return next;
}

/** Applies the status scenarios: OpenRGB is not connected, or the lights run slow. */
export function applyStatusScenarios(status: Status): Status {
  let next = status;
  if (openRgbDown()) next = { ...next, openrgb: "down" };
  if (slowFps()) next = { ...next, fps_measured: 12 };
  if (linksConnecting()) next = { ...next, openrgb: "connecting", service: "connecting" };
  if (openRgbConnecting()) next = { ...next, openrgb: "connecting" };
  if (fanConnecting()) next = { ...next, service: "connecting" };
  return next;
}

/** Applies the device scenarios: the first device reports no zones. */
export function applyDeviceScenarios(devices: DeviceInfo[]): DeviceInfo[] {
  if (noDevices()) return [];
  if (!deviceEmpty()) return devices;
  return devices.map((d, i) => (i === 0 ? { ...d, zones: [] } : d));
}

// ---- scenes (0.14.0) ----
//
// The Scenes page's states, one token each. The demo's own scenes already show four groups with
// an unassigned light (Gaming, the default), a group with a device that is not connected (its
// Memory group) and, in Evening, a group whose effect is gone.
//
//   scene:<id>    the active scene is that one (`scene:focus` is one group, `scene:evening` a failed one)
//   groups-0      the active scene has no group: every light is unassigned
//   groups-20     a 20-group scene "Stage" is active, on twenty-four more lights
//   long-names    the active scene and its first group carry 40-character names
//   new-light     a light appears that no group holds, and the page has not seen it before
//   heavy         with groups-20: every Stage light has 600 LEDs (the page's worst case for paint cost)

export const GROUPS_0 = "groups-0";
export const GROUPS_20 = "groups-20";
export const LONG_NAMES = "long-names";
export const NEW_LIGHT = "new-light";
export const HEAVY = "heavy";

/** The id the scene token names, if any. */
function sceneToken(): string | null {
  const t = tokens().find((x) => x.startsWith("scene:"));
  return t ? t.slice("scene:".length) : null;
}

/** A light on a device of its own: a strip of `leds` LEDs, connected. */
function demoStrip(id: string, label: string, leds: number): LightView {
  return {
    light: { id: `${id}#z0`, device_id: id, zone_index: 0, arrangement: "strip", reversed: false },
    led_count: leds,
    connected: true,
    label,
  };
}

/** Applies the scene scenarios to the demo's scenes and lights. */
export function applySceneScenarios(
  base: ScenesResponse,
  lights: LightView[],
): { scenes: Scene[]; active: string; lights: LightView[] } {
  const scenes = structuredClone(base.scenes);
  const all = structuredClone(lights);
  let active = base.active;
  const picked = sceneToken();
  if (picked && scenes.some((s) => s.id === picked)) active = picked;

  if (has(GROUPS_20)) {
    const effects = ["aurora", "rainbow_wave", "breathing", "spiral", "fire", "white"];
    const extra = Array.from({ length: 24 }, (_, i) =>
      demoStrip(`demo:stage-${i + 1}`, `Stage light ${i + 1}`, has(HEAVY) ? 600 : 8 + ((i * 5) % 29)),
    );
    all.push(...extra);
    const groups: Group[] = extra.slice(0, 20).map((l, i) => ({
      id: `g-stage-${i + 1}`,
      name: i === 0 ? "Key light" : `Stage group ${i + 1}`,
      members: [l.light.id],
      plays: { type: "effect", effect: effects[i % effects.length] ?? "aurora", props: {} },
      mode: i % 3 === 0 ? "each_its_own" : "flow_across",
      brightness: 100 - ((i * 7) % 60),
    }));
    scenes.push({ id: "stage", name: "Stage", rev: 1, groups });
    active = "stage";
  }

  const current = scenes.find((s) => s.id === active);
  if (current && has(GROUPS_0)) current.groups = [];
  // The long names are 40 characters on the dot, the most the daemon keeps.
  if (current && has(LONG_NAMES)) {
    current.name = [..."Evening wind-down, the whole case on, low"].slice(0, 40).join("");
    const first = current.groups[0];
    if (first) first.name = [..."Front intake and the radiator fans, top!"].slice(0, 40).join("");
  }

  if (has(NEW_LIGHT)) {
    const lamp = demoStrip("demo:desk-lamp", "Desk lamp", 12);
    // The page has seen every other light, so only this one is announced.
    try {
      window.localStorage.setItem(SEEN_LIGHTS_KEY, JSON.stringify(all.map((l) => l.light.id)));
    } catch {
      // The notice may not show: storage is not available here.
    }
    all.push(lamp);
  }
  return { scenes, active, lights: all };
}
