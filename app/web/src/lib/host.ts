// The Tauri host bridge. The app runs as a static page inside the Tauri 2 window
// (withGlobalTauri), so commands and events go through window.__TAURI__ without an npm import.
// In a plain browser there is no host, and the editor runs on its demo backend instead.

import {
  CMD_GET_API_CONFIG,
  CMD_OPEN_LOGS_FOLDER,
  CMD_UPDATE_CHECK_NOW,
  CMD_UPDATE_DISMISS_UPDATED,
  CMD_UPDATE_INSTALL,
  CMD_UPDATE_LATER,
  CMD_UPDATE_SET_AUTO,
  CMD_UPDATE_SKIP,
  CMD_UPDATE_STATE,
  CMD_UPDATE_UNSKIP,
  EVT_NAVIGATE,
  EVT_STATE_CHANGED,
  EVT_UPDATE_STATE,
} from "@/proto/tauri";
import type { ApiConfig } from "@/proto/ApiConfig";
import type { UpdateState } from "@/proto/UpdateState";
import { demoUpdateActive, demoUpdateCommand } from "@/lib/demoUpdate";

type Unlisten = () => void;

export interface TauriGlobal {
  core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
  event: {
    listen: (
      event: string,
      handler: (e: { payload: unknown }) => void,
    ) => Promise<Unlisten>;
  };
}

declare global {
  interface Window {
    __TAURI__?: TauriGlobal;
  }
}

export function tauriHost(): TauriGlobal | null {
  if (typeof window === "undefined") return null;
  return window.__TAURI__ ?? null;
}

export function isDemo(): boolean {
  return tauriHost() === null;
}

export async function fetchApiConfig(): Promise<ApiConfig> {
  const host = tauriHost();
  if (!host) throw new Error("no Tauri host");
  return (await host.core.invoke(CMD_GET_API_CONFIG)) as ApiConfig;
}

export async function openLogsFolder(): Promise<void> {
  const host = tauriHost();
  if (!host) return;
  await host.core.invoke(CMD_OPEN_LOGS_FOLDER);
}

/** Listens for the tray's "Open editor" page request. Returns an unlisten function. */
export async function onNavigate(handler: (page: string) => void): Promise<Unlisten> {
  const host = tauriHost();
  if (!host) return () => undefined;
  return host.event.listen(EVT_NAVIGATE, (e) => {
    const payload = e.payload as { page?: unknown } | null;
    if (payload && typeof payload.page === "string") handler(payload.page);
  });
}

/** Listens for the daemon's state-changed event (the editor refetches). */
export async function onStateChanged(handler: () => void): Promise<Unlisten> {
  const host = tauriHost();
  if (!host) return () => undefined;
  return host.event.listen(EVT_STATE_CHANGED, () => handler());
}

// ---- updates ----
// The tray owns the update flow; the editor shows its state and sends the person's choices. In a
// plain browser the `?update=` fixture (lib/demoUpdate.ts) stands in; without it nothing runs.

function isUpdateState(v: unknown): v is UpdateState {
  return typeof v === "object" && v !== null && typeof (v as { current_version?: unknown }).current_version === "string";
}

/** True when update controls can act: inside the tray, or in a browser with a fixture. */
export function canUpdate(): boolean {
  return tauriHost() !== null || demoUpdateActive();
}

/** Runs one update command. Returns the state when the command answers with one. */
async function updateCall(cmd: string, args?: Record<string, unknown>): Promise<UpdateState | null> {
  const host = tauriHost();
  if (!host) return demoUpdateActive() ? demoUpdateCommand(cmd, args) : null;
  const out = await host.core.invoke(cmd, args);
  return isUpdateState(out) ? out : null;
}

export const getUpdateState = () => updateCall(CMD_UPDATE_STATE);
export const updateCheckNow = () => updateCall(CMD_UPDATE_CHECK_NOW);
export const updateInstall = () => updateCall(CMD_UPDATE_INSTALL);
export const updateLater = () => updateCall(CMD_UPDATE_LATER);
export const updateSkip = () => updateCall(CMD_UPDATE_SKIP);
export const updateUnskip = () => updateCall(CMD_UPDATE_UNSKIP);
export const updateSetAuto = (enabled: boolean) => updateCall(CMD_UPDATE_SET_AUTO, { enabled });
export const updateDismissUpdated = () => updateCall(CMD_UPDATE_DISMISS_UPDATED);

/** Listens for the tray's update-state event. Returns an unlisten function. */
export async function onUpdateState(handler: (state: UpdateState) => void): Promise<Unlisten> {
  const host = tauriHost();
  if (!host) return () => undefined;
  return host.event.listen(EVT_UPDATE_STATE, (e) => {
    if (isUpdateState(e.payload)) handler(e.payload);
  });
}
