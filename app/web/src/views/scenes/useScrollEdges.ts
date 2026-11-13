"use client";

// Whether a sideways scroller has more to show on either side, so the page can fade that edge: a
// row of keys that scrolls with no bar (the bars are hidden everywhere) says so with a cue.

import { useCallback, useEffect, useRef, useState } from "react";

export interface ScrollEdges {
  /** Content is hidden past the left edge. */
  start: boolean;
  /** Content is hidden past the right edge. */
  end: boolean;
}

/** A pixel or two of rounding must not light a cue for a row that fits. */
const SLACK = 2;

/** The ref for the scroller, and which of its edges have more behind them. */
export function useScrollEdges<T extends HTMLElement>(): { ref: React.RefObject<T | null>; edges: ScrollEdges } {
  const ref = useRef<T>(null);
  const [edges, setEdges] = useState<ScrollEdges>({ start: false, end: false });

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const start = el.scrollLeft > SLACK;
    const end = el.scrollLeft + el.clientWidth < el.scrollWidth - SLACK;
    setEdges((cur) => (cur.start === start && cur.end === end ? cur : { start, end }));
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    measure();
    el.addEventListener("scroll", measure, { passive: true });
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(measure) : null;
    observer?.observe(el);
    // The keys come and go (a scene is added): the scroller's content is watched too.
    const content = typeof MutationObserver === "function" ? new MutationObserver(measure) : null;
    content?.observe(el, { childList: true, subtree: true });
    return () => {
      el.removeEventListener("scroll", measure);
      observer?.disconnect();
      content?.disconnect();
    };
  }, [measure]);

  return { ref, edges };
}
