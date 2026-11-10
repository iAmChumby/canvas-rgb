"use client";

// Whether a scrolling pane has more below its fold (#257 phase 4, fix pass 1): the inspector's fold
// cue shows only while there is. It writes `data-more` on the pane (true or false) from its scroll
// position and its size, with no render, and the stylesheet draws or hides the cue from it.

import { useEffect, type RefObject } from "react";

/** A pane within this many pixels of its end counts as at the end. */
const SLACK = 2;

export function useMoreBelow(ref: RefObject<HTMLElement | null>, key: string) {
  useEffect(() => {
    const pane = ref.current;
    if (!pane) return;
    const update = () => {
      pane.dataset.more = pane.scrollHeight - pane.clientHeight - pane.scrollTop > SLACK ? "true" : "false";
    };
    update();
    pane.addEventListener("scroll", update, { passive: true });
    // The pane's own size, and its sections' (folding one changes how far the pane scrolls).
    const watcher = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    watcher?.observe(pane);
    for (const child of Array.from(pane.children)) watcher?.observe(child);
    // A section or a notice added after the pane mounted grows it with no scroll and no resize of
    // a watched child: watch the tree too, and size-watch each new direct child (#257 review).
    const added = typeof MutationObserver === "undefined"
      ? null
      : new MutationObserver((records) => {
          for (const record of records) {
            for (const node of Array.from(record.addedNodes)) {
              if (node instanceof Element && node.parentElement === pane) watcher?.observe(node);
            }
          }
          update();
        });
    added?.observe(pane, { childList: true, subtree: true });
    return () => {
      pane.removeEventListener("scroll", update);
      watcher?.disconnect();
      added?.disconnect();
    };
  }, [ref, key]);
}
