"use client";

// The builder's drag session (#257 phase 2), moved out of StackBuilder.tsx unchanged in phase 4.
// One place lifts a tile or a row, follows the pointer or the keyboard, and says what a drop would
// do: a press that travels is a pointer drag, a finger lifts on a hold, Space/Enter lifts for the
// keyboard, and the release commits once (one undo step). The capture is released and the window
// listeners come off whether the session ends in a drop, a cancellation, a lost capture, a blur or
// the page unloading.

import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import {
  CANCEL_ANNOUNCEMENT,
  MASK_EMPTY_HINT,
  TOUCH_HOLD_MS,
  cycleTarget,
  dropAnnouncement,
  liftAnnouncement,
  lifted,
  liftsOnHold,
  liftsOnMove,
  maskTargets,
  NO_DRAG,
  pressMoved,
  readDropBoxes,
  reorderToSlot,
  stackSlots,
  stageTargets,
  targetAnnouncement,
  targetKey,
  type DragSource,
  type DragState,
  type TargetNames,
} from "@/lib/dragSession";
import { dropArea, dropTargetAt, dropTargetWords, pointInBox, type DropBoxes, type DropTarget } from "@/lib/dropTargets";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { LayerArea } from "@/proto/LayerArea";
import type { GroupCanvas } from "@/lib/groupCanvas";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";

/** A press that has not yet travelled far enough to be a drag (#257 phase 2). */
interface Press {
  source: DragSource;
  pointerId: number;
  pointerType: string;
  start: { x: number; y: number };
  lifted: boolean;
  timer: ReturnType<typeof setTimeout> | null;
  element: HTMLElement | null;
  move: (event: PointerEvent) => void;
  up: (event: PointerEvent) => void;
  /** The capture went away under a lifted press (the element left the page, the browser took it). */
  lost: () => void;
  /** Once lifted, a finger's move is the drag's, not a page scroll (dnd-kit's TouchSensor rule). */
  touch: (event: TouchEvent) => void;
  /** Escape ends a lifted pointer or touch drag, as it ends a keyboard one. */
  key: (event: KeyboardEvent) => void;
}

export function useDragSession({
  canvas,
  devices,
  draft,
  draftRef,
  picked,
  setAnnounce,
  setDraft,
  addEffect,
  applyMask,
}: {
  canvas: GroupCanvas | null;
  devices: DeviceInfo[] | null;
  draft: Stack;
  draftRef: { current: Stack };
  picked: string | null;
  setAnnounce: (words: string) => void;
  setDraft: (update: Stack | ((current: Stack) => Stack), run?: string, hold?: boolean) => void;
  addEffect: (effect: string, place?: { area?: LayerArea; at?: number }) => Promise<void> | void;
  applyMask: (id: string, effect: string) => Promise<boolean>;
}) {
  /** The library drag session (#257 phase 2): what is lifted and where it would land. */
  const [drag, setDrag] = useState<DragState>(NO_DRAG);
  const dragRef = useRef<DragState>(NO_DRAG);
  /** A press that has not yet become a drag: the tile or grip it started on, and its pointer. */
  const press = useRef<Press | null>(null);
  /** A drag just ended: the click that follows the release is not a tap-to-add. */
  const suppressClick = useRef(false);
  /** The safety timer that drops `suppressClick` when the click it waited for never came. */
  const suppressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** The drop boxes of the drag in progress, read once at the lift and again when the page moves. */
  const boxesRef = useRef<DropBoxes | null>(null);
  /** The release that follows a cancelled drag, still to be swallowed as a click. */
  const orphanUp = useRef<(() => void) | null>(null);

  const targetNames = (): TargetNames => ({ canvas, devices, layers: draftRef.current.layers });

  const showDrag = (next: DragState) => {
    dragRef.current = next;
    setDrag(next);
  };

  /** A target changed: the live region says where the drop would land now. */
  const sayTarget = (target: DropTarget | null) => {
    setAnnounce(target ? targetAnnouncement(target, targetNames()) : "");
  };

  const endPress = () => {
    const current = press.current;
    if (!current) return;
    if (current.timer !== null) clearTimeout(current.timer);
    window.removeEventListener("pointermove", current.move);
    window.removeEventListener("pointerup", current.up);
    window.removeEventListener("pointercancel", current.up);
    window.removeEventListener("touchmove", current.touch);
    window.removeEventListener("keydown", current.key, true);
    current.element?.removeEventListener("lostpointercapture", current.lost);
    press.current = null;
  };

  const clearDrag = () => {
    endPress();
    boxesRef.current = null;
    showDrag(NO_DRAG);
  };

  /** The click that follows a release or a key is not a tap-to-add; the next input or a second ends it. */
  const armSuppress = () => {
    suppressClick.current = true;
    if (suppressTimer.current !== null) clearTimeout(suppressTimer.current);
    suppressTimer.current = setTimeout(() => {
      suppressClick.current = false;
      suppressTimer.current = null;
    }, 1000);
  };

  /**
   * A Space or Enter that steers a lift is a key whose click must not add: Space clicks on its
   * keyup, so the keyup is swallowed with it, and the click it still sends finds the guard armed.
   */
  const guardKeyClick = () => {
    armSuppress();
    const release = (event: KeyboardEvent) => {
      if (event.key === " ") event.preventDefault();
    };
    window.addEventListener("keyup", release, { capture: true, once: true });
  };

  /**
   * The session ends with no drop: the capture is released, the ghost goes, the source is cleared
   * and the live region says nothing changed. A press that never lifted just ends, silently. The
   * release that may still come is swallowed as a click, so a cancelled drag is never also an add.
   */
  const cancelSession = () => {
    const wasLifted = dragRef.current.source !== null;
    const current = press.current;
    if (current?.lifted) {
      try {
        current.element?.releasePointerCapture(current.pointerId);
      } catch {
        // The capture is already gone.
      }
      const swallow = () => {
        window.removeEventListener("pointerup", swallow);
        window.removeEventListener("pointercancel", swallow);
        orphanUp.current = null;
        armSuppress();
      };
      window.addEventListener("pointerup", swallow);
      window.addEventListener("pointercancel", swallow);
      orphanUp.current = () => {
        window.removeEventListener("pointerup", swallow);
        window.removeEventListener("pointercancel", swallow);
        orphanUp.current = null;
      };
    }
    clearDrag();
    if (wasLifted) setAnnounce(CANCEL_ANNOUNCEMENT);
  };

  const moveDrag = (point: { x: number; y: number }, shift: boolean) => {
    const current = dragRef.current;
    if (!current.source) return;
    // The boxes were read at the lift and are read again when the page scrolls or resizes, not
    // on every move: a move only compares a point with them.
    const boxes = (boxesRef.current ??= readDropBoxes());
    const target = dropTargetAt({
      point,
      boxes,
      canvas,
      layers: draftRef.current.layers,
      payload: current.source.kind,
      shift,
    });
    // A mask over bare picture has nothing to land on: the drop would refuse, and the reason
    // follows the pointer (§4).
    const hint =
      current.source.kind === "mask" && target === null && pointInBox(point, boxes.stage)
        ? MASK_EMPTY_HINT
        : null;
    if (hint !== current.hint && hint !== null) setAnnounce(hint);
    else if (targetKey(target) !== targetKey(current.target)) sayTarget(target);
    showDrag({ ...current, target, point, hint });
  };

  /** The drop, applied once: one drop is one undo step. */
  const commitDrop = (source: DragSource, target: DropTarget | null) => {
    const names = targetNames();
    if (!target) {
      setAnnounce(source.kind === "mask" ? MASK_EMPTY_HINT : CANCEL_ANNOUNCEMENT);
      clearDrag();
      return;
    }
    if (source.kind === "mask") {
      if (target.kind === "row" || target.kind === "maskLayer") {
        clearDrag();
        // The drop's words wait for the effect read: a drop abandoned to an Undo in that gap says
        // nothing rather than announcing a mask that never landed (#257 review).
        void applyMask(target.id, source.effect).then((applied) => {
          if (applied) setAnnounce(dropAnnouncement(source, target, names));
        });
        return;
      }
      clearDrag();
      return;
    }
    if (source.kind === "effect") {
      if (target.kind === "stackSlot" || target.kind === "row") {
        void addEffect(source.effect, { at: target.kind === "stackSlot" ? target.at : target.index + 1 });
      } else {
        const area = dropArea(target);
        if (area) void addEffect(source.effect, { area });
      }
    } else if (target.kind === "stackSlot" || target.kind === "row") {
      // A drop on a row takes that row's place: it goes above it.
      const at = target.kind === "stackSlot" ? target.at : target.index + 1;
      setDraft((now) => ({ ...now, layers: reorderToSlot(now.layers, source.id, at) }));
    }
    setAnnounce(dropAnnouncement(source, target, names));
    clearDrag();
  };

  const beginKeyboard = (source: DragSource) => {
    const layers = draftRef.current.layers;
    let first: DropTarget | null;
    if (source.kind === "mask") {
      // A mask starts on the picked layer's row when there is one, otherwise the top row (the
      // last in the stack's own bottom-first order).
      const targets = maskTargets(layers);
      first =
        targets.find((target) => target.kind === "maskLayer" && target.id === picked) ??
        targets[targets.length - 1] ??
        null;
    } else if (source.kind === "effect") {
      first = stageTargets(canvas)[0] ?? null;
    } else {
      first = { kind: "stackSlot", at: Math.max(0, layers.findIndex((layer) => layer.id === source.id)) };
    }
    // The key that lifts is a key whose click must not also add the tile.
    guardKeyClick();
    showDrag(lifted(source, "keyboard", first));
    setAnnounce(liftAnnouncement(source));
  };

  const beginPress = (source: DragSource, event: ReactPointerEvent<HTMLElement>) => {
    if (event.button > 0 || dragRef.current.source) return;
    endPress();
    orphanUp.current?.();
    const pointerType = event.pointerType || "mouse";
    const start = { x: event.clientX, y: event.clientY };
    const element = event.currentTarget;
    const current: Press = {
      source,
      pointerId: event.pointerId,
      pointerType,
      start,
      lifted: false,
      timer: null,
      element,
      move: () => {},
      up: () => {},
      lost: () => {},
      touch: () => {},
      key: () => {},
    };
    /** The move or the hold went past the slop: the drag begins where the press did. */
    const lift = () => {
      if (current.lifted) return;
      current.lifted = true;
      if (current.timer !== null) {
        clearTimeout(current.timer);
        current.timer = null;
      }
      try {
        element.setPointerCapture(current.pointerId);
      } catch {
        // A pointer that is already gone has nothing to capture.
      }
      const boxes = readDropBoxes();
      boxesRef.current = boxes;
      const target = dropTargetAt({ point: start, boxes, canvas, layers: draftRef.current.layers, payload: source.kind });
      showDrag({ source, mode: pointerType === "touch" ? "touch" : "pointer", target, point: start, hint: null });
      setAnnounce(liftAnnouncement(source));
    };
    current.move = (e) => {
      if (e.pointerId !== current.pointerId || !press.current) return;
      if (!current.lifted) {
        if (!liftsOnMove(pointerType)) {
          // A finger that travels before the hold is a scroll, not a lift: the hold is off.
          if (pressMoved(start, { x: e.clientX, y: e.clientY })) endPress();
          return;
        }
        if (!pressMoved(start, { x: e.clientX, y: e.clientY })) return;
        lift();
      }
      // The press is the drag's: a finger does not scroll the page under the ghost.
      e.preventDefault();
      moveDrag({ x: e.clientX, y: e.clientY }, e.shiftKey);
    };
    current.up = (e) => {
      if (e.pointerId !== current.pointerId) return;
      const wasLifted = current.lifted;
      endPress();
      if (!wasLifted) return;
      // The click that follows a release must not also add the tile.
      armSuppress();
      if (e.type === "pointercancel") {
        setAnnounce(CANCEL_ANNOUNCEMENT);
        clearDrag();
        return;
      }
      commitDrop(source, dragRef.current.target);
    };
    // The capture went while the drag was lifted: the session ends rather than waiting for a
    // release that will not come to this element.
    current.lost = () => {
      if (press.current === current && current.lifted) cancelSession();
    };
    // `touch-action` lets the page pan, so a still hold can still scroll. After the lift the pan
    // must not start: the browser would take the gesture and send `pointercancel` (#257). A
    // non-passive touchmove is the one place that can stop it.
    current.touch = (e) => {
      if (current.lifted && e.cancelable) e.preventDefault();
    };
    current.key = (e) => {
      if (e.key !== "Escape" || !current.lifted) return;
      e.preventDefault();
      e.stopPropagation();
      cancelSession();
    };
    press.current = current;
    window.addEventListener("touchmove", current.touch, { passive: false });
    window.addEventListener("keydown", current.key, true);
    element.addEventListener("lostpointercapture", current.lost);
    window.addEventListener("pointermove", current.move, { passive: false });
    window.addEventListener("pointerup", current.up);
    window.addEventListener("pointercancel", current.up);
    if (liftsOnHold(pointerType)) {
      current.timer = setTimeout(() => {
        current.timer = null;
        lift();
      }, TOUCH_HOLD_MS);
    }
  };

  /** The keyboard's target step for Up/Down: the nearest slot, then one place at a time. */
  const slotStep = (current: DropTarget | null, step: 1 | -1): DropTarget | null => {
    const slots = stackSlots(draftRef.current.layers.length);
    if (current?.kind !== "stackSlot") return (step > 0 ? slots[slots.length - 1] : slots[0]) ?? null;
    return cycleTarget(slots, current, step);
  };

  // The keyboard sensor: the arrows pick, Enter drops, Escape cancels. Capture, and stopped there,
  // so a key that steers the drag is not also a key the tile or the row under it acts on: the
  // Enter that drops must not reach the focused tile and lift it again.
  useEffect(() => {
    if (!drag.source || drag.mode !== "keyboard") return;
    const source = drag.source;
    const onKey = (event: KeyboardEvent) => {
      const current = dragRef.current;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setAnnounce(CANCEL_ANNOUNCEMENT);
        clearDrag();
        return;
      }
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        event.stopPropagation();
        guardKeyClick();
        commitDrop(source, current.target);
        return;
      }
      if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
        if (source.kind !== "effect") return;
        event.preventDefault();
        event.stopPropagation();
        const next = cycleTarget(stageTargets(canvas), current.target, event.key === "ArrowRight" ? 1 : -1);
        showDrag({ ...current, target: next });
        sayTarget(next);
        return;
      }
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        event.preventDefault();
        event.stopPropagation();
        const step = event.key === "ArrowUp" ? 1 : -1;
        const next =
          source.kind === "mask"
            ? cycleTarget(maskTargets(draftRef.current.layers), current.target, step)
            : slotStep(current.target, step);
        showDrag({ ...current, target: next });
        sayTarget(next);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // The names and the handlers read the draft and the desk from the render they began in; a
    // move re-renders the page, and only a new lift starts a new listener.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag.source, drag.mode, canvas, devices]);

  // The session's loose ends, for the life of the builder: a window that loses focus or a tab
  // that goes to the background ends any press or drag (the pointer's release will not come
  // here), the next press or key ends a click guard, and leaving the page ends everything with
  // no setState and no add.
  const sessionRef = useRef({ cancel: cancelSession, end: endPress });
  useEffect(() => {
    sessionRef.current = { cancel: cancelSession, end: endPress };
  });
  useEffect(() => {
    const onBlur = () => sessionRef.current.cancel();
    const onHidden = () => {
      if (document.visibilityState === "hidden") sessionRef.current.cancel();
    };
    const onInput = () => {
      suppressClick.current = false;
    };
    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("pointerdown", onInput, true);
    window.addEventListener("keydown", onInput, true);
    return () => {
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", onHidden);
      window.removeEventListener("pointerdown", onInput, true);
      window.removeEventListener("keydown", onInput, true);
      // Leaving mid-press or mid-drag: the capture goes, the timer stops, the window listeners
      // come off. Nothing is set and nothing is added: the release that follows finds no press.
      const held = press.current;
      if (held?.lifted) {
        try {
          held.element?.releasePointerCapture(held.pointerId);
        } catch {
          // Nothing left to release.
        }
      }
      sessionRef.current.end();
      orphanUp.current?.();
      if (suppressTimer.current !== null) clearTimeout(suppressTimer.current);
      dragRef.current = NO_DRAG;
      boxesRef.current = null;
    };
  }, []);

  // The drop boxes are read when a drag lifts, and again when the page scrolls or resizes under it
  // or when the insertion line changes the list's height; a pointer move reads none.
  const liftedMode = drag.source ? drag.mode : null;
  useEffect(() => {
    if (liftedMode !== "pointer" && liftedMode !== "touch") return;
    const reread = () => {
      boxesRef.current = readDropBoxes();
    };
    reread();
    window.addEventListener("scroll", reread, true);
    window.addEventListener("resize", reread);
    return () => {
      window.removeEventListener("scroll", reread, true);
      window.removeEventListener("resize", reread);
    };
  }, [liftedMode]);
  const insertAt =
    drag.source && drag.target
      ? drag.target.kind === "stackSlot"
        ? drag.target.at
        : drag.target.kind === "row"
          ? drag.target.index + 1
          : null
      : null;
  useEffect(() => {
    if (liftedMode !== "pointer" && liftedMode !== "touch") return;
    boxesRef.current = readDropBoxes();
  }, [liftedMode, insertAt]);

  /** A tile's click: a tap or a click adds the effect on top, whole canvas (spec §4). */
  const onTileAdd = (effect: string) => {
    if (suppressClick.current) {
      suppressClick.current = false;
      return;
    }
    void addEffect(effect);
  };

  const onTileLiftStart = (effect: EffectSummary, event: ReactPointerEvent<HTMLButtonElement>, asMask: boolean) => {
    beginPress(
      asMask
        ? { kind: "mask", effect: effect.id, name: effect.name }
        : { kind: "effect", effect: effect.id, name: effect.name },
      event,
    );
  };

  const onTileLiftKey = (effect: EffectSummary, event: ReactKeyboardEvent<HTMLButtonElement>, asMask: boolean) => {
    if (dragRef.current.source) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    event.preventDefault();
    beginKeyboard(
      asMask
        ? { kind: "mask", effect: effect.id, name: effect.name }
        : { kind: "effect", effect: effect.id, name: effect.name },
    );
  };

  /** The grip's press: a mouse or pen lifts on the move, a finger on the hold. */
  const onGripDown = (layer: StackLayer, event: ReactPointerEvent<HTMLButtonElement>) => {
    beginPress({ kind: "layer", id: layer.id, name: layer.name }, event);
  };

  /** Space or Enter on the grip lifts the row for the keyboard sensor. */
  const onGripKey = (layer: StackLayer, event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (dragRef.current.source) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    event.preventDefault();
    beginKeyboard({ kind: "layer", id: layer.id, name: layer.name });
  };

  /** What the stage calls the target a drag is over, in the live region's own words. */
  const dragWords = drag.target
    ? dropTargetWords(drag.target, { canvas, devices, layers: draft.layers })
    : null;

  return {
    drag,
    /** A lift is live right now: read from the ref, which a keydown in the same turn sees at once. */
    isLifted: () => dragRef.current.source !== null,
    insertAt,
    dragWords,
    onTileAdd,
    onTileLiftStart,
    onTileLiftKey,
    onGripDown,
    onGripKey,
  };
}
