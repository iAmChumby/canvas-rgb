// The editor's one shared snapshot: daemon status, fans, devices, effects, layouts, render
// and settings, plus the last 60 one-second RPM samples per fan header. Polling runs only while
// the document is visible (status and fans every 1 s; the rest every 4 s), and an
// `canvas://state-changed` event refetches everything.

import { useSyncExternalStore } from "react";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { canUpdate, getUpdateState, onStateChanged, onUpdateState } from "@/lib/host";
import { setDemoUpdateListener } from "@/lib/demoUpdate";
import type { Status } from "@/proto/Status";
import type { FansStatus } from "@/proto/FansStatus";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { FailedEffect } from "@/proto/FailedEffect";
import type { Layout } from "@/proto/Layout";
import type { RenderState } from "@/proto/RenderState";
import type { Settings } from "@/proto/Settings";
import type { DisplayStatus } from "@/proto/DisplayStatus";
import type { UpdateState } from "@/proto/UpdateState";

export const RPM_HISTORY = 60;

/**
 * How long the editor waits before it tries the daemon again after the daemon refused it.
 * A daemon that is down does not need to be asked every second.
 */
export const DOWN_RETRY_MS = 5000;

/** When the daemon last refused to answer, 0 when it is answering. */
let lastStatusFailureAt = 0;

/** Failed status reads in a row. One blip keeps the last status on screen; a second one drops it. */
let statusFailures = 0;

/**
 * When the daemon was last tried and when the next try is due. The daemon-down page shows it,
 * so the page can say something new instead of repeating its own heading.
 */
export function statusRetry(): { lastTryAt: number; nextTryInMs: number } {
  if (!lastStatusFailureAt) return { lastTryAt: 0, nextTryInMs: 0 };
  return {
    lastTryAt: lastStatusFailureAt,
    nextTryInMs: Math.max(0, lastStatusFailureAt + DOWN_RETRY_MS - Date.now()),
  };
}

export interface Snap {
  status: Status | null;
  statusError: string | null;
  fans: FansStatus | null;
  fansError: string | null;
  devices: DeviceInfo[] | null;
  devicesError: string | null;
  /** How many devices the daemon listed the last time it answered, connected or not. The status
   * route knows only the connected ones, so this is the total the placeholder draws. */
  devicesTotal: number | null;
  effects: EffectSummary[] | null;
  effectsError: string | null;
  /** The user effect files that did not parse. */
  effectsFailed: FailedEffect[];
  layouts: Layout[] | null;
  activeLayout: string | null;
  layoutsError: string | null;
  render: RenderState | null;
  settings: Settings | null;
  settingsError: string | null;
  displays: DisplayStatus[] | null;
  displaysError: string | null;
  /** The daemon is older than the display routes: it answered 404, so it is not an error. */
  displaysUnsupported: boolean;
  rpm: Record<string, number[]>;
  /** The tray's update state. Null until the tray has answered, and in a browser with no fixture. */
  update: UpdateState | null;
  /** The areas with a load in flight after a Try again: the frame shows its placeholder while one
   * is asked for, and the button cannot be pressed twice. */
  pending: readonly string[];
}

let snap: Snap = {
  status: null,
  statusError: null,
  fans: null,
  fansError: null,
  devices: null,
  devicesError: null,
  devicesTotal: null,
  effects: null,
  effectsError: null,
  effectsFailed: [],
  layouts: null,
  activeLayout: null,
  layoutsError: null,
  render: null,
  settings: null,
  settingsError: null,
  displays: null,
  displaysError: null,
  displaysUnsupported: false,
  rpm: {},
  update: null,
  pending: [],
};

const listeners = new Set<() => void>();

export function getSnap(): Snap {
  return snap;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function patchSnap(partial: Partial<Snap>): void {
  snap = { ...snap, ...partial };
  for (const l of listeners) l();
}

/** Reads one field of the snapshot. Selectors must return existing references, not new objects. */
export function useSnap<T>(select: (s: Snap) => T): T {
  return useSyncExternalStore(
    subscribe,
    () => select(snap),
    () => select(snap),
  );
}

// ---- refreshers ----

/**
 * The areas with a load in flight after a Try again. The frame shows its placeholder while one
 * runs, so the error panel (and its button) is gone and a second press cannot start a second
 * load.
 */
let pendingAreas: string[] = [];

/**
 * The request each pending area is waiting for. The area is free again when that request has
 * settled: a request that never answers would leave the area pending for good, with no error
 * and no way to try again.
 */
const pendingRequest = new Map<string, number>();

/**
 * The number of the last request per area. It numbers the requests, so the pending marker can
 * tell which one the area is waiting for.
 */
const requests = new Map<string, number>();

/**
 * The number of the last answer that landed per area. An answer is dropped only when a NEWER
 * one has already landed: an answer that is slower than the poll must still reach the page,
 * even when a newer request is already on its way.
 */
const answered = new Map<string, number>();

/** Starts a request for an area and returns its number. */
function beginRequest(area: string): number {
  const n = (requests.get(area) ?? 0) + 1;
  requests.set(area, n);
  return n;
}

/**
 * True when this answer is not older than the one that landed last. The newest request on its
 * way does not count: only an answer that has already landed does.
 */
function isNewest(area: string, n: number): boolean {
  const last = answered.get(area);
  return last === undefined || n >= last;
}

/** Records that an answer landed: an older answer that arrives after it is dropped. */
function markAnswered(area: string, n: number): void {
  const last = answered.get(area);
  if (last === undefined || n > last) answered.set(area, n);
}

/** Marks an area as in flight, or not. */
function markPending(area: string, on: boolean): void {
  pendingAreas = on
    ? pendingAreas.includes(area)
      ? pendingAreas
      : [...pendingAreas, area]
    : pendingAreas.filter((a) => a !== area);
  patchSnap({ pending: pendingAreas });
}

/**
 * Runs a load the reader asked for again. The area is marked as in flight while it runs, so the
 * frame goes back to its placeholder and the button cannot be pressed twice. The area is free
 * again when the request it asked for has settled.
 */
async function retryArea(area: string, work: (n: number) => Promise<void>): Promise<void> {
  if (pendingAreas.includes(area)) return;
  const n = beginRequest(area);
  pendingRequest.set(area, n);
  markPending(area, true);
  try {
    await work(n);
  } finally {
    // The newest request for the area has settled: the frame may show its content again, and
    // the button comes back with it.
    if (pendingRequest.get(area) === n) {
      pendingRequest.delete(area);
      markPending(area, false);
    }
  }
}

export async function refreshStatus(n?: number): Promise<void> {
  const req = n ?? beginRequest("status");
  try {
    const status = await api.status();
    if (!isNewest("status", req)) return;
    markAnswered("status", req);
    patchSnap({ status, statusError: null });
    lastStatusFailureAt = 0;
    statusFailures = 0;
  } catch (e) {
    if (!isNewest("status", req)) return;
    markAnswered("status", req);
    statusFailures += 1;
    // The last status is old after the second miss in a row (about DOWN_RETRY_MS later). Clearing it
    // is what lets the strip and the daemon-down page say the daemon is gone.
    patchSnap(statusFailures >= 2 ? { status: null, statusError: messageOf(e) } : { statusError: messageOf(e) });
    lastStatusFailureAt = Date.now();
  }
}

export async function refreshFans(n?: number): Promise<void> {
  const req = n ?? beginRequest("fans");
  try {
    const fans = await api.fans();
    if (!isNewest("fans", req)) return;
    markAnswered("fans", req);
    const rpm = { ...snap.rpm };
    for (const h of fans.headers) {
      const prev = rpm[h.id] ?? [];
      rpm[h.id] = [...prev.slice(-(RPM_HISTORY - 1)), h.rpm ?? 0];
    }
    patchSnap({ fans, fansError: null, rpm });
  } catch (e) {
    if (!isNewest("fans", req)) return;
    markAnswered("fans", req);
    patchSnap({ fansError: messageOf(e) });
  }
}

export async function refreshDevices(n?: number): Promise<void> {
  const req = n ?? beginRequest("devices");
  try {
    const devices = (await api.devices()).devices;
    if (!isNewest("devices", req)) return;
    markAnswered("devices", req);
    patchSnap({ devices, devicesError: null, devicesTotal: devices.length });
  } catch (e) {
    if (!isNewest("devices", req)) return;
    markAnswered("devices", req);
    patchSnap({ devicesError: messageOf(e) });
  }
}

export async function refreshEffects(n?: number): Promise<void> {
  const req = n ?? beginRequest("effects");
  try {
    const r = await api.effects();
    if (!isNewest("effects", req)) return;
    markAnswered("effects", req);
    patchSnap({ effects: r.effects, effectsFailed: r.failed ?? [], effectsError: null });
  } catch (e) {
    if (!isNewest("effects", req)) return;
    markAnswered("effects", req);
    patchSnap({ effectsError: messageOf(e) });
  }
}

export async function refreshLayouts(n?: number): Promise<void> {
  const req = n ?? beginRequest("layouts");
  try {
    const r = await api.layouts();
    if (!isNewest("layouts", req)) return;
    markAnswered("layouts", req);
    patchSnap({ layouts: r.layouts, activeLayout: r.active, layoutsError: null });
  } catch (e) {
    if (!isNewest("layouts", req)) return;
    markAnswered("layouts", req);
    patchSnap({ layoutsError: messageOf(e) });
  }
}

export async function refreshRender(n?: number): Promise<void> {
  const req = n ?? beginRequest("render");
  try {
    const render = await api.render();
    if (!isNewest("render", req)) return;
    markAnswered("render", req);
    patchSnap({ render });
  } catch {
    // The status error already says whether the daemon answers.
  }
}

export async function refreshSettings(n?: number): Promise<void> {
  const req = n ?? beginRequest("settings");
  try {
    const settings = await api.settings();
    if (!isNewest("settings", req)) return;
    markAnswered("settings", req);
    patchSnap({ settings, settingsError: null });
  } catch (e) {
    if (!isNewest("settings", req)) return;
    markAnswered("settings", req);
    patchSnap({ settingsError: messageOf(e) });
  }
}

export async function refreshDisplays(n?: number): Promise<void> {
  const req = n ?? beginRequest("displays");
  try {
    const displays = await api.displays();
    if (!isNewest("displays", req)) return;
    markAnswered("displays", req);
    patchSnap({ displays, displaysError: null, displaysUnsupported: false });
  } catch (e) {
    if (!isNewest("displays", req)) return;
    markAnswered("displays", req);
    // A daemon that is older than the display routes answers 404. That is not a failure to read
    // the displays: the page says the daemon is old and what to do about it.
    const unsupported = e instanceof ApiFailure && e.status === 404;
    const message = unsupported
      ? "This service is older than the display support: it has no display routes. Update canvas-rgb and the tray app."
      : messageOf(e);
    patchSnap({ displaysError: message, displaysUnsupported: unsupported });
  }
}

/** Asks for a load again, the way the Try again button does. */
export const retryStatus = (): Promise<void> => retryArea("status", refreshStatus);
export const retryFans = (): Promise<void> => retryArea("fans", refreshFans);
export const retryDevices = (): Promise<void> => retryArea("devices", refreshDevices);
export const retryEffects = (): Promise<void> => retryArea("effects", refreshEffects);
export const retryLayouts = (): Promise<void> => retryArea("layouts", refreshLayouts);
export const retrySettings = (): Promise<void> => retryArea("settings", refreshSettings);
export const retryDisplays = (): Promise<void> => retryArea("displays", refreshDisplays);

/**
 * Reads the update state from the tray. A plain browser has no tray: with no `?update=` fixture
 * the snapshot stays null, and the controls say "Tray app only".
 */
export async function refreshUpdate(): Promise<void> {
  if (!canUpdate()) return;
  const n = beginRequest("update");
  try {
    const state = await getUpdateState();
    // A tray that answers late must not overwrite a newer answer.
    if (!isNewest("update", n)) return;
    markAnswered("update", n);
    if (state) patchSnap({ update: state });
  } catch {
    // A tray without the update commands: the editor shows nothing about updates.
  }
}

export async function refreshAll(): Promise<void> {
  await Promise.all([
    refreshStatus(),
    refreshFans(),
    refreshDevices(),
    refreshEffects(),
    refreshLayouts(),
    refreshRender(),
    refreshSettings(),
    refreshDisplays(),
  ]);
}

// ---- sync loop ----

function visible(): boolean {
  return typeof document === "undefined" || document.visibilityState === "visible";
}

/** Starts polling and event listening. Returns a stop function. */
export function startSync(): () => void {
  let fast: ReturnType<typeof setInterval> | null = null;
  let slow: ReturnType<typeof setInterval> | null = null;
  let unlisten: (() => void) | null = null;
  let updateUnlisten: (() => void) | null = null;
  let stopped = false;

  const tickFast = () => {
    if (!visible()) return;
    // A daemon that just refused is not asked again until its turn comes.
    if (lastStatusFailureAt && Date.now() < lastStatusFailureAt + DOWN_RETRY_MS) return;
    void refreshStatus();
    void refreshFans();
  };
  const tickSlow = () => {
    if (!visible()) return;
    if (lastStatusFailureAt && Date.now() < lastStatusFailureAt + DOWN_RETRY_MS) return;
    void refreshDevices();
    void refreshEffects();
    void refreshRender();
    // The layout can change outside the editor (the CLI, the daemon's own defaults), and a
    // stale desk canvas is worse than one extra request every four seconds.
    void refreshLayouts();
    void refreshSettings();
    void refreshDisplays();
  };
  const onVisibility = () => {
    if (visible()) void refreshAll();
  };

  void refreshAll();
  void refreshUpdate();
  // The tray pushes every change of the update state, so nothing polls it. The fixture pushes its own.
  setDemoUpdateListener((state) => patchSnap({ update: state }));
  void onUpdateState((state) => patchSnap({ update: state })).then((u) => {
    if (stopped) u();
    else updateUnlisten = u;
  });
  fast = setInterval(tickFast, 1000);
  slow = setInterval(tickSlow, 4000);
  document.addEventListener("visibilitychange", onVisibility);
  void onStateChanged(() => {
    void refreshAll();
  }).then((u) => {
    if (stopped) u();
    else unlisten = u;
  });

  return () => {
    stopped = true;
    if (fast) clearInterval(fast);
    if (slow) clearInterval(slow);
    document.removeEventListener("visibilitychange", onVisibility);
    if (unlisten) unlisten();
    if (updateUnlisten) updateUnlisten();
    setDemoUpdateListener(null);
  };
}
