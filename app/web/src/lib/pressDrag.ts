"use client";

import type { DragEvent, PointerEvent } from "react";

/**
 * The one rule for every control the pointer drags (#105, the class fix for
 * `cause:ui-interaction`): a press never starts a native drag or a text selection. Chromium
 * starts both from the compatibility mousedown a press would fire, so canceling the
 * `pointerdown` keeps the door shut. `refuseDragStart` catches anything the browser still
 * tries from inside the surface. A drag surface also carries `draggable={false}` where it is
 * an HTML element and `user-select: none` in its own CSS, as the second guard.
 *
 * A disabled control keeps the old press: a click on it must still blur — and so commit — a
 * field focused elsewhere, and that blur rides on the browser's own press behaviour.
 */

/**
 * Focuses a surface after a press so the keyboard keeps working, without claiming the
 * keyboard's focus ring: `focusVisible: false` is a request not to match `:focus-visible`, so
 * the ring stays pointer-blind for this focus. Engines that do not know the member ignore it.
 * Structural, so an HTML or an SVG surface both fit.
 */
export function focusForKeys(el: { focus(options?: FocusOptions): void }): void {
  el.focus({
    preventScroll: true,
    focusVisible: false,
  } as FocusOptions & { focusVisible: boolean });
}

/** Cancels a press on a drag surface, so no native drag or text selection can start from it. */
export function cancelPress<T extends HTMLElement | SVGElement>(
  e: PointerEvent<T>,
  opts?: { focus?: boolean },
): void {
  e.preventDefault();
  if (opts?.focus) focusForKeys(e.currentTarget);
}

/** Refuses any dragstart the browser still tries from inside the surface. */
export function refuseDragStart<T extends Element>(e: DragEvent<T>): void {
  e.preventDefault();
}
