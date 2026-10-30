// The update fixture: lets every state of the update flow be seen in a plain browser, where there
// is no tray to ask. It is read only when the page runs without the Tauri host, and only when the
// address carries `?update=<name>`; inside the tray none of this runs.
//
// Names: idle, checking, up-to-date, available, snoozed, skipped, downloading, downloading-unknown,
// verifying, verify-failed, uac, declined, installing, offline, rate-limited, check-failed,
// install-failed, install-failed-text, updated. With a fixture the controls work on this in-memory state, so the
// flow can be clicked through: Update now walks the install phases.

import type { UpdateState } from "@/proto/UpdateState";
import type { UpdatePhase } from "@/proto/UpdatePhase";
import {
  CMD_UPDATE_CHECK_NOW,
  CMD_UPDATE_DISMISS_UPDATED,
  CMD_UPDATE_INSTALL,
  CMD_UPDATE_LATER,
  CMD_UPDATE_SET_AUTO,
  CMD_UPDATE_SKIP,
  CMD_UPDATE_STATE,
  CMD_UPDATE_UNSKIP,
} from "@/proto/tauri";

const CURRENT = "0.1.0";
const NEXT = "0.1.1";
const MB = 1024 * 1024;

export const DEMO_NOTES = [
  "### Added",
  "- Updates: canvas-rgb looks for a newer release at start and once a day, and installs it when you press **Update now**.",
  "- A What's new page that shows the notes of the release that is waiting.",
  "- `canvas-rgb.exe --after-update` starts the tray quietly after an install.",
  "",
  "### Changed",
  "- The tray menu gains an Update to 0.1.1 row while a release is waiting.",
  "- Settings gains an Updates panel with Check now.",
  "",
  "### Fixed",
  "- The editor no longer opens a window after the installer relaunches the tray.",
].join("\n");

function base(): UpdateState {
  return {
    current_version: CURRENT,
    auto_check: true,
    last_check: "2026-10-29T09:14:00Z",
    phase: { kind: "idle" },
    available: null,
    skipped_version: null,
    snoozed: false,
    updated_to: null,
  };
}

function info() {
  return { version: NEXT, notes: DEMO_NOTES, pub_date: "2026-10-29T08:00:00Z" };
}

/** The state a fixture name stands for, or null when the name is unknown. */
export function fixtureState(name: string): UpdateState | null {
  const s = base();
  const avail = (phase: UpdatePhase): UpdateState => ({ ...s, available: info(), phase });
  switch (name) {
    case "idle":
      return s;
    case "checking":
      return { ...s, phase: { kind: "checking" } };
    case "up-to-date":
      return { ...s, phase: { kind: "up_to_date" } };
    case "available":
      return avail({ kind: "available" });
    case "snoozed":
      return { ...avail({ kind: "available" }), snoozed: true };
    case "skipped":
      return { ...avail({ kind: "available" }), skipped_version: NEXT };
    case "downloading":
      return avail({ kind: "downloading", downloaded: Math.round(4.2 * MB), total: Math.round(11.8 * MB) });
    case "downloading-unknown":
      return avail({ kind: "downloading", downloaded: Math.round(4.2 * MB), total: null });
    case "verifying":
      return avail({ kind: "verifying" });
    case "verify-failed":
      return avail({ kind: "verify_failed" });
    case "uac":
      return avail({ kind: "waiting_for_uac" });
    case "declined":
      return avail({ kind: "declined" });
    case "installing":
      return avail({ kind: "installing" });
    case "offline":
      return { ...s, phase: { kind: "check_failed", reason: "offline" } };
    case "rate-limited":
      return { ...s, phase: { kind: "check_failed", reason: "rate_limited" } };
    case "check-failed":
      return { ...s, phase: { kind: "check_failed", reason: "other" } };
    case "install-failed":
      return avail({ kind: "install_failed", message: "the installer exited with code 5" });
    case "install-failed-text":
      return avail({ kind: "install_failed", message: "Setup could not replace canvas-rgb.exe because it is in use" });
    case "updated":
      return { ...s, current_version: NEXT, updated_to: info(), phase: { kind: "up_to_date" } };
    default:
      return null;
  }
}

/** The fixture name in the address, or null (also on the server, where there is no address). */
export function demoUpdateName(): string | null {
  if (typeof window === "undefined") return null;
  const v = new URLSearchParams(window.location.search).get("update");
  return v && fixtureState(v) ? v : null;
}

/** True when a fixture is in the address: the controls then work on the in-memory state. */
export function demoUpdateActive(): boolean {
  return demoUpdateName() !== null;
}

let state: UpdateState | null = null;
let listener: ((s: UpdateState) => void) | null = null;
let timers: ReturnType<typeof setTimeout>[] = [];

export function setDemoUpdateListener(fn: ((s: UpdateState) => void) | null): void {
  listener = fn;
}

function current(): UpdateState {
  if (!state) state = fixtureState(demoUpdateName() ?? "idle") ?? base();
  return state;
}

function put(next: UpdateState): UpdateState {
  state = next;
  listener?.(next);
  return next;
}

function after(ms: number, fn: () => void): void {
  timers.push(setTimeout(fn, ms));
}

/** Update now in the fixture: download, check, wait, install, at a pace that can be watched. */
function walkInstall(): void {
  for (const t of timers) clearTimeout(t);
  timers = [];
  const total = Math.round(11.8 * MB);
  const steps = 12;
  for (let i = 0; i <= steps; i++) {
    after(i * 350, () => {
      put({ ...current(), phase: { kind: "downloading", downloaded: Math.round((total * i) / steps), total } });
    });
  }
  after((steps + 1) * 350, () => put({ ...current(), phase: { kind: "verifying" } }));
  after((steps + 1) * 350 + 1200, () => put({ ...current(), phase: { kind: "waiting_for_uac" } }));
  after((steps + 1) * 350 + 3000, () => put({ ...current(), phase: { kind: "installing" } }));
}

/** Answers one update command from the in-memory state. */
export function demoUpdateCommand(cmd: string, args?: Record<string, unknown>): UpdateState {
  const s = current();
  switch (cmd) {
    case CMD_UPDATE_STATE:
      return s;
    case CMD_UPDATE_CHECK_NOW:
      put({ ...s, phase: { kind: "checking" } });
      after(900, () => {
        const c = current();
        put({
          ...c,
          last_check: new Date().toISOString(),
          phase: c.available ? { kind: "available" } : { kind: "up_to_date" },
        });
      });
      return current();
    case CMD_UPDATE_INSTALL:
      walkInstall();
      return current();
    case CMD_UPDATE_LATER:
      return put({ ...s, snoozed: true });
    case CMD_UPDATE_SKIP:
      return put({ ...s, skipped_version: s.available?.version ?? null });
    case CMD_UPDATE_UNSKIP:
      return put({ ...s, skipped_version: null });
    case CMD_UPDATE_SET_AUTO:
      return put({ ...s, auto_check: args?.enabled === true });
    case CMD_UPDATE_DISMISS_UPDATED:
      return put({ ...s, updated_to: null });
    default:
      return s;
  }
}
