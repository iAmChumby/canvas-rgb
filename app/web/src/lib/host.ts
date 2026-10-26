// The Tauri host bridge. The app runs as a static page inside the Tauri 2 window
// (withGlobalTauri), so commands and events go through window.__TAURI__ without an npm import.
// In a plain browser there is no host, and the editor runs on its demo backend instead.

import { CMD_GET_API_CONFIG, CMD_OPEN_LOGS_FOLDER, EVT_NAVIGATE, EVT_STATE_CHANGED } from "@/proto/tauri";
import type { ApiConfig } from "@/proto/ApiConfig";

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
