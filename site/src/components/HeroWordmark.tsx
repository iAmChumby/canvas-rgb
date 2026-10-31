"use client";

import { useEffect, useRef } from "react";
import { Wordmark } from "@editor/components/BrandMark";

/** The editor's spectrum, read from its own tokens. */
const SPECTRUM = ["--rgb-1", "--rgb-2", "--rgb-3", "--rgb-4", "--rgb-5", "--rgb-6"];

/**
 * The model (issue #15): the light FOLLOWS THE POINTER, the way a cursor spotlight does, with
 * a short ease that runs on time and not on frames. Each frame it closes `1 - exp(-dt / tau)`
 * of the gap to the pointer, so it trails by the same amount of time at 30, 60 and 144 Hz.
 */
const TAU_FOLLOW = 0.075;
/** And back onto the wordmark, once the pointer has left the hero: slower, so it settles. */
const TAU_SETTLE = 0.45;
/** The light's radius, as a share of the hero's smaller side, so it is round at every size. */
const RADIUS = 0.42;
/**
 * The frame's fade band, as a share of the hero's smaller side, inside these bounds. The band
 * hides the last few pixels of the canvas, so the light never shows a cut against the frame,
 * and the light's centre is kept the same distance inside the frame, so its own peak is never
 * inside the band (which would dim the pool exactly where the pointer is).
 */
const FADE_MIN = 24;
const FADE_MAX = 48;
/** The field is drawn at this share of the hero's size and scaled up by CSS: the light is soft,
    so it loses nothing, and the canvas stays cheap. One scale for both axes. */
const SCALE = 0.45;
/** How long the spectrum in the easel's canvas takes to sweep across, once, on load. */
const SWEEP = 0.62;

/**
 * The hero.
 *
 * The mark is the canvas-rgb wordmark, the editor's own, with the easel as the "a". Behind it
 * is one soft pool of light in the accent's colour: two neighbouring hues, no rainbow.
 *
 * The light is the cursor spotlight: it sits where the pointer is, inside the hero, and trails
 * it by a short, time-based ease. It never leans and never stops short: the pool's centre is
 * the pointer, to within the frame's fade band. When the pointer leaves, the pool settles back
 * onto the wordmark. The frame fades the pool out over the last few pixels, so the canvas
 * never shows an edge, and the pool's tail is dimmed as it nears the frame rather than cut.
 *
 * Nothing reads layout per frame. The hero's and the mark's boxes are measured on resize (and
 * once the entrance has finished), in page coordinates; a pointer move and a scroll are only
 * arithmetic on cached numbers.
 *
 * The little rectangle inside the easel is the canvas, and it shows the editor's spectrum,
 * sliding under the pointer. It is small and crisp: no blur, no glow.
 *
 * Under `prefers-reduced-motion`, and on a screen with no hover (touch), the hero draws one
 * still frame behind the wordmark and asks for no other.
 */
export function HeroWordmark() {
  const wrapRef = useRef<HTMLSpanElement>(null);
  const paneRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const wrap = wrapRef.current;
    const pane = paneRef.current;
    if (!wrap || !pane) return;
    const hero = wrap.closest(".site-hero");
    if (!hero) return;
    // The field of light is a sibling of the content column, so it covers the whole hero. It
    // is the frame everything below is measured in.
    const field = hero.querySelector<HTMLCanvasElement>(".site-hero-field");
    if (!field) return;
    const ctx = field.getContext("2d", { alpha: true });
    const paneCtx = pane.getContext("2d");
    if (!ctx || !paneCtx) return;

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    const noHover = window.matchMedia("(hover: none)");
    /** With the motion off the hero is one still frame: no pool that follows, no sweep. */
    const still = () => reduced.matches || noHover.matches;
    const root = getComputedStyle(document.documentElement);
    const colours = SPECTRUM.map((n) => root.getPropertyValue(n).trim() || "#ff3b5c");

    // Everything below is cached by measure(); nothing is read from the DOM per frame.
    let frameW = 1;
    let frameH = 1;
    let fw = 1;
    let fh = 1;
    // The frame's top-left in page pixels, and the page's scroll, so a client position
    // becomes a position in the frame by arithmetic alone.
    let framePageX = 0;
    let framePageY = 0;
    let scrollX = window.scrollX;
    let scrollY = window.scrollY;
    // The wordmark's box in the frame's own pixels: its centre is where the light rests.
    let markX = 1;
    let markY = 1;
    let markL = 0;
    let markT = 0;
    let markW = 1;
    let markH = 1;
    let rad = 1;
    // The frame's fade band, and the distance the light's centre keeps from the frame.
    let fade = FADE_MIN;

    // The pointer in the frame's pixels, the light, and the last client position.
    const target = { x: 0, y: 0 };
    const pos = { x: 0, y: 0 };
    let clientX = 0;
    let clientY = 0;
    let seen = false;
    let inside = false;

    /** The scheme, read once and then kept up to date: no DOM read per frame. */
    const isLight = () => {
      const attr = document.documentElement.getAttribute("data-theme");
      if (attr === "light") return true;
      if (attr === "dark") return false;
      return window.matchMedia("(prefers-color-scheme: light)").matches;
    };
    let light = isLight();
    const scheme = window.matchMedia("(prefers-color-scheme: light)");
    const theme = new MutationObserver(() => {
      light = isLight();
    });
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const onScheme = () => {
      light = isLight();
    };
    scheme.addEventListener("change", onScheme);

    const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

    const measure = () => {
      scrollX = window.scrollX;
      scrollY = window.scrollY;
      const box = field.getBoundingClientRect();
      frameW = Math.max(1, box.width);
      frameH = Math.max(1, box.height);
      framePageX = box.left + scrollX;
      framePageY = box.top + scrollY;
      // One scale for both axes, so the light stays round: the backing store has the same
      // shape as the box it is drawn in.
      fw = Math.max(1, Math.round(frameW * SCALE));
      fh = Math.max(1, Math.round((fw * frameH) / frameW));
      field.width = fw;
      field.height = fh;
      const mark = wrap.getBoundingClientRect();
      markL = mark.left - box.left;
      markT = mark.top - box.top;
      markW = Math.max(1, mark.width);
      markH = Math.max(1, mark.height);
      markX = markL + markW / 2;
      markY = markT + markH / 2;
      // The light is a circle, and its size follows the smaller side of the frame, so it is
      // round and the same shape at every window size.
      rad = Math.min(frameW, frameH) * RADIUS;
      // The fade band and the inset are one number: the band hides the frame, and the inset
      // keeps the pool's peak out of the band, so the pool is at full strength wherever the
      // pointer is.
      fade = clamp(Math.min(frameW, frameH) * 0.05, FADE_MIN, FADE_MAX);
      field.style.setProperty("--fade", `${fade}px`);
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

    measure();
    place();
    // The light starts on the mark, never in a corner.
    pos.x = markX;
    pos.y = markY;

    /** Arithmetic only: the last client position, in the frame's pixels. */
    const retarget = () => {
      const x = clientX + scrollX - framePageX;
      const y = clientY + scrollY - framePageY;
      // The centre keeps its distance from the frame: the fade band hides the last few pixels,
      // and the pool's peak never sits inside it.
      target.x = clamp(x, fade, Math.max(fade, frameW - fade));
      target.y = clamp(y, fade, Math.max(fade, frameH - fade));
    };

    const onMove = (e: Event) => {
      const ev = e as PointerEvent;
      // A finger is not a pointer to follow: touch gets the still frame.
      if (ev.pointerType === "touch" || still()) return;
      clientX = ev.clientX;
      clientY = ev.clientY;
      seen = true;
      inside = true;
      retarget();
    };
    const onLeave = () => {
      inside = false;
    };
    const onScroll = () => {
      scrollX = window.scrollX;
      scrollY = window.scrollY;
      if (seen) retarget();
    };
    hero.addEventListener("pointermove", onMove, { passive: true });
    hero.addEventListener("pointerdown", onMove, { passive: true });
    hero.addEventListener("pointerleave", onLeave, { passive: true });
    hero.addEventListener("pointercancel", onLeave, { passive: true });
    window.addEventListener("scroll", onScroll, { passive: true });

    /** The pool of light: one colour, soft, fading out completely, always round. */
    const drawField = (t: number, dt: number) => {
      const following = inside && seen;
      const wantX = following ? target.x : markX;
      const wantY = following ? target.y : markY;
      if (dt > 0) {
        // Time, not frames: the same gap closes in the same seconds at any refresh rate.
        const k = 1 - Math.exp(-dt / (following ? TAU_FOLLOW : TAU_SETTLE));
        pos.x += (wantX - pos.x) * k;
        pos.y += (wantY - pos.y) * k;
      } else {
        pos.x = wantX;
        pos.y = wantY;
      }
      // One colour, the accent's, drifting a little either side of it.
      const hue = 214 + Math.sin(t * 0.11) * 11;
      const a = light ? 0.2 : 0.3;

      ctx.clearRect(0, 0, fw, fh);
      const cx = pos.x * SCALE;
      const cy = pos.y * SCALE;
      const r = rad * SCALE;
      const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
      g.addColorStop(0, `hsla(${hue}, 78%, ${light ? 58 : 56}%, ${a})`);
      g.addColorStop(0.35, `hsla(${hue}, 78%, ${light ? 60 : 58}%, ${a * 0.45})`);
      g.addColorStop(0.75, `hsla(${hue}, 78%, ${light ? 62 : 60}%, 0)`);
      g.addColorStop(1, "hsla(0, 0%, 0%, 0)");
      ctx.fillStyle = g;
      ctx.fillRect(0, 0, fw, fh);
    };

    /** The spectrum inside the easel's canvas: a small crisp band under the pointer. */
    const drawPane = (t: number, sweepOn: boolean) => {
      const w = pane.width;
      const h = pane.height;
      if (w < 2 || h < 2) return;
      paneCtx.clearRect(0, 0, w, h);
      let nx = seen ? (target.x - markL) / markW : 0.5 + Math.sin(t * 0.28) * 0.1;
      const ny = seen ? (target.y - markT) / markH : 0.5;
      let hueShift = (nx - 0.5) * 24;
      if (sweepOn) {
        // On load the spectrum sweeps once across the canvas, and then hands over to the
        // pointer. It is the same band, only moved.
        const s = Math.min(1, t / SWEEP);
        const ease = s * s * (3 - 2 * s);
        const sweepX = -0.7 + 1.4 * s;
        const sweepHue = -26 + 52 * s;
        nx = sweepX + (nx - sweepX) * ease;
        hueShift = sweepHue + (hueShift - sweepHue) * ease;
      }
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
      paneCtx.filter = `hue-rotate(${hueShift.toFixed(1)}deg)`;
      paneCtx.fillStyle = g;
      paneCtx.fillRect(0, 0, w, h);
      paneCtx.filter = "none";
    };

    let raf = 0;
    // The entrance is timed from the first paint, so the sweep in the easel's canvas starts
    // with the words settling in and not when the script happens to run.
    const paint = performance
      .getEntriesByType("paint")
      .find((e) => e.name === "first-contentful-paint");
    const t0 = paint ? paint.startTime : 0;
    let last = 0;
    const frame = (now: number) => {
      const t = (now - t0) / 1000;
      // The first frame, and a frame after a long pause (a hidden tab), take no big step.
      const dt = last ? Math.min(0.1, (now - last) / 1000) : 0.016;
      last = now;
      drawField(t, dt);
      drawPane(t, true);
      raf = requestAnimationFrame(frame);
    };
    /** One still frame: the light rests on the wordmark, and the easel's canvas does not move. */
    const paint1 = () => {
      seen = false;
      inside = false;
      pos.x = markX;
      pos.y = markY;
      drawField(0, 0);
      drawPane(0, false);
    };
    const start = () => {
      if (raf) return;
      last = 0;
      raf = requestAnimationFrame(frame);
    };
    const stop = () => {
      if (!raf) return;
      cancelAnimationFrame(raf);
      raf = 0;
    };
    /** Reduced motion and touch draw one frame and ask for no other. */
    const begin = () => {
      stop();
      if (still()) paint1();
      else start();
    };

    const ro = new ResizeObserver(() => {
      // Measuring resizes the canvases, and that clears them. With the loop stopped there is
      // nothing to paint them again, so the still frame is drawn here.
      measure();
      place();
      if (!raf) {
        if (still()) paint1();
        else start();
      }
    });
    ro.observe(hero);
    // The wordmark rises into place on load; its box is measured again once it has landed.
    const onLanded = (e: Event) => {
      if (e.target !== wrap) return;
      measure();
      place();
      if (!raf && still()) paint1();
    };
    wrap.addEventListener("animationend", onLanded);

    begin();
    reduced.addEventListener("change", begin);
    noHover.addEventListener("change", begin);

    return () => {
      stop();
      ro.disconnect();
      theme.disconnect();
      scheme.removeEventListener("change", onScheme);
      reduced.removeEventListener("change", begin);
      noHover.removeEventListener("change", begin);
      wrap.removeEventListener("animationend", onLanded);
      hero.removeEventListener("pointermove", onMove);
      hero.removeEventListener("pointerdown", onMove);
      hero.removeEventListener("pointerleave", onLeave);
      hero.removeEventListener("pointercancel", onLeave);
      window.removeEventListener("scroll", onScroll);
    };
  }, []);

  return (
    <span className="site-hero-wordmark" ref={wrapRef}>
      <Wordmark />
      <canvas ref={paneRef} className="site-hero-pane" aria-hidden="true" />
    </span>
  );
}
