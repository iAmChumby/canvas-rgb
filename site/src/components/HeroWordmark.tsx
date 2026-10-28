"use client";

import { useEffect, useRef } from "react";
import { Wordmark } from "@editor/components/BrandMark";

/** The editor's spectrum, read from its own tokens. */
const SPECTRUM = ["--rgb-1", "--rgb-2", "--rgb-3", "--rgb-4", "--rgb-5", "--rgb-6"];

/**
 * The hero.
 *
 * The mark is the canvas-rgb wordmark, the editor's own, with the easel as the "a". Behind it
 * is one soft pool of light in the accent's colour: two neighbouring hues, drifting slowly,
 * easing toward the pointer. No rainbow, no hard edge: the field is drawn on a canvas that
 * is larger than the hero, so the light fades out before it reaches anything it could be
 * cut off by.
 *
 * The little rectangle inside the easel is the canvas, and it shows the editor's spectrum,
 * sliding under the pointer. It is small and crisp: no blur, no glow.
 *
 * Under `prefers-reduced-motion` both draw one still frame and stop.
 */
export function HeroWordmark() {
  const fieldRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLSpanElement>(null);
  const paneRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const field = fieldRef.current;
    const wrap = wrapRef.current;
    const pane = paneRef.current;
    if (!field || !wrap || !pane) return;
    const ctx = field.getContext("2d", { alpha: true });
    const paneCtx = pane.getContext("2d");
    if (!ctx || !paneCtx) return;
    const hero = field.closest(".site-hero");
    if (!hero) return;

    // The field is the hero's own frame: the light is drawn well inside it and fades out
    // before it reaches an edge, so no edge of the canvas can ever show. One scale for both
    // axes, so the light stays round.
    const SCALE = 0.45;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    const root = getComputedStyle(document.documentElement);
    const colours = SPECTRUM.map((n) => root.getPropertyValue(n).trim() || "#ff3b5c");

    let heroW = 1;
    let heroH = 1;
    let fw = 1;
    let fh = 1;
    // Where the mark is, so the light sits behind it.
    let markX = 1;
    let markY = 1;
    // The light's own size, and how far it reaches: it has to stay that far from every
    // edge, or the edge of the canvas would show.
    let rad = 1;
    let reach = 1;

    const clampX = (v: number) => Math.min(Math.max(v, reach), Math.max(reach, heroW - reach));
    const clampY = (v: number) => Math.min(Math.max(v, reach), Math.max(reach, heroH - reach));

    const measure = () => {
      const box = hero.getBoundingClientRect();
      heroW = Math.max(1, box.width);
      heroH = Math.max(1, box.height);
      // One scale for both axes, so the light stays round: the backing store has the same
      // shape as the box it is drawn in.
      fw = Math.max(1, Math.round(heroW * SCALE));
      fh = Math.max(1, Math.round((fw * heroH) / heroW));
      field.width = fw;
      field.height = fh;
      const mark = wrap.getBoundingClientRect();
      markX = mark.left - box.left + mark.width / 2;
      markY = mark.top - box.top + mark.height / 2;
      // The light is a circle, and its size follows the smaller side of the frame, so it is
      // round and the same shape at every window size.
      rad = Math.min(heroW, heroH) * 0.42;
      reach = rad * 0.75;
      // The light starts on the mark, never in a corner: easing from a corner would draw it
      // across the edge of the canvas on the way.
      pos.x = clampX(markX);
      pos.y = clampY(markY);
    };

    // The easel's canvas, measured from the mark itself, so the pane sits exactly on it.
    const place = () => {
      const svg = wrap.querySelector("svg");
      const rect = svg?.querySelector(".easel-canvas");
      if (!svg || !rect) return;
      const host = wrap.getBoundingClientRect();
      const r = rect.getBoundingClientRect();
      pane.style.left = `${r.left - host.left}px`;
      pane.style.top = `${r.top - host.top}px`;
      pane.style.width = `${r.width}px`;
      pane.style.height = `${r.height}px`;
      // The rectangle's own corner, so the pane has the shape of the canvas it covers. The
      // computed rx comes back in pixels ("1.3px"), so it is parsed and not cast.
      const cs = getComputedStyle(rect);
      const computed = parseFloat(cs.rx);
      const rx = Number.isFinite(computed) && computed > 0 ? computed : Number(rect.getAttribute("rx") ?? 0);
      const unit = Number(rect.getAttribute("width") || 1);
      const radius = Number.isFinite(rx) && unit > 0 ? (rx * r.width) / unit : 0;
      pane.style.borderRadius = radius.toFixed(2) + "px";
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      pane.width = Math.max(1, Math.round(r.width * dpr));
      pane.height = Math.max(1, Math.round(r.height * dpr));
    };

    // The pointer, in the hero's own pixels.
    const target = { x: 0, y: 0 };
    const pos = { x: 0, y: 0 };
    let seen = false;
    let hue = 214;

    measure();
    place();

    const onMove = (e: Event) => {
      const ev = e as PointerEvent;
      const box = hero.getBoundingClientRect();
      target.x = ev.clientX - box.left;
      target.y = ev.clientY - box.top;
      seen = true;
    };
    hero.addEventListener("pointermove", onMove, { passive: true });
    hero.addEventListener("pointerdown", onMove, { passive: true });

    const isLight = () => {
      const attr = document.documentElement.getAttribute("data-theme");
      if (attr === "light") return true;
      if (attr === "dark") return false;
      return window.matchMedia("(prefers-color-scheme: light)").matches;
    };

    /** The pool of light: one colour, soft, fading out completely, always round. */
    const drawField = (t: number) => {
      const light = isLight();
      const wantX = seen
        ? clampX(markX + (target.x - markX) * 0.4)
        : clampX(markX + Math.sin(t * 0.17) * heroW * 0.04);
      const wantY = seen
        ? clampY(markY + (target.y - markY) * 0.4)
        : clampY(markY + Math.cos(t * 0.13) * heroH * 0.04);
      pos.x += (wantX - pos.x) * 0.05;
      pos.y += (wantY - pos.y) * 0.05;
      // One colour, drifting a little either side of the accent.
      hue = 214 + Math.sin(t * 0.11) * 11;

      ctx.clearRect(0, 0, fw, fh);
      const cx = pos.x * SCALE;
      const cy = pos.y * SCALE;
      const r = rad * SCALE;
      // One colour, drifting a little either side of the accent.
      const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
      g.addColorStop(0, `hsla(${hue}, 78%, ${light ? 58 : 56}%, ${light ? 0.17 : 0.21})`);
      g.addColorStop(0.35, `hsla(${hue}, 78%, ${light ? 60 : 58}%, ${(light ? 0.17 : 0.21) * 0.45})`);
      g.addColorStop(0.75, `hsla(${hue}, 78%, ${light ? 62 : 60}%, 0)`);
      g.addColorStop(1, "hsla(0, 0%, 0%, 0)");
      ctx.fillStyle = g;
      ctx.fillRect(0, 0, fw, fh);
    };

    /** The spectrum inside the easel's canvas: a small crisp band under the pointer. */
    const drawPane = (t: number) => {
      const w = pane.width;
      const h = pane.height;
      if (w < 2 || h < 2) return;
      paneCtx.clearRect(0, 0, w, h);
      const host = wrap.getBoundingClientRect();
      const nx = seen
        ? (target.x + hero.getBoundingClientRect().left - host.left) / Math.max(1, host.width)
        : 0.5 + Math.sin(t * 0.28) * 0.1;
      const ny = seen
        ? (target.y + hero.getBoundingClientRect().top - host.top) / Math.max(1, host.height)
        : 0.5;
      // The band tilts with the pointer and slides under it.
      const ang = (ny - 0.5) * 0.3;
      const cx = w / 2 + (nx - 0.5) * w * 0.5;
      const cy = h / 2;
      const g = paneCtx.createLinearGradient(
        cx - Math.cos(ang) * w * 0.8,
        cy - Math.sin(ang) * h * 0.8,
        cx + Math.cos(ang) * w * 0.8,
        cy + Math.sin(ang) * h * 0.8,
      );
      for (let i = 0; i < colours.length; i++) {
        g.addColorStop(i / (colours.length - 1), colours[i] ?? "#ff3b5c");
      }
      // A little hue from the pointer, and no more.
      paneCtx.filter = `hue-rotate(${((nx - 0.5) * 24).toFixed(1)}deg)`;
      paneCtx.fillStyle = g;
      paneCtx.fillRect(0, 0, w, h);
      paneCtx.filter = "none";
    };

    let raf = 0;
    const frame = (now: number) => {
      drawField(now / 1000);
      drawPane(now / 1000);
      raf = requestAnimationFrame(frame);
    };
    const still = () => {
      seen = false;
      pos.x = markX;
      pos.y = markY;
      drawField(0);
      drawPane(0);
    };
    const start = () => {
      if (raf) return;
      raf = requestAnimationFrame(frame);
    };
    const stop = () => {
      if (!raf) return;
      cancelAnimationFrame(raf);
      raf = 0;
    };

    const ro = new ResizeObserver(() => {
      // Measuring resizes the canvases, and that clears them. With the motion off there is no
      // loop to paint them again, so the still frame is drawn here.
      measure();
      place();
      if (reduced.matches) still();
    });
    ro.observe(hero);

    if (reduced.matches) {
      still();
    } else {
      start();
    }
    const onReduced = () => {
      if (reduced.matches) {
        stop();
        still();
      } else {
        start();
      }
    };
    reduced.addEventListener("change", onReduced);

    return () => {
      stop();
      ro.disconnect();
      reduced.removeEventListener("change", onReduced);
      hero.removeEventListener("pointermove", onMove);
      hero.removeEventListener("pointerdown", onMove);
    };
  }, []);

  return (
    <>
      <canvas ref={fieldRef} className="site-hero-field" aria-hidden="true" />
      <span className="site-hero-wordmark" ref={wrapRef}>
        <Wordmark />
        <canvas ref={paneRef} className="site-hero-pane" aria-hidden="true" />
      </span>
    </>
  );
}
