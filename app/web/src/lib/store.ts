// The editor's one shared snapshot: daemon status, fans, devices, effects, layouts, render
// and settings, plus the last 60 one-second RPM samples per fan header. Polling runs only while
// the document is visible (status and fans every 1 s; the rest every 4 s), and an
// `canvas://state-changed` event refetches everything.

import { useSyncExternalStore } from "react";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { onStateChanged } from "@/lib/host";
import type { Status } from "@/proto/Status";
import type { FansStatus } from "@/proto/FansStatus";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { Layout } from "@/proto/Layout";
import type { RenderState } from "@/proto/RenderState";
import type { Settings } from "@/proto/Settings";
import type { DisplayStatus } from "@/proto/DisplayStatus";

export const RPM_HISTORY = 60;

/**
 * How long the editor waits before it tries the daemon again after the daemon refused it.
 * A daemon that is down does not need to be asked every second.
 */
export const DOWN_RETRY_MS = 5000;

/** When the daemon last refused to answer, 0 when it is answering. */
let lastStatusFailureAt = 0;

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
  effects: EffectSummary[] | null;
  effectsError: string | null;
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
}

let snap: Snap = {
  status: null,
  statusError: null,
  fans: null,
  fansError: null,
  devices: null,
  devicesError: null,
  effects: null,
  effectsError: null,
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

export async function refreshStatus(): Promise<void> {
  try {
    patchSnap({ status: await api.status(), statusError: null });
    lastStatusFailureAt = 0;
  } catch (e) {
    patchSnap({ statusError: messageOf(e) });
    lastStatusFailureAt = Date.now();
  }
}

export async function refreshFans(): Promise<void> {
  try {
    const fans = await api.fans();
    const rpm = { ...snap.rpm };
    for (const h of fans.headers) {
      const prev = rpm[h.id] ?? [];
      rpm[h.id] = [...prev.slice(-(RPM_HISTORY - 1)), h.rpm ?? 0];
    }
    patchSnap({ fans, fansError: null, rpm });
  } catch (e) {
    patchSnap({ fansError: messageOf(e) });
  }
}

export async function refreshDevices(): Promise<void> {
  try {
    patchSnap({ devices: (await api.devices()).devices, devicesError: null });
  } catch (e) {
    patchSnap({ devicesError: messageOf(e) });
  }
}

export async function refreshEffects(): Promise<void> {
  try {
    const r = await api.effects();
    patchSnap({ effects: r.effects, effectsError: null });
  } catch (e) {
    patchSnap({ effectsError: messageOf(e) });
  }
}

export async function refreshLayouts(): Promise<void> {
  try {
    const r = await api.layouts();
    patchSnap({ layouts: r.layouts, activeLayout: r.active, layoutsError: null });
  } catch (e) {
    patchSnap({ layoutsError: messageOf(e) });
  }
}

export async function refreshRender(): Promise<void> {
  try {
    patchSnap({ render: await api.render() });
  } catch {
    // The status error already says whether the daemon answers.
  }
}

export async function refreshSettings(): Promise<void> {
  try {
    patchSnap({ settings: await api.settings(), settingsError: null });
  } catch (e) {
    patchSnap({ settingsError: messageOf(e) });
  }
}

export async function refreshDisplays(): Promise<void> {
  try {
    patchSnap({ displays: await api.displays(), displaysError: null, displaysUnsupported: false });
  } catch (e) {
    // A daemon that is older than the display routes answers 404. That is not a failure to read
    // the displays: the page says the daemon is old and what to do about it.
    const unsupported = e instanceof ApiFailure && e.status === 404;
    const message = unsupported
      ? "This daemon has no display routes: it is older than the display support. Update the daemon and the tray app."
      : messageOf(e);
    patchSnap({ displaysError: message, displaysUnsupported: unsupported });
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
  };
}
