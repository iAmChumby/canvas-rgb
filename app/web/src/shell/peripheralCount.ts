"use client";

import { useEffect, useSyncExternalStore } from "react";
import { api } from "@/lib/endpoints";

/**
 * How many peripherals the adapters see.
 *
 * The editor's shared snapshot does not hold the peripherals (the Peripherals page keeps its
 * own list, because it reads one device in detail), so the rail asks for the list itself and
 * keeps the count. It is a read-only route, and the rail refreshes it slowly: the number
 * changes when a device is plugged in, not every second.
 *
 * A daemon without the peripheral routes leaves the count unknown. The row then says nothing,
 * which is better than a number nobody can stand behind.
 */

/** How often the rail asks again. */
const REFRESH_MS = 10000;

let count: number | null = null;
let timer: ReturnType<typeof setInterval> | null = null;
/** How many components are watching. The poll runs while at least one is. */
let watchers = 0;
const listeners = new Set<() => void>();

function publish(): void {
  for (const l of listeners) l();
}

async function refresh(): Promise<void> {
  try {
    const r = await api.peripherals();
    count = r.peripherals.length;
  } catch {
    // The daemon refused or has no such route: the count stays as it was, or unknown.
  }
  publish();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function snapshot(): number | null {
  return count;
}

/** The number of peripherals, or null when the daemon has not said. */
export function usePeripheralCount(): number | null {
  useEffect(() => {
    watchers += 1;
    if (watchers === 1) {
      void refresh();
      timer = setInterval(() => void refresh(), REFRESH_MS);
    }
    return () => {
      watchers -= 1;
      if (watchers === 0 && timer) {
        clearInterval(timer);
        timer = null;
      }
    };
  }, []);
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
