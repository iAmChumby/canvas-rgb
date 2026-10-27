// Demo scenarios the editor's own pages need. The demo backend reads this file; no view ever
// reads it, so the editor's pages cannot tell a scenario from a real answer.
//
// The daemon sends the same shapes the scenarios produce, so a view that renders a state needs
// no demo knowledge at all.
//
// To wire this in, the demo backend needs these lines (and the import):
//
//   import {
//     answerDelayMs, applyDeviceScenarios, applyFanScenarios, applyLayoutScenarios,
//     applyStatusScenarios, daemonDown, saveError,
//   } from "@/lib/demoScenarios";
//
//   // in initialState():
//   devices: applyDeviceScenarios(DEVICES),
//   layouts: noLayout() ? [] : [{ ...LAYOUT_DESK }],
//   activeLayout: noLayout() ? "" : LAYOUT_DESK.id,
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
import type { Layout } from "@/proto/Layout";
import type { Status } from "@/proto/Status";

/** Why a header was handed back: the chip's own safety, not a request. */
export const HANDBACK_REASON = "the daemon's safety handback: the chip was too hot to trust";

/** The scenario tokens this file understands, next to the demo's own ones. */
export const DAEMON_DOWN = "daemon-down";
export const OPENRGB_DOWN = "openrgb-down";
export const NO_LAYOUT = "no-layout";
export const DEVICE_EMPTY = "device-empty";
export const SLOW = "slow";
export const SLOW_FPS = "slow-fps";
export const SAVE_ERROR = "save-error";

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
export function noLayout(): boolean {
  return has(NO_LAYOUT);
}

/** One device reports no zones. */
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

/** How long the demo waits before it answers. */
export function answerDelayMs(): number {
  return slow() ? 700 : 2;
}

/** The header ids the demo should report as handed back to BIOS: "handback:<header id>". */
export function handbackHeaders(): string[] {
  return tokens()
    .filter((t) => t.startsWith("handback:"))
    .map((t) => t.slice("handback:".length))
    .filter(Boolean);
}

/** Applies the fan scenarios to a status. */
export function applyFanScenarios(status: FansStatus): FansStatus {
  const ids = handbackHeaders();
  if (ids.length === 0) return status;
  return {
    ...status,
    headers: status.headers.map((h) =>
      ids.includes(h.id)
        ? { ...h, on_bios: true, override_reason: h.override_reason ?? HANDBACK_REASON }
        : h,
    ),
  };
}

/** Applies the status scenarios: OpenRGB is not connected, or the lights run slow. */
export function applyStatusScenarios(status: Status): Status {
  let next = status;
  if (openRgbDown()) next = { ...next, openrgb: "down" };
  if (slowFps()) next = { ...next, fps_measured: 12 };
  return next;
}

/** Applies the device scenarios: the first device reports no zones. */
export function applyDeviceScenarios(devices: DeviceInfo[]): DeviceInfo[] {
  if (!deviceEmpty()) return devices;
  return devices.map((d, i) => (i === 0 ? { ...d, zones: [] } : d));
}

/** Applies the layout scenarios: there is no layout yet. */
export function applyLayoutScenarios(layouts: Layout[], active: string): { layouts: Layout[]; active: string } {
  if (!noLayout()) return { layouts, active };
  return { layouts: [], active: "" };
}
