"use client";

// The focus trap one modal dialog owns while it is open (#232): Tab and Shift+Tab cycle focus
// inside it, a Tab from a control behind it is pulled back in, and Escape closes it. One
// implementation, two placements.
//
// Placement is the whole design. The palette is the topmost dialog, so its trap runs at window
// capture and stops the keys before anything else on the page can act on them; that is what
// keeps a palette open over the phone layer sheet from fighting the sheet's own trap (the sheet
// never sees a Tab or an Escape the palette owns). The sheet keeps its legacy placement, bubble
// on document, behind the `capture: false` flag, so its behaviour over the page behind it is
// unchanged.

import { useEffect, type RefObject } from "react";

/** What Tab reaches inside a dialog, in document order. */
const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function useFocusTrap({
  panelRef,
  onEscape,
  capture = true,
  enabled = true,
}: {
  panelRef: RefObject<HTMLElement | null>;
  onEscape: () => void;
  /**
   * Where the key listener sits. The default, true, is a window-capture listener: the palette's
   * trap, which has to preempt the phone sheet's document-bubble trap while it is over it.
   * False is a document-bubble listener: the phone sheet's own placement, unchanged.
   */
  capture?: boolean;
  /** False while the dialog is closed or stepped aside (a picker over the phone sheet): no trap. */
  enabled?: boolean;
}) {
  useEffect(() => {
    if (!enabled) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Tab" && event.key !== "Escape") return;
      // A key this dialog owns must not travel further: a trap behind it (the sheet's, for the
      // palette) would otherwise act on it even mid-panel, where no focus moved.
      event.stopPropagation();
      if (event.key === "Escape") {
        event.preventDefault();
        onEscape();
        return;
      }
      const panel = panelRef.current;
      if (!panel) return;
      const items = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (items.length === 0) {
        event.preventDefault();
        panel.focus();
        return;
      }
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const active = document.activeElement;
      if (!panel.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
        return;
      }
      if (event.shiftKey && (active === first || active === panel)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (active === last || active === panel)) {
        event.preventDefault();
        first.focus();
      }
      // Mid-panel, Tab is the browser's own; only the edges wrap.
    };
    if (capture) {
      window.addEventListener("keydown", onKey, true);
      return () => window.removeEventListener("keydown", onKey, true);
    }
    document.addEventListener("keydown", onKey, false);
    return () => document.removeEventListener("keydown", onKey, false);
  }, [panelRef, onEscape, capture, enabled]);
}
