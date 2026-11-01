// The words the update flow uses, in one place. They are the settled words of the update debate
// (.build/debate-update/CONSENSUS.md, "State words"); change them there first.

import type { DotTone } from "@/components/Panel";
import type { UpdateState } from "@/proto/UpdateState";

const MB = 1024 * 1024;

/** The one sentence the person reads before Windows asks for permission. */
export function uacLine(version: string): string {
  return `Windows will ask for permission to install ${version}. While it installs, canvas-rgb stops for a moment: the lights pause and the fans run on your motherboard's settings. Choosing No leaves everything as it is.`;
}

/** "09:14" for today, "Oct 28, 09:14" for another day. */
export function fmtChecked(iso: string | null): string {
  if (!iso) return "Never";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "Never";
  const time = d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === new Date().toDateString()) return time;
  return `${d.toLocaleDateString("en-US", { month: "short", day: "numeric" })}, ${time}`;
}

function mb(bytes: number): string {
  return (bytes / MB).toFixed(1);
}

/** True while the tray is downloading, checking the download, waiting for Windows or installing. */
export function isInstalling(u: UpdateState): boolean {
  const k = u.phase.kind;
  return k === "downloading" || k === "verifying" || k === "waiting_for_uac" || k === "installing";
}

/** Which Swap label of the Update now key is showing: 0 Update now ... 4 Installing. */
export function keyStep(u: UpdateState): number {
  switch (u.phase.kind) {
    case "downloading":
      return 1;
    case "verifying":
      return 2;
    case "waiting_for_uac":
      return 3;
    case "installing":
      return 4;
    case "install_failed":
      return 5;
    default:
      return 0;
  }
}

export const KEY_LABELS = [
  "Update now",
  "Downloading 100%",
  "Checking download",
  "Waiting for Windows",
  "Installing",
  "Try again",
];

/** The text of the Downloading label: a percent when the length is known, megabytes when not. */
export function downloadingLabel(u: UpdateState): string {
  const p = u.phase;
  if (p.kind !== "downloading") return KEY_LABELS[1] ?? "";
  if (p.total && p.total > 0) return `Downloading ${Math.floor((p.downloaded / p.total) * 100)}%`;
  return `Downloading ${mb(p.downloaded)} MB`;
}

/** How full the slot under the key is, 0 to 1, or null when the length is not known. */
export function slotFill(u: UpdateState): number | null {
  const p = u.phase;
  switch (p.kind) {
    case "downloading":
      return p.total && p.total > 0 ? Math.min(1, p.downloaded / p.total) : null;
    case "verifying":
    case "waiting_for_uac":
    case "installing":
      return 1;
    default:
      return 0;
  }
}

/**
 * What the installer said, when it said a sentence a person can read. An exit code, a hex code or
 * a bare word is not one, and gets the plain line instead.
 */
export function installerSentence(message: string): string {
  const m = message.trim();
  const raw = /exit(ed)?\s+(with\s+)?code|0x[0-9a-f]+|error\s*\d+|^\W*\d+\W*$/i.test(m);
  if (m === "" || raw || !/\s/.test(m)) return "The installer stopped before it finished.";
  const cap = m.charAt(0).toUpperCase() + m.slice(1);
  return /[.!?]$/.test(cap) ? cap : `${cap}.`;
}

/**
 * The sentence after an install that did not finish. The version is named only when it is not the
 * version the failed update was for: "canvas-rgb is still on 0.3.0" after an update *to* 0.3.0
 * reads as if the update never reached this PC (#60).
 */
export function installFailedSentence(message: string, current: string, failedFor?: string | null): string {
  const first = installerSentence(message);
  if (!current) return first;
  if (failedFor && sameVersion(failedFor, current)) return first;
  return `${first} canvas-rgb is still on ${current}.`;
}

/** "0.3.0" and "v0.3.0" are the same version. */
function sameVersion(a: string, b: string): boolean {
  const norm = (v: string) => v.trim().replace(/^v/i, "");
  return norm(a) === norm(b);
}

/** The state line of the updates page: what is happening, in the settled words. */
export function stateLine(u: UpdateState): string {
  const v = u.available?.version ?? "";
  const p = u.phase;
  switch (p.kind) {
    case "checking":
      return "Checking for updates…";
    case "downloading":
      return p.total && p.total > 0
        ? `Downloading ${v}: ${mb(p.downloaded)} of ${mb(p.total)} MB.`
        : `Downloading ${v}: ${mb(p.downloaded)} MB.`;
    case "verifying":
      return "Checking the download against canvas-rgb's release key.";
    case "verify_failed":
      return "The download failed its check and was deleted. Nothing was installed.";
    case "waiting_for_uac":
      return `Windows will ask for permission to install ${v}.`;
    case "declined":
      return "Permission declined. Nothing was changed.";
    case "installing":
      return `Installing ${v}.`;
    case "check_failed":
      return p.reason === "offline"
        ? "Could not check for updates: no connection."
        : p.reason === "rate_limited"
          ? "Could not check for updates: GitHub rate limit."
          : "Could not check for updates.";
    case "install_failed":
      return installFailedSentence(p.message, u.current_version, u.available?.version);
    case "up_to_date":
    case "idle":
    case "available":
    default:
      if (u.available) return `${u.available.version} available.`;
      if (u.updated_to) return `Updated to ${u.updated_to.version}.`;
      return "Up to date.";
  }
}

/** The longest line the state can say; the state line reserves room for it, so nothing under it moves. */
export const LONGEST_STATE = "The download failed its check and was deleted. Nothing was installed.";

/** The tone of the dot beside the state line. */
export function stateTone(u: UpdateState): DotTone {
  switch (u.phase.kind) {
    case "check_failed":
    case "verify_failed":
    case "install_failed":
      return "down";
    case "declined":
      return "warn";
    case "checking":
    case "downloading":
    case "verifying":
    case "waiting_for_uac":
    case "installing":
      return "work";
    default:
      return u.available ? "work" : "ok";
  }
}

/** True when the available version is the one the person skipped. */
export function isSkipped(u: UpdateState): boolean {
  return u.available !== null && u.skipped_version === u.available.version;
}

/** The short status of the Settings panel: a tone and the words. */
export function settingsStatus(u: UpdateState): { tone: DotTone; text: string } {
  const p = u.phase;
  switch (p.kind) {
    case "checking":
      return { tone: "work", text: "Checking for updates…" };
    case "downloading":
      return { tone: "work", text: `Downloading ${u.available?.version ?? ""}` };
    case "verifying":
      return { tone: "work", text: "Checking the download" };
    case "waiting_for_uac":
      return { tone: "work", text: "Waiting for Windows" };
    case "installing":
      return { tone: "work", text: `Installing ${u.available?.version ?? ""}` };
    case "declined":
      return { tone: "warn", text: "Permission declined" };
    case "verify_failed":
      return { tone: "down", text: "The download failed its check" };
    case "install_failed":
      return { tone: "down", text: "The install failed" };
    case "check_failed":
      return {
        tone: "down",
        text:
          p.reason === "offline"
            ? "Check failed: offline"
            : p.reason === "rate_limited"
              ? "Check failed: GitHub rate limit"
              : "Check failed",
      };
    default:
      if (u.available && isSkipped(u)) return { tone: "warn", text: `${u.available.version} skipped` };
      if (u.available) return { tone: "work", text: `${u.available.version} available` };
      return { tone: "ok", text: "Up to date" };
  }
}
