"use client";

import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";

/** The space between the control and the panel, in css pixels. */
const GAP = 6;
/** The space kept to the window's edge, in css pixels. */
const EDGE = 8;
/** The least height a panel may have, in css pixels: below this it is a sliver. */
const MIN_H = 96;

/**
 * A panel that hangs under a control.
 *
 * It takes no room in the page: it is drawn over the content, so opening it never moves
 * anything. It is drawn at the end of the document, so no ancestor's overflow can cut it.
 * It closes on Escape and on a click outside it.
 *
 * The panel stays in the window. It hangs under the control when there is room for it there,
 * and above the control when there is not - so a list of effects near the bottom of the screen
 * opens upwards instead of running off the edge. It is also pulled back inside the window
 * sideways, and it scrolls when it is taller than the window.
 */
export function Popover({
  anchor,
  open,
  onClose,
  children,
  className,
  label,
}: {
  /** The control it hangs under. */
  anchor: React.RefObject<HTMLElement | null>;
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  className?: string;
  label?: string;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<CSSProperties | undefined>(undefined);

  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const el = anchor.current;
      const panel = panelRef.current;
      if (!el || !panel) return;
      const r = el.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const width = panel.offsetWidth || 280;
      // The content's own height, not the height the window allows: the panel may be shorter
      // than its content because it scrolls, and the flip is decided by the content.
      const height = panel.scrollHeight || panel.offsetHeight || 0;

      // Sideways: the panel's right edge sits on the anchor's right edge, so it opens towards
      // the middle of the window, and it is then pulled back inside the window.
      const left =
        width >= vw - EDGE * 2
          ? EDGE
          : Math.max(EDGE, Math.min(r.right - width, vw - width - EDGE));

      // Up or down: under the control when the whole panel fits there, above it when it does
      // not and there is room above, and inside the window either way.
      const below = r.bottom + GAP;
      const above = r.top - GAP - height;
      const top =
        height > 0 && below + height > vh - EDGE && above >= EDGE
          ? above
          : Math.max(EDGE, Math.min(below, vh - EDGE - height));

      setStyle({
        left: `${Math.round(left)}px`,
        top: `${Math.round(top)}px`,
        maxHeight: `${Math.max(MIN_H, vh - EDGE * 2)}px`,
      });
    };
    place();
    // The panel's own size changes when its content settles (a picture loads), and then it has
    // to be placed again.
    const ro = new ResizeObserver(place);
    if (panelRef.current) ro.observe(panelRef.current);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open, anchor]);

  // Escape closes it, and so does a click anywhere else.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node | null;
      if (!t) return;
      if (panelRef.current?.contains(t)) return;
      if (anchor.current?.contains(t)) return;
      onClose();
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("pointerdown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("pointerdown", onDown);
    };
  }, [open, onClose, anchor]);

  if (!open || typeof document === "undefined") return null;
  return createPortal(
    <div
      ref={panelRef}
      className={className ? `popover ${className}` : "popover"}
      role="dialog"
      aria-label={label}
      style={style}
    >
      {children}
    </div>,
    document.body,
  );
}
