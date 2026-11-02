"use client";

// Build mode's URL: the Effects route with `?stack=<name>` (D95.5.1). The builder is opened by a
// small in-page navigation rather than a Next route change, so a page that is dirty keeps the
// unsaved prompt in charge of the browser's Back; a reload reads the same query and reopens it.

import { useEffect, useState } from "react";

export type StackRoute = [name: string | null, go: (name: string | null, options?: { replace?: boolean }) => void];

function read(): string | null {
  if (typeof window === "undefined") return null;
  return new URLSearchParams(window.location.search).get("stack");
}

export function useStackParam(): StackRoute {
  const [name, setName] = useState<string | null>(read);

  useEffect(() => {
    const onPop = () => setName(read());
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  const go = (next: string | null, options?: { replace?: boolean }) => {
    const url = next === null ? "/effects/" : `/effects/?stack=${encodeURIComponent(next)}`;
    const state = { canvasRgbStack: next };
    if (options?.replace) window.history.replaceState(state, "", url);
    else window.history.pushState(state, "", url);
    setName(next);
  };

  return [name, go];
}
