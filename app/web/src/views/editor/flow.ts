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

export type Step =
  | { kind: "hello" }
  | { kind: "case" }
  | { kind: "parts" }
  | { kind: "match" }
  | { kind: "finish" };

/** The part the flow is limited to, when one device is redone from its row. */
export interface Scope {
  deviceId: string;
  zoneIndex: number;
}

/**
 * The five screens of one run, parts first: Hello, Case, Your parts (put in what is in the PC),
 * Match the lights (which part is lit), Finish. A redo of one device is Match and Finish.
 */
export function buildSteps(_layout: Layout, _devices: DeviceInfo[] | null, scope: Scope | null): Step[] {
  if (scope) return [{ kind: "match" }, { kind: "finish" }];
  return [{ kind: "hello" }, { kind: "case" }, { kind: "parts" }, { kind: "match" }, { kind: "finish" }];
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
