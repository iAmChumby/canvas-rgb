// The onboarding flow (flow.ts): which screens a run has, and when it opens by itself.
//
// One screen: the hello. It is the first run's greeting, with the count of lights found, and
// what it leads to is the Scenes page (#310: Layout, its case and its part-placing setup are
// gone; a first run makes a scene of every light).
//
// The window opens while the daemon's `Status.onboarding_complete` is false. The hello leads to
// Scenes, where the setup checklist (#579) stands beside the page; its "Done" sets the flag. What
// is remembered outside it belongs to one launch: that the hello is behind the owner, that the
// checklist was asked for again, and the two "Later"s (the checklist's, which is the hello's too,
// and the chain takeover's header setup), each put off until the next app start.
//
// A "launch" is one webview session. `sessionStorage` lives exactly that long (it survives a
// reload and dies with the window), so the launch has an id kept there, and "Later" writes the
// id it was said in to `localStorage`. The same id next time means the same launch.

import { useSyncExternalStore } from "react";
import { previewActive } from "@/lib/previewMode";
import type { SetupRow } from "@/lib/setup";
import { removeLocal, storeLocal, storeSession } from "@/lib/storage";
import type { Status } from "@/proto/Status";

export type Step = { kind: "hello" };

/** The one screen of a run. */
export function buildSteps(): Step[] {
  return [{ kind: "hello" }];
}

/** The window opens by itself while the owner has not finished the onboarding. */
export function shouldOpen(status: Status | null): boolean {
  // A preview opens on the page and its checklist: the hello is where it was asked for.
  if (previewActive()) return false;
  return status !== null && !status.onboarding_complete && !laterThisLaunch() && !helloPassed();
}

const LAUNCH_KEY = "canvas-rgb.launch";
const LATER_KEY = "canvas-rgb.onboarding-later";
const SHOWN_KEY = "canvas-rgb.onboarding-shown";

let memoryLaunch: string | null = null;
let memoryLater = false;

/** A launch id no one has used yet. */
function newLaunchId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/**
 * The id of this launch: the same across a reload, new after the app is closed and opened.
 *
 * The session storage holds it, so a window opened from the tray (a new webview, with no session
 * storage of its own) makes a new one. The page's own copy is only the fallback for a browser
 * that keeps no session storage: there the id cannot change while the page lives, which is the
 * same rule.
 *
 * The page keeps no copy of the id while the session storage works, so that a launch ends with
 * the session storage and not with the page. That is what keeps the launch's own "Later" from
 * outliving the launch it was said in (#380), and so what leaves the header's own record as the
 * thing that keeps a header shut.
 */
export function launchId(): string {
  try {
    const kept = window.sessionStorage.getItem(LAUNCH_KEY);
    if (kept) return kept;
    // A preview writes nothing, so its id lives with the page.
    if (previewActive()) return (memoryLaunch ??= newLaunchId());
    const id = newLaunchId();
    storeSession(LAUNCH_KEY, id);
    return id;
  } catch {
    // Not remembered across a reload; still one id for this page.
    if (!memoryLaunch) memoryLaunch = newLaunchId();
    return memoryLaunch;
  }
}

/** True when the owner said "Later" in this launch. */
export function laterThisLaunch(): boolean {
  if (memoryLater) return true;
  // A preview's "Later" is its own: what the real launch put off says nothing here.
  if (previewActive()) return false;
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
    storeLocal(LATER_KEY, launchId());
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
    storeSession(SHOWN_KEY, "1");
  } catch {
    // The redirect may happen again after a reload; harmless.
  }
}

// ---- the chain setup's own "Later" (#338, #380) ----
//
// The first run's header setup is offered by the chain takeover on Scenes. Its "Later" is
// remembered the same way the hello's is: the launch that said it is written down, and the same
// launch does not offer the headers again. It lives only in `localStorage` and is compared with
// this launch's id, so a fresh app start (a new launch id) offers them again, and a reload keeps
// quiet. No memory copy: the store is the one truth here, which is also what lets a test start a
// new launch by clearing it.
//
// This is the launch's own shut. The header's own record is in `@/views/chain/offered`: the
// takeover remembers every header it has offered, so a header is offered by itself at most once,
// ever, and a window opened from the tray (a new launch) does not forget it (#380).

const CHAINS_LATER_KEY = "canvas-rgb.chains-later";
/** The preview's own "Later": it writes nothing. */
let previewChainsLater = false;

/** True when the owner said "Later" to the header setup in this launch. */
export function chainLaterThisLaunch(): boolean {
  if (previewActive()) return previewChainsLater;
  try {
    return window.localStorage.getItem(CHAINS_LATER_KEY) === launchId();
  } catch {
    return false;
  }
}

/** The owner said "Later": the takeover stays shut until the app is started again. */
export function putOffChainsUntilNextLaunch(): void {
  if (previewActive()) previewChainsLater = true;
  try {
    storeLocal(CHAINS_LATER_KEY, launchId());
  } catch {
    // Not remembered: the takeover may come again in this page.
  }
}

/** The standing panel's "Set up": the headers are offered again in this launch. */
export function offerChainsAgain(): void {
  previewChainsLater = false;
  try {
    removeLocal(CHAINS_LATER_KEY);
  } catch {
    // Nothing was remembered, so there is nothing to forget.
  }
}

// ---- the setup checklist (#579) ----
//
// No tick and no step is stored: the rows are read from the daemon (`@/lib/setup`).

const HELLO_KEY = "canvas-rgb.hello-passed";
const SETUP_KEY = "canvas-rgb.setup-offered";

/** The page's own copy of a flag the session storage does not hold (a preview, a refused write). */
const flags = new Map<string, boolean>();
const setupListeners = new Set<() => void>();
let setupVersion = 0;

function readFlag(key: string): boolean {
  const own = flags.get(key);
  if (own !== undefined) return own;
  // A preview's flags are its own: what the real launch did says nothing here.
  if (previewActive()) return false;
  try {
    return window.sessionStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

function writeFlag(key: string, on: boolean): void {
  if (previewActive()) {
    flags.set(key, on);
    return;
  }
  try {
    storeSession(key, on ? "1" : "0");
    // Written: the session storage is the truth, so a new launch (an empty one) starts clean.
    flags.delete(key);
  } catch {
    // Kept in memory for this page only.
    flags.set(key, on);
  }
}

function setupChanged(): void {
  setupVersion += 1;
  for (const l of setupListeners) l();
}

function subscribeSetup(listener: () => void): () => void {
  setupListeners.add(listener);
  return () => {
    setupListeners.delete(listener);
  };
}

/** True once the hello is behind the owner in this launch. */
export function helloPassed(): boolean {
  return readFlag(HELLO_KEY);
}

/** The hello's way on: Scenes and the checklist. */
export function passHello(): void {
  writeFlag(HELLO_KEY, true);
  setupChanged();
}

/** The checklist is on the page: asked for from Help, or not finished and not put off. */
export function setupVisible(status: Status | null): boolean {
  if (!status) return false;
  // `=== false`: a status that does not carry the field is not an unfinished onboarding.
  return readFlag(SETUP_KEY) || (status.onboarding_complete === false && !laterThisLaunch());
}

/** `setupVisible`, drawn again when a flag changes. */
export function useSetupVisible(status: Status | null): boolean {
  useSyncExternalStore(
    subscribeSetup,
    () => setupVersion,
    () => setupVersion,
  );
  return setupVisible(status);
}

/** Help's "Setup checklist": the checklist stands on Scenes again, finished or put off. */
export function offerSetupAgain(): void {
  memoryLater = false;
  try {
    removeLocal(LATER_KEY);
  } catch {
    // Nothing was remembered, so there is nothing to forget.
  }
  writeFlag(HELLO_KEY, true);
  writeFlag(SETUP_KEY, true);
  setupChanged();
}

/** "Done": the daemon holds the fact from here; the page forgets that it asked. */
export function closeSetup(): void {
  writeFlag(SETUP_KEY, false);
  setupChanged();
}

/** "Set up later": shut until the app is started again. */
export function putOffSetup(): void {
  putOffUntilNextLaunch();
  closeSetup();
}

let welcomeAtLaunch: boolean | null = null;

/**
 * "Welcome back": the onboarding is not finished and a row was already done when the app was
 * first read. Decided once per page load and stored nowhere.
 */
export function noteLaunch(status: Status | null, rows: readonly SetupRow[]): boolean {
  if (welcomeAtLaunch === null && status !== null) {
    welcomeAtLaunch = status.onboarding_complete === false && rows.some((r) => r.state === "done");
  }
  return welcomeAtLaunch ?? false;
}
