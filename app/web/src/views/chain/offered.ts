"use client";

// Which headers the takeover has already offered by itself, and which the owner has put off.
//
// The setup opened by itself every time the window came up from the tray (#380): the "Later" it
// remembered belonged to the launch, and a launch is one webview session, so a window opened from
// the tray was a new launch and forgot it.
//
// What is remembered here belongs to the header, not to the launch. The header's identity is its
// device and its zone, and the record lives in this viewer's own storage, so a header is offered
// by itself at most once, ever, and "Later", the close key and Escape are remembered across
// window opens and app restarts.

/** The browser storage key that holds the headers the app has already offered by itself. */
export const CHAIN_OFFERED_KEY = "canvas-rgb.chains-offered";

/** One header's identity: its device and its zone. */
export function chainKey(deviceId: string, zoneIndex: number): string {
  return `${deviceId}#z${zoneIndex}`;
}

// The record is read on every render of the takeover, so it is parsed once and kept: the raw
// text it came from is what says whether it changed.
let cachedRaw: string | null | undefined;
let cached: Set<string> | null = null;

function stored(): Set<string> {
  let raw: string | null = null;
  try {
    raw = window.localStorage.getItem(CHAIN_OFFERED_KEY);
  } catch {
    return cached ?? new Set();
  }
  if (raw === cachedRaw && cached) return cached;
  cachedRaw = raw;
  let set: Set<string>;
  try {
    const parsed: unknown = raw === null ? null : JSON.parse(raw);
    set = new Set(Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : []);
  } catch {
    set = new Set();
  }
  cached = set;
  return set;
}

/** The headers the app has already offered by itself, or the owner has put off. */
export function offeredChains(): ReadonlySet<string> {
  return stored();
}

/** Remembers a header: the app does not offer it by itself again. */
export function markChainOffered(key: string): void {
  const set = stored();
  if (set.has(key)) return;
  set.add(key);
  const text = JSON.stringify([...set]);
  try {
    window.localStorage.setItem(CHAIN_OFFERED_KEY, text);
    cachedRaw = text;
  } catch {
    // Not remembered: the header may be offered by itself again.
  }
}
