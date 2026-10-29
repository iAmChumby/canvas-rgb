"use client";

import { useCallback, useState } from "react";
import type { DeviceInfo } from "@/proto/DeviceInfo";

// New lights: once the onboarding is finished, a device that turns up afterwards never takes
// over the window. It is a quiet card on the Layout page instead ("1 new light").
//
// "New" means not seen when the owner finished (or last dismissed): the ids of the lights the
// onboarding knew about are kept on this machine. An install that finished before this existed
// has no record, so the first time it looks it learns what is there now and says nothing.

const KEY = "canvas-rgb.known-lights";

function read(): Set<string> | null {
  try {
    const raw = window.localStorage.getItem(KEY);
    return raw ? new Set(JSON.parse(raw) as string[]) : null;
  } catch {
    return null;
  }
}

function write(ids: Iterable<string>): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify([...ids]));
  } catch {
    // Not remembered: the card may show again at the next start.
  }
}

/** The lights that can show: connected, with something to light. */
function lights(devices: DeviceInfo[] | null): DeviceInfo[] {
  return (devices ?? []).filter((d) => d.connected && d.zones.some((z) => z.led_count > 0));
}

/** The owner finished the onboarding: every light there now is known. */
export function rememberLights(devices: DeviceInfo[] | null): void {
  write(lights(devices).map((d) => d.id));
}

/** The lights found since the onboarding was finished, and the way to dismiss one. */
export function useNewLights(devices: DeviceInfo[] | null, complete: boolean) {
  const [, bump] = useState(0);
  const dismiss = useCallback((id: string) => {
    const known = read() ?? new Set<string>();
    known.add(id);
    write(known);
    bump((n) => n + 1);
  }, []);
  if (!complete || !devices) return { fresh: [] as DeviceInfo[], dismiss };
  const known = read();
  if (!known) {
    rememberLights(devices);
    return { fresh: [] as DeviceInfo[], dismiss };
  }
  return { fresh: lights(devices).filter((d) => !known.has(d.id)), dismiss };
}
