"use client";

import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";

/**
 * A panel that hangs under a control.
 *
 * It takes no room in the page: it is drawn over the content, so opening it never moves
 * anything. It is drawn at the end of the document, so no ancestor's overflow can cut it.
 * It closes on Escape and on a click outside it.
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

  // The panel's right edge sits on the anchor's right edge, so it opens towards the middle of
  // the window and never runs past the edge.
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const el = anchor.current;
      const panel = panelRef.current;
      if (!el || !panel) return;
      const r = el.getBoundingClientRect();
      const width = panel.offsetWidth || 280;
      const left = Math.max(8, Math.min(r.right - width, window.innerWidth - width - 8));
      setStyle({ left: `${Math.round(left)}px`, top: `${Math.round(r.bottom + 6)}px` });
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
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
