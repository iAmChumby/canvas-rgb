"use client";

// The group rows' drag (#384).
//
// The same shape as the scene strip's drag (#371): a press that travels past a few pixels lifts
// the row, a copy follows the pointer and a marker stands in the gap the row would land in. The
// rows around it never move, so the list shifts nothing around it.
//
// The handle holds the pointer while the drag lasts (pointer capture), so the drag ends where the
// pointer let go, even outside the window. It ends without a write when the pointer is taken
// away: the window loses the focus, the pointer is cancelled, or the button is no longer down.
//
// A finger does not drag: the page scrolls, and the group's own menu is the path there.

import { useCallback, useEffect, useRef, useState } from "react";
import { DRAG_THRESHOLD, cancelPress, refuseDragStart } from "@/lib/pressDrag";
import { groupAccent } from "@/views/scenes/groupAccent";
import type { Group } from "@/proto/Group";

/** Half the gap between two rows: the marker stands in the middle of it. */
const GAP = 4;

/** The press that may become a drag. */
interface Press {
  /** The group that is pressed. */
  id: string;
  /** The name on the row. */
  label: string;
  /** The handle that is pressed: it holds the pointer while the drag lasts. */
  element: HTMLElement;
  /** The pointer that pressed it. */
  pointerId: number;
  /** Where the press began. */
  x: number;
  y: number;
  /** How far into the handle the pointer grabbed it. */
  grabX: number;
  grabY: number;
  /** The width of the row. */
  width: number;
  /** The height of the row. */
  height: number;
}

/** A handle that can hold the pointer. */
interface Capture {
  setPointerCapture?: (pointerId: number) => void;
  releasePointerCapture?: (pointerId: number) => void;
}

/** Lets the handle go: the pointer is free again, whichever way the drag ended. */
function releaseCapture(start: Press): void {
  const el = start.element as unknown as Capture;
  try {
    el.releasePointerCapture?.(start.pointerId);
  } catch {
    // The pointer is not held any more: there is nothing to let go of.
  }
}

/** The lifted row, and where its copy is. */
export interface GroupDrag {
  /** The group that is lifted. */
  id: string;
  /** The name on the row. */
  label: string;
  /** The place of the pointer, in the window. */
  x: number;
  y: number;
  /** How far into the handle the pointer grabbed it. */
  grabX: number;
  grabY: number;
  /** The width of the row. */
  width: number;
  /** The height of the row. */
  height: number;
  /** The accent of the group that is lifted. */
  tone: string;
}

export interface GroupReorder {
  /** The copy that follows the pointer, or null when no row is lifted. */
  drag: GroupDrag | null;
  /** Where the marker stands, in the list's own coordinates, or null when there is none. */
  marker: number | null;
  /** A press on a handle: it may become a drag. */
  begin: (group: Group, event: React.PointerEvent<HTMLButtonElement>) => void;
  /** The browser tried to drag the row away: a row is a control, not a picture. */
  refuse: (event: React.DragEvent<HTMLButtonElement>) => void;
}

/**
 * The drag of one group row.
 *
 * `list` is the box the rows are in; the marker stands in the gap the row would land in, in that
 * box's own coordinates, so it stays in the gap even when the page scrolls.
 */
export function useGroupReorder(
  groups: Group[],
  list: React.RefObject<HTMLElement | null>,
  onReorder: (order: string[]) => void,
): GroupReorder {
  const [drag, setDrag] = useState<GroupDrag | null>(null);
  const [marker, setMarker] = useState<number | null>(null);
  /** The press that may become a drag. */
  const press = useRef<Press | null>(null);
  /** The gap the lifted row would land in. */
  const gap = useRef<{ before: string | null; top: number } | null>(null);
  /** The press became a drag. */
  const lifted = useRef(false);

  /** The rows of the list, without the one that is lifted. */
  const rest = useCallback((): HTMLElement[] => {
    const el = list.current;
    if (!el) return [];
    const id = press.current?.id;
    return [...el.querySelectorAll<HTMLElement>("[data-group]")].filter(
      (row) => row.dataset.group !== id,
    );
  }, [list]);

  /**
   * The gap the row would land in: the first row that is under the pointer. The marker's place is
   * in the list's box, so it stays in the gap when the page scrolls.
   */
  const gapAt = useCallback(
    (y: number) => {
      const el = list.current;
      if (!el) return null;
      const rows = rest();
      const box = el.getBoundingClientRect();
      for (const row of rows) {
        const r = row.getBoundingClientRect();
        if (y < r.top + r.height / 2) {
          return { before: row.dataset.group ?? null, top: r.top - box.top - GAP };
        }
      }
      const last = rows[rows.length - 1];
      const top = last ? last.getBoundingClientRect().bottom - box.top + GAP : 0;
      return { before: null, top };
    },
    [list, rest],
  );

  /** The order the rows would have, with the lifted one in the gap. */
  const orderWith = useCallback(
    (id: string, before: string | null) => {
      const ids = groups.map((g) => g.id);
      const others = ids.filter((x) => x !== id);
      const at = before === null ? others.length : others.indexOf(before);
      const index = at < 0 ? others.length : at;
      return [...others.slice(0, index), id, ...others.slice(index)];
    },
    [groups],
  );

  const begin = useCallback((group: Group, event: React.PointerEvent<HTMLButtonElement>) => {
    // A finger does not drag: the page scrolls, and the group's menu is the phone's path.
    if (event.pointerType !== "mouse" && event.pointerType !== "pen") return;
    // The press never starts a native drag or a text selection, and the handle keeps the focus so
    // the keyboard still works after a mouse press.
    cancelPress(event, { focus: true });
    const rect = event.currentTarget.getBoundingClientRect();
    press.current = {
      id: group.id,
      label: group.name,
      element: event.currentTarget,
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      grabX: event.clientX - rect.left,
      grabY: event.clientY - rect.top,
      width: rect.width,
      height: rect.height,
    };
    // The handle holds the pointer while the drag lasts: the drag ends where the pointer lets go,
    // even outside the window.
    const el = event.currentTarget as unknown as Capture;
    try {
      el.setPointerCapture?.(event.pointerId);
    } catch {
      // A window without pointer capture: the drag still ends on the pointerup.
    }
    lifted.current = false;
  }, []);

  const refuse = useCallback((event: React.DragEvent<HTMLButtonElement>) => {
    refuseDragStart(event);
  }, []);

  useEffect(() => {
    const move = (event: PointerEvent) => {
      const start = press.current;
      if (!start) return;
      // The pointer is not down any more: the button was let go of outside the window, or the
      // drag was taken away. The drag ends here, without a write.
      if (event.buttons === 0) {
        stop(false);
        return;
      }
      if (!lifted.current) {
        // The pointer has travelled past the threshold: the row is lifted. The distance counts,
        // not the axes: a drag along the list is a drag.
        const dx = event.clientX - start.x;
        const dy = event.clientY - start.y;
        if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
        lifted.current = true;
      }
      const where = gapAt(event.clientY);
      if (where) {
        gap.current = where;
        setMarker(where.top);
      }
      setDrag({
        id: start.id,
        label: start.label,
        x: event.clientX,
        y: event.clientY,
        grabX: start.grabX,
        grabY: start.grabY,
        width: start.width,
        height: start.height,
        tone: groupAccent(start.id),
      });
    };

    const stop = (dropped: boolean) => {
      const start = press.current;
      press.current = null;
      if (!start) return;
      // The handle lets the pointer go, whichever way the drag ended.
      releaseCapture(start);
      if (!lifted.current) {
        // A press, not a drag: nothing moves.
        setDrag(null);
        setMarker(null);
        return;
      }
      const where = gap.current;
      const was = groups.map((g) => g.id);
      // The order the rows would have, with the lifted one in the gap. A group that was deleted
      // while the drag lasted is gone from the scene, so it is not in the order either: the drop
      // then changes nothing, and nothing is written and no undo step is made.
      const here = new Set(was);
      const order = orderWith(start.id, where?.before ?? null).filter((id) => here.has(id));
      lifted.current = false;
      gap.current = null;
      setDrag(null);
      setMarker(null);
      if (dropped && order.join("|") !== was.join("|")) onReorder(order);
    };

    const key = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || !press.current) return;
      // The drag is called off: nothing is written.
      releaseCapture(press.current);
      press.current = null;
      lifted.current = false;
      gap.current = null;
      setDrag(null);
      setMarker(null);
    };

    const up = () => stop(true);
    const cancel = () => stop(false);
    // The window lost the focus: the drag is over, and nothing is written.
    const blur = () => stop(false);

    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", key);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key);
      window.removeEventListener("blur", blur);
    };
  }, [gapAt, groups, onReorder, orderWith]);

  // The page goes away while a drag lasts: the handle lets the pointer go, so the browser is not
  // left holding a capture for a node that is gone.
  useEffect(
    () => () => {
      const start = press.current;
      if (start) releaseCapture(start);
    },
    [],
  );

  return { drag, marker, begin, refuse };
}
