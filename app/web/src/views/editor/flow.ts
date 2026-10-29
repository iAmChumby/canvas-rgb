// The onboarding flow (flow.ts): which screens a run has, and when it opens by itself.
//
// The flow works on the layout, like the setup always did: a part is mapped when its
// `part.mapped` says so, so what the owner has done survives a restart. The window opens while
// the daemon's `Status.onboarding_complete` is false; the owner reaching the end sets it. The
// only thing remembered outside the layout is "Set up later": the owner put it off until the
// next app start.
//
// A "launch" is one webview session. `sessionStorage` lives exactly that long (it survives a
// reload and dies with the window), so the launch has an id kept there, and "Later" writes the
// id it was said in to `localStorage`. The same id next time means the same launch.

import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";
import type { Status } from "@/proto/Status";
import { setupZones } from "@/views/editor/setup";

export type Step =
  | { kind: "hello" }
  | { kind: "case" }
  | { kind: "fans"; deviceId: string; zoneIndex: number }
  | { kind: "fixed"; zones: { deviceId: string; zoneIndex: number }[] }
  | { kind: "place" }
  | { kind: "finish" };

/** The part the flow is limited to, when one device is redone from its row. */
export interface Scope {
  deviceId: string;
  zoneIndex: number;
}

/**
 * The six screens of one run, worked out when the flow opens (and again when the owner leaves
 * the hello, because a rescan may have found lights). Hello, Case, one Fans screen per header
 * with several LEDs, one Fixed screen per fixed device, Place, Finish. A zone that is already mapped has no screen
 * of its own: the flow after "Set up later" picks up where it stopped.
 *
 * Place is the place for what is missing, too: it is a row there, not a screen.
 */
export function buildSteps(layout: Layout, devices: DeviceInfo[] | null, scope: Scope | null): Step[] {
  const zones = setupZones(layout, devices).filter((z) => z.placements.length > 0);
  if (scope) {
    const z = zones.find((x) => x.deviceId === scope.deviceId && x.zoneIndex === scope.zoneIndex);
    if (!z) return [{ kind: "finish" }];
    return [
      z.doubtful
        ? { kind: "fans", deviceId: z.deviceId, zoneIndex: z.zoneIndex }
        : { kind: "fixed", zones: [{ deviceId: z.deviceId, zoneIndex: z.zoneIndex }] },
      { kind: "finish" },
    ];
  }
  const steps: Step[] = [{ kind: "hello" }, { kind: "case" }];
  for (const z of zones.filter((x) => x.doubtful && !x.mapped)) {
    steps.push({ kind: "fans", deviceId: z.deviceId, zoneIndex: z.zoneIndex });
  }
  // One screen per fixed device: more devices means more screens, never more on one screen.
  const fixed = zones.filter((x) => !x.doubtful && !x.mapped);
  const seen: string[] = [];
  for (const z of fixed) if (!seen.includes(z.deviceId)) seen.push(z.deviceId);
  for (const deviceId of seen) {
    steps.push({
      kind: "fixed",
      zones: fixed.filter((z) => z.deviceId === deviceId).map((z) => ({ deviceId: z.deviceId, zoneIndex: z.zoneIndex })),
    });
  }
  steps.push({ kind: "place" }, { kind: "finish" });
  return steps;
}

/** The window opens by itself while the owner has not finished the onboarding. */
export function shouldOpen(status: Status | null): boolean {
  return status !== null && !status.onboarding_complete && !laterThisLaunch();
}

const LAUNCH_KEY = "canvas-rgb.launch";
const LATER_KEY = "canvas-rgb.onboarding-later";
const SHOWN_KEY = "canvas-rgb.onboarding-shown";

let memoryLaunch: string | null = null;
let memoryLater = false;

/** The id of this launch: the same across a reload, new after the app is closed and opened. */
export function launchId(): string {
  if (memoryLaunch) return memoryLaunch;
  let id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  try {
    const kept = window.sessionStorage.getItem(LAUNCH_KEY);
    if (kept) id = kept;
    else window.sessionStorage.setItem(LAUNCH_KEY, id);
  } catch {
    // Not remembered across a reload; still one id for this page.
  }
  memoryLaunch = id;
  return id;
}

/** True when the owner said "Later" in this launch. */
export function laterThisLaunch(): boolean {
  if (memoryLater) return true;
  try {
    return window.localStorage.getItem(LATER_KEY) === launchId();
  } catch {
    return false;
  }
}

/** The owner said "Later": the flow stays shut until the app is started again. */
export function putOffUntilNextLaunch(): void {
  memoryLater = true;
  try {
    window.localStorage.setItem(LATER_KEY, launchId());
  } catch {
    // Kept in memory for this page only.
  }
}

/** True once the first-page redirect has happened in this launch. */
export function redirectedThisLaunch(): boolean {
  try {
    return window.sessionStorage.getItem(SHOWN_KEY) === "1";
  } catch {
    return false;
  }
}

export function markRedirected(): void {
  try {
    window.sessionStorage.setItem(SHOWN_KEY, "1");
  } catch {
    // The redirect may happen again after a reload; harmless.
  }
}
