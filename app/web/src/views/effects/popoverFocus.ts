// A popover closed by Escape puts focus back on the key that opened it. A click outside closes it
// without moving the focus the person already has. Split out of StackBuilder.tsx in #257 phase 4.

import type { RefObject } from "react";

export function closePopover(selector: string, opener: RefObject<HTMLElement | null>, close: () => void) {
  const panel = document.querySelector(selector);
  const inside = panel !== null && document.activeElement instanceof Node && panel.contains(document.activeElement);
  close();
  if (inside) {
    setTimeout(() => opener.current?.querySelector<HTMLElement>("button")?.focus(), 0);
  }
}
