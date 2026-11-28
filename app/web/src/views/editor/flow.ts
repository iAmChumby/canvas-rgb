// The onboarding flow (flow.ts): which screens a run has, and when it opens by itself.
//
// One screen: the hello. It is the first run's greeting, with the count of lights found, and
// what it leads to is the Scenes page (#310: Layout, its case and its part-placing setup are
// gone; a first run makes a scene of every light).
//
// The app opens on Scenes while the daemon's `Status.onboarding_complete` is false: on the hello
// the first time, and from then on on the setup checklist (#579) beside the page, whose "Done" or
// "Hide checklist" sets the flag. No tick is stored. Outside that flag are only the editor's own
// UI marks: the hello has been seen (once per install), and, for one launch each, that Help asked
// for the checklist again and the two "Later"s (the checklist's, which is the hello's too, and the
// chain takeover's header setup).
//
// A "launch" is one webview session. `sessionStorage` lives exactly that long (it survives a
// reload and dies with the window), so the launch has an id kept there, and "Later" writes the
// id it was said in to `localStorage`. The same id next time means the same launch. Help's ask
// for the checklist is kept the same way.

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

/** The app opens on Scenes by itself while the owner has not finished the onboarding. */
export function opensOnScenes(status: Status | null): boolean {
  // A preview stays on the page it was loaded on.
  if (previewActive()) return false;
  return status !== null && !status.onboarding_complete && !laterThisLaunch();
}

/** The hello opens by itself once per install: the first time Scenes is reached that way. */
export function shouldOpen(status: Status | null): boolean {
  return opensOnScenes(status) && !helloSeen();
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

const HELLO_SEEN_KEY = "canvas-rgb.hello-seen";
const SETUP_KEY = "canvas-rgb.setup-offered";

/** The page's own copies, for a preview and for a write the storage refused. */
let memoryHelloSeen = false;
let memorySetup = false;

function storedHelloSeen(): boolean {
  try {
    return window.localStorage.getItem(HELLO_SEEN_KEY) === "1";
  } catch {
    return false;
  }
}

/**
 * Whether the hello was already behind the owner when this page was loaded, read before the page
 * can write it. A preview's page is its own: it starts with nothing seen.
 */
const helloSeenAtLoad = typeof window !== "undefined" && !previewActive() && storedHelloSeen();

/** True once the hello has been left, in this launch or an earlier one. */
export function helloSeen(): boolean {
  if (memoryHelloSeen) return true;
  return !previewActive() && storedHelloSeen();
}

/** The hello was left, by "Continue" or by "Set up later": it does not open by itself again. */
export function markHelloSeen(): void {
  if (previewActive()) memoryHelloSeen = true;
  try {
    storeLocal(HELLO_SEEN_KEY, "0");
  } catch {
    // Kept in memory for this page only.
    memoryHelloSeen = true;
  }
  setupChanged();
}

/**
 * "Welcome back" heads the checklist when the owner had seen the hello before this page, the
 * onboarding is not finished and something is done. A first launch never reads it.
 */
export function welcomeBack(status: Status | null, rows: readonly SetupRow[]): boolean {
  return !helloSeenAtLoad || (status !== null && !rows.some((r) => r.state === "done"));
}

/** True while Help's ask for the checklist holds: for the launch it was made in. */
function setupOffered(): boolean {
  if (memorySetup) return true;
  if (previewActive()) return false;
  try {
    return window.localStorage.getItem(SETUP_KEY) === launchId();
  } catch {
    return false;
  }
}

const setupListeners = new Set<() => void>();
let setupVersion = 0;

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

/** The checklist is on the page: asked for from Help, or not finished and not put off. */
export function setupVisible(status: Status | null): boolean {
  if (!status) return false;
  // `=== false`: a status that does not carry the field is not an unfinished onboarding.
  return setupOffered() || (status.onboarding_complete === false && !laterThisLaunch());
}

/** `setupVisible`, drawn again when a mark changes. */
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
  if (previewActive()) memorySetup = true;
  try {
    removeLocal(LATER_KEY);
    storeLocal(SETUP_KEY, launchId());
  } catch {
    // Kept in memory for this page only.
    memorySetup = true;
  }
  setupChanged();
}

/** "Done" or "Hide checklist": the daemon holds the fact from here; the ask is forgotten. */
export function closeSetup(): void {
  memorySetup = false;
  try {
    removeLocal(SETUP_KEY);
  } catch {
    // Nothing was remembered, so there is nothing to forget.
  }
  setupChanged();
}

/** "Set up later": shut until the app is started again. */
export function putOffSetup(): void {
  putOffUntilNextLaunch();
  closeSetup();
}
