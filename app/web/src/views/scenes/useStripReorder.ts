"use client";

// The scene strip's drag (#371).
//
// A press that travels past a few pixels lifts the key: a copy follows the pointer and a marker
// stands in the gap the key would land in. The keys themselves never move or resize, so the
// strip shifts nothing around it. The drag ends with one write, and the click that ends it does
// not set the scene.
//
// A finger does not drag: the strip scrolls sideways on a phone, and the scene's menu is the path
// there.

import { useCallback, useEffect, useRef, useState } from "react";
import { cancelPress, refuseDragStart } from "@/lib/pressDrag";
import type { Scene } from "@/proto/Scene";

/** How far the pointer must travel before the key is lifted: a press is not a drag. */
export const DRAG_THRESHOLD = 6;
/** How close to the strip's edge the pointer must be for the strip to scroll. */
const EDGE = 48;
/** How far the strip scrolls near its edge, in pixels. */
const SCROLL_STEP = 12;
/** Half the gap between two keys: the marker stands in the middle of it. */
const GAP = 4;

/** The press that may become a drag. */
interface Press {
  /** The scene that is pressed. */
  id: string;
  /** The name on the key. */
  label: string;
  /** Where the press began. */
  x: number;
  y: number;
  /** How far into the key the pointer grabbed it. */
  grabX: number;
  grabY: number;
  /** The width of the key. */
  width: number;
}

/** The lifted key, and where its copy is. */
export interface StripDrag {
  /** The scene that is lifted. */
  id: string;
  /** The name on the key. */
  label: string;
  /** The place of the pointer, in the window. */
  x: number;
  y: number;
  /** How far into the key the pointer grabbed it. */
  grabX: number;
  grabY: number;
  /** The width of the key. */
  width: number;
}

export interface StripReorder {
  /** The copy that follows the pointer, or null when no key is lifted. */
  drag: StripDrag | null;
  /** Where the marker stands, in the strip's own coordinates, or null when there is none. */
  marker: number | null;
  /** A press on a key: it may become a drag. */
  begin: (scene: Scene, event: React.PointerEvent<HTMLButtonElement>) => void;
  /** The browser tried to drag the key away: a key is a control, not a picture. */
  refuse: (event: React.DragEvent<HTMLButtonElement>) => void;
  /**
   * True when the click that follows a drag must be ignored: the pointer just moved a key, and
   * the click is the end of that drag, not a choice of a scene.
   */
  swallowClick: () => boolean;
}

/**
 * The drag of one scene key.
 *
 * `strip` is the scroller the keys are in; the marker is placed in the scroller's box, with the
 * scroll offset added, so it stands in the gap the key would land in even when the strip is
 * scrolled sideways.
 */
export function useStripReorder(
  scenes: Scene[],
  strip: React.RefObject<HTMLElement | null>,
  onReorder: (order: string[]) => void,
): StripReorder {
  const [drag, setDrag] = useState<StripDrag | null>(null);
  const [marker, setMarker] = useState<number | null>(null);
  /** The press that may become a drag. */
  const press = useRef<Press | null>(null);
  /** The gap the lifted key would land in. */
  const gap = useRef<{ before: string | null; left: number } | null>(null);
  /** The press became a drag. */
  const lifted = useRef(false);
  /** The drag just ended: the click that follows it is not a choice of a scene. */
  const ended = useRef(false);

  /** The keys of the strip, without the one that is lifted. */
  const rest = useCallback((): HTMLElement[] => {
    const el = strip.current;
    if (!el) return [];
    const id = press.current?.id;
    return [...el.querySelectorAll<HTMLElement>("[data-scene]")].filter((k) => k.dataset.scene !== id);
  }, [strip]);

  /**
   * The gap the key would land in: the first key that is right of the pointer. The marker's place
   * is in the scroller's box, with the strip's scroll offset added.
   */
  const gapAt = useCallback(
    (x: number) => {
      const el = strip.current;
      const box = el?.parentElement;
      if (!el || !box) return null;
      const keys = rest();
      const boxRect = box.getBoundingClientRect();
      for (const key of keys) {
        const r = key.getBoundingClientRect();
        if (x < r.left + r.width / 2) {
          return { before: key.dataset.scene ?? null, left: r.left - boxRect.left + el.scrollLeft - GAP };
        }
      }
      const last = keys[keys.length - 1];
      const left = last
        ? last.getBoundingClientRect().right - boxRect.left + el.scrollLeft + GAP
        : 0;
      return { before: null, left };
    },
    [rest, strip],
  );

  /** The order the keys would have, with the lifted one in the gap. */
  const orderWith = useCallback(
    (id: string, before: string | null) => {
      const ids = scenes.map((s) => s.id);
      const others = ids.filter((x) => x !== id);
      const at = before === null ? others.length : others.indexOf(before);
      const index = at < 0 ? others.length : at;
      return [...others.slice(0, index), id, ...others.slice(index)];
    },
    [scenes],
  );

  /** The strip scrolls toward the pointer when it is near an edge. */
  const scrollNear = useCallback(
    (x: number) => {
      const el = strip.current;
      if (!el) return;
      const box = el.getBoundingClientRect();
      if (x < box.left + EDGE) el.scrollLeft -= SCROLL_STEP;
      else if (x > box.right - EDGE) el.scrollLeft += SCROLL_STEP;
    },
    [strip],
  );

  const begin = useCallback((scene: Scene, event: React.PointerEvent<HTMLButtonElement>) => {
    // A finger does not drag: the strip scrolls sideways, and the menu is the phone's path.
    if (event.pointerType !== "mouse" && event.pointerType !== "pen") return;
    // The press never starts a native drag or a text selection, and the key keeps the focus so
    // the keyboard still works after a mouse press.
    cancelPress(event, { focus: true });
    const rect = event.currentTarget.getBoundingClientRect();
    press.current = {
      id: scene.id,
      label: scene.name,
      x: event.clientX,
      y: event.clientY,
      grabX: event.clientX - rect.left,
      grabY: event.clientY - rect.top,
      width: rect.width,
    };
    lifted.current = false;
    ended.current = false;
  }, []);

  const refuse = useCallback((event: React.DragEvent<HTMLButtonElement>) => {
    refuseDragStart(event);
  }, []);

  const swallowClick = useCallback(() => {
    if (!ended.current) return false;
    ended.current = false;
    return true;
  }, []);

  useEffect(() => {
    const move = (event: PointerEvent) => {
      const start = press.current;
      if (!start) return;
      if (!lifted.current) {
        // The pointer has travelled past the threshold: the key is lifted. The distance counts,
        // not the axes: a drag along the strip is a drag.
        const dx = event.clientX - start.x;
        const dy = event.clientY - start.y;
        if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
        // The key is lifted.
        lifted.current = true;
      }
      const where = gapAt(event.clientX);
      if (where) {
        gap.current = where;
        setMarker(where.left);
      }
      setDrag({
        id: start.id,
        label: start.label,
        x: event.clientX,
        y: event.clientY,
        grabX: start.grabX,
        grabY: start.grabY,
        width: start.width,
      });
      scrollNear(event.clientX);
    };

    const stop = (dropped: boolean) => {
      const start = press.current;
      press.current = null;
      if (!start) return;
      if (!lifted.current) {
        // A press, not a drag: the key's own click sets the scene.
        setDrag(null);
        setMarker(null);
        return;
      }
      const where = gap.current;
      const order = orderWith(start.id, where?.before ?? null);
      const was = scenes.map((s) => s.id);
      lifted.current = false;
      ended.current = true;
      gap.current = null;
      setDrag(null);
      setMarker(null);
      if (dropped && order.join("|") !== was.join("|")) onReorder(order);
    };

    const key = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || !press.current) return;
      // The drag is called off: nothing is written.
      press.current = null;
      lifted.current = false;
      gap.current = null;
      setDrag(null);
      setMarker(null);
    };

    const up = () => stop(true);
    const cancel = () => stop(false);

    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key);
    };
  }, [gapAt, onReorder, orderWith, scenes]);

  return { drag, marker, begin, refuse, swallowClick };
}
