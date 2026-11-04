"use client";

import { useLayoutEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent, type PointerEvent } from "react";
import { cancelPress, refuseDragStart } from "@/lib/pressDrag";
import { dragThresholdPx } from "@/lib/hitTest";
import {
  RAIL_BIG_STEP_PX,
  RAIL_DEFAULT_PX,
  RAIL_MAX_PX,
  RAIL_MIN_PX,
  RAIL_STEP_PX,
  applyRailWidth,
  clampRailWidth,
  railMaxPx,
  readAppliedRailWidth,
  readStoredRailWidth,
  storeRailWidth,
} from "@/lib/railWidth";

/** How long two presses may be apart and still be a double-click, in milliseconds. */
const DOUBLE_CLICK_MS = 500;
/** How far apart two presses may be and still be a double-click, in css pixels. */
const DOUBLE_CLICK_SLOP_PX = 8;

/*
 * The width, outside React.
 *
 * The document is the source: the early script (app/layout.tsx) put the width on it before the
 * first paint, and this handle writes to it when the reader changes the width. React reads the
 * width back through useSyncExternalStore, the same way the editor reads the window (see
 * components/MediaQuery.ts), so a drag, a key and a resize all reach the page the same way, and
 * the first render on the client already has the width the early script applied.
 */
const listeners = new Set<() => void>();
/** Whether a drag owns the width right now: a resize must not take it away mid-drag. */
let dragInProgress = false;

function notify(): void {
  for (const listener of listeners) listener();
}

/** The width the document shows, clamped to this window. */
function getWidth(): number {
  const applied = readAppliedRailWidth();
  if (applied !== null) return clampRailWidth(applied, window.innerWidth);
  // The early script could not run (storage blocked): the stored width is the truth.
  return clampRailWidth(readStoredRailWidth() ?? RAIL_DEFAULT_PX, window.innerWidth);
}

/** The width the server rendered: the default, before anything is known. */
function getServerWidth(): number {
  return RAIL_DEFAULT_PX;
}

function subscribeWidth(onChange: () => void): () => void {
  listeners.add(onChange);
  return () => {
    listeners.delete(onChange);
  };
}

function getWindowWidth(): number {
  return window.innerWidth;
}

/** The window's width before it is measured: the bound is the rail's own cap until then. */
function getServerWindowWidth(): number {
  return 0;
}

function subscribeWindow(onChange: () => void): () => void {
  const onResize = () => {
    // The window changed: the bound moves with it, and the rail gives way before the page does.
    // The width this viewer chose is kept, so a wider window gets it back.
    if (!dragInProgress) {
      const stored = readStoredRailWidth();
      const next = clampRailWidth(stored ?? readAppliedRailWidth() ?? RAIL_DEFAULT_PX, window.innerWidth);
      if (readAppliedRailWidth() !== next) {
        applyRailWidth(next);
        notify();
      }
    }
    onChange();
  };
  window.addEventListener("resize", onResize);
  return () => window.removeEventListener("resize", onResize);
}

/** A new width: it is shown at once, and kept when the reader is finished with it. */
function publishWidth(next: number, keep: boolean): number {
  const w = clampRailWidth(next, window.innerWidth);
  applyRailWidth(w);
  if (keep) storeRailWidth(w);
  notify();
  return w;
}

/**
 * The rail's resize handle: a thin grab strip on the rail's right edge.
 *
 * The rail is the shell's first grid column, so its width is a page-level fact. This handle is
 * the one place the reader can change it: a drag with the mouse, the arrow keys for a step,
 * Home and End for the bounds, and a double-click (or Enter) to put it back to its default. The
 * width is kept in this viewer's localStorage and applied before the first paint, so the rail is
 * already the right width when the page appears.
 *
 * The bounds are the editor's (`lib/railWidth.ts`): never narrower than the default, never wider
 * than 320 px or a quarter of the window, and never so wide that the page column drops below the
 * width #67's layouts were fitted to. When the window shrinks the rail gives way first.
 *
 * Where the bounds meet (at 1280 px the widest the rail may be is its default) there is nothing
 * to move, so no handle is drawn: a separator that cannot move would still take focus. Below
 * 1280 px the rail is the icon rail and the phone has a dock of its own; the stylesheet hides it
 * there too.
 */
export function RailResizeHandle() {
  const width = useSyncExternalStore(subscribeWidth, getWidth, getServerWidth);
  const windowWidth = useSyncExternalStore(subscribeWindow, getWindowWidth, getServerWindowWidth);
  const bound = windowWidth > 0 ? railMaxPx(windowWidth) : RAIL_MAX_PX;
  /** Whether the pointer is dragging the rail now: the handle's line shows it. */
  const [dragging, setDragging] = useState(false);
  /** The one pointer that owns the drag: a second finger can neither move nor end it. */
  const owner = useRef<number | null>(null);
  /** Where the drag began: the pointer's place and the width the rail had then. */
  const start = useRef<{ x: number; width: number } | null>(null);
  /** Whether the press has travelled far enough to be a drag. */
  const moved = useRef(false);
  /** The last press that was a click, so the next one near it is a double-click. */
  const lastClick = useRef<{ t: number; x: number } | null>(null);

  // The early script (app/layout.tsx) applied the width before the first paint. If it could not
  // (storage blocked), the width is applied here, so the rail and the page agree.
  useLayoutEffect(() => {
    const stored = readStoredRailWidth();
    if (stored === null) return;
    const next = clampRailWidth(stored, window.innerWidth);
    if (readAppliedRailWidth() === next) return;
    applyRailWidth(next);
    notify();
  }, []);

  const endDrag = (e: PointerEvent<HTMLDivElement>) => {
    owner.current = null;
    start.current = null;
    dragInProgress = false;
    setDragging(false);
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    // A press never starts a native drag or a text selection (#105); the keyboard keeps working
    // after a mouse press, without claiming the keyboard's focus ring.
    cancelPress(e, { focus: true });
    // Two presses near each other are a double-click: the rail goes back to its default. The
    // browser's own dblclick never arrives, because the canceled press takes the compatibility
    // mouse events with it.
    const last = lastClick.current;
    if (last && Date.now() - last.t <= DOUBLE_CLICK_MS && Math.abs(e.clientX - last.x) <= DOUBLE_CLICK_SLOP_PX) {
      lastClick.current = null;
      publishWidth(RAIL_DEFAULT_PX, true);
      return;
    }
    if (owner.current !== null) return;
    owner.current = e.pointerId;
    moved.current = false;
    start.current = { x: e.clientX, width: getWidth() };
    e.currentTarget.setPointerCapture(e.pointerId);
    dragInProgress = true;
    setDragging(true);
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (e.pointerId !== owner.current) return;
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    const s = start.current;
    if (!s) return;
    if (!moved.current) {
      // A press that has not travelled a few pixels is a click, not a drag: the rail must not
      // move under it.
      const kind = e.pointerType === "touch" ? "touch" : "mouse";
      if (Math.abs(e.clientX - s.x) < dragThresholdPx(kind)) return;
      moved.current = true;
    }
    // The rail's edge follows the pointer: the drag moves the width by how far the pointer
    // travelled, so the rail does not jump to the pointer when the drag begins.
    publishWidth(s.width + (e.clientX - s.x), false);
  };

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    if (e.pointerId !== owner.current) return;
    const wasDrag = moved.current;
    endDrag(e);
    if (!wasDrag) {
      // A click, not a drag: the next press near it is the second half of a double-click.
      lastClick.current = { t: Date.now(), x: e.clientX };
      return;
    }
    // The drag is over: the width is kept, so it is still there next time.
    storeRailWidth(getWidth());
  };

  const onPointerCancel = (e: PointerEvent<HTMLDivElement>) => {
    if (e.pointerId !== owner.current) return;
    const wasDrag = moved.current;
    const s = start.current;
    endDrag(e);
    // The browser took the pointer: the rail goes back to the width it had before the drag.
    if (wasDrag && s) publishWidth(s.width, false);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? RAIL_BIG_STEP_PX : RAIL_STEP_PX;
    const now = getWidth();
    let next: number | null = null;
    if (e.key === "ArrowRight") next = now + step;
    else if (e.key === "ArrowLeft") next = now - step;
    else if (e.key === "Home") next = RAIL_MIN_PX;
    else if (e.key === "End") next = railMaxPx(window.innerWidth);
    else if (e.key === "Enter") next = RAIL_DEFAULT_PX;
    if (next === null) return;
    e.preventDefault();
    publishWidth(next, true);
  };

  // The bounds meet: nothing to resize, so nothing to focus. Before the window is measured
  // (`windowWidth` is 0) the handle is drawn, so server and first client render agree.
  if (windowWidth > 0 && bound <= RAIL_MIN_PX) return null;

  return (
    <div
      className="rail-resize"
      role="separator"
      tabIndex={0}
      aria-orientation="vertical"
      aria-label="Resize the left rail"
      aria-valuemin={RAIL_MIN_PX}
      aria-valuemax={bound}
      aria-valuenow={width}
      data-dragging={dragging ? "true" : "false"}
      draggable={false}
      onDragStart={refuseDragStart}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
      onLostPointerCapture={onPointerCancel}
      onKeyDown={onKeyDown}
    />
  );
}
