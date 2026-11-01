"use client";

import { useCallback, useLayoutEffect, useState, useSyncExternalStore } from "react";

/**
 * True while the media query matches. The editor uses it where the layout has to change
 * shape, not just size: the effect detail becomes its own view on a phone, and the fan
 * headers become cards instead of a table.
 */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      const mq = window.matchMedia(query);
      mq.addEventListener("change", onChange);
      return () => mq.removeEventListener("change", onChange);
    },
    [query],
  );
  const snapshot = useCallback(() => window.matchMedia(query).matches, [query]);
  return useSyncExternalStore(subscribe, snapshot, () => false);
}

/**
 * True while the page's own box (`.content`, the container the stylesheet's `@container page`
 * rules answer to) is at most `max` px wide, measured the way the container query measures it:
 * the box without its padding. The fan page uses it, so the sheet that its CSS switches on at
 * 640 px and the page's own phone-or-desktop choice are one condition. A window query cannot
 * give that: the container is narrower than the window by the rail and the padding, so the
 * two disagreed between 721 and 752 px. Reads false until the box is measured, and when
 * there is no box.
 */
export function usePageNarrow(max = 640): boolean {
  const [narrow, setNarrow] = useState(false);
  useLayoutEffect(() => {
    const box = document.querySelector<HTMLElement>(".content");
    if (!box) return;
    const measure = () => {
      const cs = getComputedStyle(box);
      const width =
        box.clientWidth - (parseFloat(cs.paddingLeft) || 0) - (parseFloat(cs.paddingRight) || 0);
      setNarrow(width > 0 && width <= max);
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(box);
    return () => watch.disconnect();
  }, [max]);
  return narrow;
}
