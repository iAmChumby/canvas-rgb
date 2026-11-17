"use client";

import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { motionOffNow, readCssMs } from "@/lib/motion";

/** The space between the control and the panel, in css pixels. */
const GAP = 6;
/** The space kept to the window's edge, in css pixels. */
const EDGE = 8;
/** The least height a panel may have, in css pixels: below this it is a sliver. */
const MIN_H = 96;

/**
 * Which horizontal side of a panel the growth origin sits on: the panel corner
 * nearest the anchor (issue #187, rule 1). The panel's centre against the
 * anchor's centre decides it: an anchor left of the panel's centre is nearest
 * the panel's left edge; an anchor right of it is nearest the right edge. An
 * exact tie (the panel exactly under its anchor) falls back to the window's
 * middle side: a panel that covers its anchor grows toward the middle.
 */
export function panelOriginSide(
  anchorLeft: number,
  anchorWidth: number,
  left: number,
  width: number,
  viewportW: number,
): "left" | "right" {
  const anchorCentre = anchorLeft + anchorWidth / 2;
  const panelCentre = left + width / 2;
  if (anchorCentre < panelCentre) return "left";
  if (anchorCentre > panelCentre) return "right";
  return anchorCentre < viewportW / 2 ? "left" : "right";
}

/** The rectangle of the control a panel hangs from. */
export type AnchorRect = { left: number; right: number; top: number; bottom: number; width: number };

/** Where a panel goes: its left and top, the most height it may take, and whether it grew up. */
export type PanelPlacement = { left: number; top: number; maxHeight: number; up: boolean };

/**
 * Place a panel in the window.
 *
 * By default a panel that does not fit below its anchor is clamped back inside the window, which
 * can bring it up over the anchor's own row. A panel that must stay clear of that row
 * (`keepClear`) is capped instead to the room on one side of the anchor and scrolls inside that
 * room: it hangs below the anchor when there is room there, flips above when there is not, and
 * its near edge always stops a gap short of the anchor. Its height never exceeds the content, so
 * a short menu keeps its old size. Only below MIN_H on both sides can no useful panel fit; then
 * the smallest panel stays inside the window.
 */
export function placePanel(
  a: AnchorRect,
  vw: number,
  vh: number,
  contentHeight: number,
  width: number,
  keepClear: boolean,
): PanelPlacement {
  // Sideways: the panel's right edge sits on the anchor's right edge, pulled inside the window.
  const left =
    width >= vw - EDGE * 2
      ? EDGE
      : Math.max(EDGE, Math.min(a.right - width, vw - width - EDGE));

  const below = a.bottom + GAP;
  const above = a.top - GAP - contentHeight;
  const up = contentHeight > 0 && below + contentHeight > vh - EDGE && above >= EDGE;
  const top = up ? above : Math.max(EDGE, Math.min(below, vh - EDGE - contentHeight));
  const maxHeight = Math.max(MIN_H, vh - EDGE * 2);
  if (!keepClear) return { left, top, maxHeight, up };

  const roomBelow = vh - EDGE - below;
  const roomAbove = a.top - GAP - EDGE;
  if (roomBelow >= MIN_H || roomBelow >= roomAbove) {
    const h = Math.max(MIN_H, Math.min(contentHeight || roomBelow, roomBelow));
    return { left, top: below, maxHeight: h, up: false };
  }
  if (roomAbove >= MIN_H) {
    const h = Math.max(MIN_H, Math.min(contentHeight || roomAbove, roomAbove));
    return { left, top: a.top - GAP - h, maxHeight: h, up: true };
  }
  return { left, top: Math.max(EDGE, Math.min(below, vh - EDGE - MIN_H)), maxHeight: MIN_H, up: false };
}

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
  keepClearOfAnchor = false,
}: {
  /** The control it hangs under. */
  anchor: React.RefObject<HTMLElement | null>;
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  className?: string;
  label?: string;
  /** Keep the panel clear of the anchor's own row, scrolling inside the room beside it. */
  keepClearOfAnchor?: boolean;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const id = useId();
  const [style, setStyle] = useState<CSSProperties | undefined>(undefined);
  // The exit (issue #187, rule 2): when `open` drops, the panel stays for
  // --dur-exit under its exit animation, then goes. It closes back into the
  // trigger, toward the same origin it grew from. The state adjustment runs
  // while rendering (the Crossfade pattern), so the panel is in the tree
  // before the placement effect measures it. Reopening during the exit
  // cancels the exit at once: `closing` drops and the timer's cleanup clears
  // it, so the same panel node stays in the tree.
  const [rendered, setRendered] = useState(open);
  const [closing, setClosing] = useState(false);
  if (open && closing) {
    setClosing(false);
  } else if (open && !rendered) {
    setRendered(true);
    setClosing(false);
  } else if (!open && rendered && !closing) {
    setClosing(true);
  }
  // A panel that closes with focus inside it hands focus back to its anchor, before the
  // closing panel goes inert, so focus never falls to the page body.
  useLayoutEffect(() => {
    if (!closing) return;
    const panel = panelRef.current;
    if (panel && panel.contains(document.activeElement)) anchor.current?.focus({ preventScroll: true });
  }, [closing, anchor]);
  useEffect(() => {
    if (!closing) return;
    const ms = motionOffNow() ? 0 : readCssMs("--dur-exit", 90);
    const t = setTimeout(() => {
      setRendered(false);
      setClosing(false);
    }, ms);
    return () => clearTimeout(t);
  }, [closing]);

  // The control names the panel it opened (aria-controls) while the panel is open, so a tool or a
  // screen reader can tell which control a panel belongs to.
  useLayoutEffect(() => {
    const el = anchor.current;
    if (!open || !el) return;
    const had = el.getAttribute("aria-controls");
    el.setAttribute("aria-controls", id);
    return () => {
      if (had === null) el.removeAttribute("aria-controls");
      else el.setAttribute("aria-controls", had);
    };
  }, [open, anchor, id]);

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
      const { left, top, maxHeight, up } = placePanel(
        { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width },
        vw,
        vh,
        height,
        width,
        keepClearOfAnchor,
      );

      // The origin (issue #187, rule 1): the panel corner nearest the anchor.
      // The panel hangs from the anchor's right edge (pulled inside the window
      // at its left edge), so which corner is nearest is read off the placed
      // geometry — an anchor left of the panel's centre is nearest the left
      // corner, an anchor right of it nearest the right corner.
      const side = panelOriginSide(r.left, r.width, left, width, vw);

      setStyle({
        left: `${Math.round(left)}px`,
        top: `${Math.round(top)}px`,
        maxHeight: `${Math.round(maxHeight)}px`,
        transformOrigin: `${up ? "bottom" : "top"} ${side}`,
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
  }, [open, anchor, keepClearOfAnchor]);

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

  if (!rendered || typeof document === "undefined") return null;
  return createPortal(
    <div
      ref={panelRef}
      id={id}
      className={className ? `popover ${className}` : "popover"}
      role="dialog"
      aria-label={label}
      // While closing the panel cannot act: it is inert (out of the tab order
      // and unfocusable) and hidden from assistive tech; the trigger's own
      // expanded state already says it is closed.
      inert={closing || undefined}
      aria-hidden={closing || undefined}
      data-closing={closing ? "true" : undefined}
      style={style}
    >
      {children}
    </div>,
    document.body,
  );
}
