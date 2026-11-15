// The onboarding flow (flow.ts): which screens a run has, and when it opens by itself.
//
// One screen: the hello. It is the first run's greeting, with the count of lights found, and
// what it leads to is the Scenes page (#310: Layout, its case and its part-placing setup are
// gone; a first run makes a scene of every light).
//
// The window opens while the daemon's `Status.onboarding_complete` is false; finishing the hello
// sets it. The only things remembered outside it are the two "Later"s: the hello's, and the chain
// takeover's header setup, each put off until the next app start.
//
// A "launch" is one webview session. `sessionStorage` lives exactly that long (it survives a
// reload and dies with the window), so the launch has an id kept there, and "Later" writes the
// id it was said in to `localStorage`. The same id next time means the same launch.

import type { Status } from "@/proto/Status";

export type Step = { kind: "hello" };

/** The one screen of a run. */
export function buildSteps(): Step[] {
  return [{ kind: "hello" }];
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

/** Marks the first-page redirect of this launch. */
export function markRedirected(): void {
  try {
    window.sessionStorage.setItem(SHOWN_KEY, "1");
  } catch {
    // The redirect may happen again after a reload; harmless.
  }
}

// ---- the chain setup's own "Later" (#338) ----
//
// The first run's header setup is offered by the chain takeover on Scenes. Its "Later" is
// remembered the same way the hello's is: the launch that said it is written down, and the same
// launch does not offer the headers again. It lives only in `localStorage` and is compared with
// this launch's id, so a fresh app start (a new launch id) offers them again, and a reload keeps
// quiet. No memory copy: the store is the one truth here, which is also what lets a test start a
// new launch by clearing it.

const CHAINS_LATER_KEY = "canvas-rgb.chains-later";

/** True when the owner said "Later" to the header setup in this launch. */
export function chainLaterThisLaunch(): boolean {
  try {
    return window.localStorage.getItem(CHAINS_LATER_KEY) === launchId();
  } catch {
    return false;
  }
}

/** The owner said "Later": the takeover stays shut until the app is started again. */
export function putOffChainsUntilNextLaunch(): void {
  try {
    window.localStorage.setItem(CHAINS_LATER_KEY, launchId());
  } catch {
    // Not remembered: the takeover may come again in this page.
  }
}

/** The standing panel's "Set up": the headers are offered again in this launch. */
export function offerChainsAgain(): void {
  try {
    window.localStorage.removeItem(CHAINS_LATER_KEY);
  } catch {
    // Nothing was remembered, so there is nothing to forget.
  }
}
