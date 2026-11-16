"use client";

// Which lights the page has told the owner about. A light that appears after the page has seen
// the others, and that no group holds, is "new": the page says so once, with one action per group.
// What the page has seen is kept in the browser's own storage, so a restart does not announce
// every light again. The first visit announces nothing: every light it sees is taken as known.

import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import type { LightView } from "@/proto/LightView";

import { SEEN_LIGHTS_KEY } from "@/lib/seenLights";

export { SEEN_LIGHTS_KEY };

// The set lives in the browser's storage and is read through an external store, so a second
// window and a test that rewrites the key both show up without an effect copying it into state.
const listeners = new Set<() => void>();
let cachedRaw: string | null | undefined;
let cachedSet: Set<string> | null = null;

/** The ids the page has seen, or null before the first visit. Stable while the stored text is. */
function getSeen(): Set<string> | null {
  let raw: string | null = null;
  try {
    raw = window.localStorage.getItem(SEEN_LIGHTS_KEY);
  } catch {
    return cachedSet;
  }
  if (raw === cachedRaw) return cachedSet;
  cachedRaw = raw;
  try {
    const parsed: unknown = raw === null ? null : JSON.parse(raw);
    cachedSet = Array.isArray(parsed) ? new Set(parsed.filter((x): x is string => typeof x === "string")) : null;
  } catch {
    cachedSet = null;
  }
  return cachedSet;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Adds `ids` to what the page has seen and tells every subscriber. */
function markSeen(ids: readonly string[]): void {
  const next = new Set(getSeen() ?? []);
  for (const id of ids) next.add(id);
  try {
    window.localStorage.setItem(SEEN_LIGHTS_KEY, JSON.stringify([...next]));
  } catch {
    // Not remembered: the notice may come again next time.
  }
  cachedRaw = undefined;
  for (const l of listeners) l();
}

/**
 * The lights to announce: the unassigned ones the page has not seen before. A light that sits in
 * a group, or that the owner dismissed, counts as seen from then on.
 */
export function useNewLights(
  all: LightView[] | null,
  unassigned: LightView[],
): { fresh: LightView[]; dismiss: (id: string) => void } {
  const seen = useSyncExternalStore(subscribe, getSeen, () => null);

  useEffect(() => {
    // An empty list is not a first look at the lights: nothing has been seen yet, and a light that
    // arrives later must not be announced for it. The first visit is the first list with a light in it.
    if (!all || all.length === 0) return;
    if (seen === null) {
      // First visit: everything on screen is known, and nothing is announced.
      markSeen(all.map((l) => l.light.id));
      return;
    }
    const open = new Set(unassigned.map((l) => l.light.id));
    // Held by a group: known too, so removing it later does not announce it again.
    const held = all.filter((l) => !open.has(l.light.id) && !seen.has(l.light.id)).map((l) => l.light.id);
    if (held.length > 0) markSeen(held);
  }, [all, unassigned, seen]);

  const dismiss = useCallback((id: string) => markSeen([id]), []);

  const fresh = useMemo(() => (seen ? unassigned.filter((l) => !seen.has(l.light.id)) : []), [seen, unassigned]);
  return { fresh, dismiss };
}
