"use client";

import { useEffect, useRef, type ReactNode } from "react";
import { boundsOf, placeTooltip, TIP_MARGIN, type TipAlign, type TipRect, type TipSide } from "@/lib/tooltipPlacement";

const SIDES: readonly TipSide[] = ["top", "bottom", "left", "right"];
const ALIGNS: readonly TipAlign[] = ["start", "center", "end"];

/**
 * A clip narrower than a readable line cannot hold a tooltip: the window is used instead, and
 * the bubble is left as it was. (A rail can clip its own links while their bubbles are meant to
 * reach over the page.) A bubble that such a clip still cuts is what `overflow-check.mjs
 * --hover-tips` reports.
 */
const MIN_TEXT_WIDTH = 120;

/** The window, cut down by every ancestor that clips the bubble. */
function boundsAround(bubble: HTMLElement): TipRect {
  const doc = document.documentElement;
  let box = boundsOf({ width: doc.clientWidth, height: doc.clientHeight });
  for (let el = bubble.parentElement; el && el !== document.body; el = el.parentElement) {
    const cs = getComputedStyle(el);
    if (cs.overflowX === "visible" && cs.overflowY === "visible") continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    // A scroll container clips at its padding edge: inside its borders and short of its
    // scrollbar, which `clientWidth`/`clientHeight` already leave out.
    const left = r.left + (parseFloat(cs.borderLeftWidth) || 0);
    const top = r.top + (parseFloat(cs.borderTopWidth) || 0);
    const right = Math.min(box.left + box.width, left + el.clientWidth);
    const bottom = Math.min(box.top + box.height, top + el.clientHeight);
    box = { left: Math.max(box.left, left), top: Math.max(box.top, top), width: 0, height: 0 };
    box.width = Math.max(0, right - box.left);
    box.height = Math.max(0, bottom - box.top);
  }
  return box;
}

/**
 * A hover and focus label for an icon control. The bubble's CSS says where it would like to sit;
 * while it is shown it is measured and moved to stay inside the window (#271), and it wraps
 * when the window has no room for one line of it. Reduced motion needs nothing here: the move
 * has no transition, and the stylesheet's reduced-motion block already drops the fade.
 */
export function Tooltip({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  const tip = useRef<HTMLSpanElement>(null);
  const bubble = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    const host = tip.current;
    const bub = bubble.current;
    if (!host || !bub) return;

    let over = false;
    let focused = false;
    let bound = false;

    const place = () => {
      const trigger = host.getBoundingClientRect();
      if (trigger.width === 0 || trigger.height === 0) return;
      const cs = getComputedStyle(bub);
      const wantedSide = cs.getPropertyValue("--tip-side").trim() as TipSide;
      const wantedAlign = cs.getPropertyValue("--tip-align").trim() as TipAlign;
      const gap = Number.parseFloat(cs.getPropertyValue("--tip-gap")) || 6;

      // A bubble wider than the room it has wraps instead of running off the window.
      bub.style.maxWidth = "";
      bub.style.whiteSpace = "";
      let bounds = boundsAround(bub);
      let room = bounds.width - 2 * TIP_MARGIN;
      if (room < MIN_TEXT_WIDTH) {
        bounds = boundsOf({ width: document.documentElement.clientWidth, height: document.documentElement.clientHeight });
        room = bounds.width - 2 * TIP_MARGIN;
      }
      let size = { width: bub.getBoundingClientRect().width, height: bub.getBoundingClientRect().height };
      if (size.width === 0 || size.height === 0) return;
      if (size.width > room) {
        bub.style.whiteSpace = "normal";
        bub.style.maxWidth = `${room}px`;
        const wrapped = bub.getBoundingClientRect();
        size = { width: wrapped.width, height: wrapped.height };
      }

      const at = placeTooltip({
        trigger: { left: trigger.left, top: trigger.top, width: trigger.width, height: trigger.height },
        bubble: size,
        bounds,
        side: SIDES.includes(wantedSide) ? wantedSide : "top",
        align: ALIGNS.includes(wantedAlign) ? wantedAlign : "center",
        gap,
      });
      // The bubble is positioned inside its `.tip`, which is the trigger.
      bub.style.left = `${at.left - trigger.left}px`;
      bub.style.top = `${at.top - trigger.top}px`;
      bub.style.right = "auto";
      bub.style.bottom = "auto";
      bub.style.transform = "none";
    };

    // A scroll, a resize or the trigger changing size moves it again, once per frame however many
    // events arrive. The first placement on show runs at once, so no frame shows the old spot.
    let frame = 0;
    const schedule = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        place();
      });
    };
    const resized = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    const unbind = () => {
      if (!bound) return;
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
      resized?.disconnect();
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      bound = false;
    };

    // The bubble shows on hover or on focus within, the same two states its CSS uses.
    const update = () => {
      if (!over && !focused) {
        unbind();
        return;
      }
      if (!bound) {
        window.addEventListener("resize", schedule);
        window.addEventListener("scroll", schedule, true);
        resized?.observe(host);
        bound = true;
      }
      place();
    };
    const enter = () => {
      over = true;
      update();
    };
    const leave = () => {
      over = false;
      update();
    };
    const focusIn = () => {
      focused = true;
      update();
    };
    const focusOut = (e: FocusEvent) => {
      if (e.relatedTarget instanceof Node && host.contains(e.relatedTarget)) return;
      focused = false;
      update();
    };

    host.addEventListener("pointerenter", enter);
    host.addEventListener("pointerleave", leave);
    host.addEventListener("focusin", focusIn);
    host.addEventListener("focusout", focusOut);
    return () => {
      host.removeEventListener("pointerenter", enter);
      host.removeEventListener("pointerleave", leave);
      host.removeEventListener("focusin", focusIn);
      host.removeEventListener("focusout", focusOut);
      unbind();
    };
  }, []);

  return (
    <span ref={tip} className={className ? `tip ${className}` : "tip"}>
      {children}
      <span ref={bubble} className="tip-bubble" role="tooltip">
        {label}
      </span>
    </span>
  );
}
