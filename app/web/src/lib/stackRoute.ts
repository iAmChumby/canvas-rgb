"use client";

// Build mode's URL: the Effects route with `?stack=<name>` (D95.5.1). The builder is opened by a
// small in-page navigation rather than a Next route change, so a page that is dirty keeps the
// unsaved prompt in charge of the browser's Back; a reload reads the same query and reopens it.

import { useEffect, useRef, useState } from "react";

/** The `history.state` key of the builder's guard entry; `STACK_ENTRY_SPENT` marks an entry the builder left. */
export const STACK_GUARD = "canvasRgbStackGuard";
export const STACK_ENTRY_SPENT = "spent";

export type StackRoute = [name: string | null, go: (name: string | null, options?: { replace?: boolean }) => void];

function read(): string | null {
  if (typeof window === "undefined") return null;
  return new URLSearchParams(window.location.search).get("stack");
}

/** True when the entry the document is on would open the builder but is one the builder's guard made or the builder left. */
function onSpentEntry(): boolean {
  if (typeof window === "undefined" || read() === null) return false;
  const state = window.history.state as Record<string, unknown> | null;
  return state !== null && typeof state === "object" && STACK_GUARD in state;
}

export function useStackParam(): StackRoute {
  // An entry the builder's guard made, or one the builder left, opens no builder: a browser cannot
  // delete the guard's copy, so a Forward onto it, or a reload of it, lands here with the builder
  // shut and the entry becomes the effects page it stands for. The page is only ever mounted on
  // such an entry by a traversal or a reload, when `history.state` is already the entry's own.
  const [spent] = useState(onSpentEntry);
  const [name, setName] = useState<string | null>(() => (spent ? null : read()));
  const current = useRef(name);
  current.current = name;

  useEffect(() => {
    if (spent) go(null, { replace: true });
    // Once, as the page opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onPop = () => {
      // A builder that is open keeps its own name; one that is shut stays shut.
      if (current.current === null && onSpentEntry()) {
        go(null, { replace: true });
        return;
      }
      setName(read());
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  function go(next: string | null, options?: { replace?: boolean }) {
    // The group the builder draws stays in the URL: `?stack=<name>&group=<id>` is one place.
    const group = typeof window === "undefined" ? null : new URLSearchParams(window.location.search).get("group");
    const parts: string[] = [];
    if (next !== null) parts.push(`stack=${encodeURIComponent(next)}`);
    if (group) parts.push(`group=${encodeURIComponent(group)}`);
    const query = parts.join("&");
    const url = query ? `/effects/?${query}` : "/effects/";
    const state = { canvasRgbStack: next };
    if (options?.replace) window.history.replaceState(state, "", url);
    else window.history.pushState(state, "", url);
    setName(next);
  }

  return [name, go];
}
