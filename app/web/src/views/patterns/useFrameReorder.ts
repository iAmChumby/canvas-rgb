"use client";

// The frame strip's drag (#468), the scene strip's drag (`views/scenes/useStripReorder.ts`, #371) for
// frames, with the same rules: a press that travels past `DRAG_THRESHOLD` lifts the key, a copy
// follows the pointer and a marker stands in the gap the key would land in, the keys themselves
// never move, the key holds the pointer while the drag lasts, Escape and a lost window end it
// without a write, a finger does not drag (the strip scrolls sideways on a phone; the arrow keys
// and the Move keys are the path there), and the click that ends a drag does not select the frame.
//
// A frame is told apart by its place (`data-frame="<index>"`), not by an id: frames have none, and
// the one write is `onMove(from, to)`.

import { useCallback, useEffect, useRef, useState } from "react";
import { DRAG_THRESHOLD, cancelPress, refuseDragStart } from "@/lib/pressDrag";
import { scrollStripNear } from "@/views/scenes/useStripReorder";

/** The lifted key and where its copy is. */
export interface FrameDrag {
  index: number;
  /** The place of the pointer, in the window. */
  x: number;
  y: number;
  /** How far into the key the pointer grabbed it. */
  grabX: number;
  grabY: number;
  width: number;
  height: number;
}

interface Press {
  index: number;
  element: HTMLElement;
  pointerId: number;
  x: number;
  y: number;
  grabX: number;
  grabY: number;
  width: number;
  height: number;
}

export interface FrameReorder {
  drag: FrameDrag | null;
  /** Where the marker stands, in the scroller's content box, or null when no key is lifted. */
  marker: number | null;
  begin: (index: number, event: React.PointerEvent<HTMLElement>) => void;
  refuse: (event: React.DragEvent<HTMLElement>) => void;
  /** True once, after a drag: the click that ends it is not a choice of a frame. */
  swallowClick: () => boolean;
}

/**
 * `strip` is the scroller the keys are in; `content` is its positioned child the keys and the
 * marker share, so the marker is placed by `offsetLeft` and stays right however the strip scrolls.
 */
export function useFrameReorder(
  strip: React.RefObject<HTMLElement | null>,
  content: React.RefObject<HTMLElement | null>,
  onMove: (from: number, to: number) => void,
): FrameReorder {
  const [drag, setDrag] = useState<FrameDrag | null>(null);
  const [marker, setMarker] = useState<number | null>(null);
  const press = useRef<Press | null>(null);
  /** The place the lifted key would take among the others (0 = first). */
  const target = useRef<number | null>(null);
  const lifted = useRef(false);
  const ended = useRef(false);

  const gapAt = useCallback(
    (x: number): { to: number; left: number } | null => {
      const box = content.current;
      const from = press.current?.index;
      if (!box || from === undefined) return null;
      const keys = [...box.querySelectorAll<HTMLElement>("[data-frame]")].filter((k) => Number(k.dataset.frame) !== from);
      for (let i = 0; i < keys.length; i++) {
        const key = keys[i] as HTMLElement;
        const r = key.getBoundingClientRect();
        if (x < r.left + r.width / 2) return { to: i, left: key.offsetLeft - 5 };
      }
      const last = keys[keys.length - 1];
      return { to: keys.length, left: last ? last.offsetLeft + last.offsetWidth + 5 : 0 };
    },
    [content],
  );

  const begin = useCallback((index: number, event: React.PointerEvent<HTMLElement>) => {
    // A finger does not drag: the strip scrolls sideways.
    if (event.pointerType !== "mouse" && event.pointerType !== "pen") return;
    cancelPress(event, { focus: true });
    const rect = event.currentTarget.getBoundingClientRect();
    press.current = {
      index,
      element: event.currentTarget,
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      grabX: event.clientX - rect.left,
      grabY: event.clientY - rect.top,
      width: rect.width,
      height: rect.height,
    };
    try {
      event.currentTarget.setPointerCapture?.(event.pointerId);
    } catch {
      // A window without pointer capture: the drag still ends on the pointerup.
    }
    lifted.current = false;
    ended.current = false;
  }, []);

  const swallowClick = useCallback(() => {
    if (!ended.current) return false;
    ended.current = false;
    return true;
  }, []);

  useEffect(() => {
    const release = (start: Press) => {
      try {
        start.element.releasePointerCapture?.(start.pointerId);
      } catch {
        // The pointer is not held any more.
      }
    };
    const finish = (dropped: boolean) => {
      const start = press.current;
      press.current = null;
      if (!start) return;
      release(start);
      const to = target.current;
      const was = lifted.current;
      lifted.current = false;
      target.current = null;
      setDrag(null);
      setMarker(null);
      if (!was) return;
      ended.current = true;
      if (dropped && to !== null && to !== start.index) onMove(start.index, to);
    };
    const move = (event: PointerEvent) => {
      const start = press.current;
      if (!start) return;
      if (event.buttons === 0) {
        finish(false);
        return;
      }
      if (!lifted.current) {
        if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < DRAG_THRESHOLD) return;
        lifted.current = true;
      }
      const where = gapAt(event.clientX);
      if (where) {
        target.current = where.to;
        setMarker(where.left);
      }
      setDrag({
        index: start.index,
        x: event.clientX,
        y: event.clientY,
        grabX: start.grabX,
        grabY: start.grabY,
        width: start.width,
        height: start.height,
      });
      if (strip.current) scrollStripNear(strip.current, event.clientX);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" && press.current) finish(false);
    };
    const up = () => finish(true);
    const cancel = () => finish(false);
    const clearEnded = () => {
      ended.current = false;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", key);
    window.addEventListener("blur", cancel);
    window.addEventListener("keydown", clearEnded);
    window.addEventListener("pointerdown", clearEnded);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key);
      window.removeEventListener("blur", cancel);
      window.removeEventListener("keydown", clearEnded);
      window.removeEventListener("pointerdown", clearEnded);
    };
  }, [gapAt, onMove, strip]);

  return { drag, marker, begin, refuse: refuseDragStart, swallowClick };
}
