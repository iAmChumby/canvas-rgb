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
/**
 * The light's radius, as a share of the hero's smaller side, so it is round at every size.
 */
const RADIUS = 0.42;
/**
 * The field is drawn at this share of the hero's size and scaled up by CSS: the light is soft,
 * so it loses nothing, and the canvas stays cheap. One scale for both axes.
 */
const SCALE = 0.45;
/**
 * How far the field reaches past the hero, above and below, in CSS pixels. It is the band
 * the light's own edge is drawn over: below the hero the light fades out before the
 * field's edge, and at the top it fades out before the sticky header's edge, so neither
 * ever cuts it. The site's own stylesheet sets the same number on the field (`--reach`),
 * and the script reads it back from there.
 */
const REACH = 24;
/** How long the spectrum in the easel's canvas takes to sweep across, once, on load. */
const SWEEP = 0.62;
/**
 * The most elapsed time one frame is allowed to ease over. A frame that is late (a long
 * task, a hidden tab, a laptop waking up) does not get to close the whole gap in one go:
 * the elapsed time is capped, so the light eases back into place over the next frames
 * instead of snapping there. There is no cliff at any gap: every frame moves the light by
 * at most the same distance.
 */
const DT_MAX = 0.05;
/**
 * The furthest the light may travel in one frame, as a share of its own radius. It is the
 * same for every frame, whatever the gap, so a late frame slides the light into place
 * instead of jumping it.
 */
const MOVE_SHARE = 1 / 16;

/**
 * What the site's QA harness sees: one frame's timing, and the light's place in the
 * field's own pixels. It is how the ease's own numbers are read from inside the frame.
 */
type GlowFrame = {
  /** The frame's time, in seconds since the first paint. */
  t: number;
  /** The elapsed time this frame eased over, in seconds. It is capped. */
  dt: number;
  /** The real gap since the last frame, in seconds. */
  gap: number;
  /** The light's centre, in the field's own pixels. */
  x: number;
  y: number;
  /** The light's strength: 0 to 1. */
  alpha: number;
};

/**
 * The hero.
 *
 * The mark is the canvas-rgb wordmark, the editor's own, with the easel as the "a". Behind it
 * is one soft pool of light in the accent's colour: two neighbouring hues, no rainbow.
 *
 * The light is the cursor spotlight: its centre IS the pointer, inside the hero, and it
 * trails it by a short, time-based ease. It never leans and never stops short. When the
 * pointer leaves, the pool settles back onto the wordmark. The field of light reaches a
 * little past the hero, and the light is faded out before the field's own edge, so the
 * field never cuts it.
 *
 * The sticky header is opaque and lies over the top of the hero, so the light has to be
 * gone by the time it reaches the header's bottom edge: otherwise the header's border
 * would cut it and leave a straight line across the hero. The light is therefore taken
 * to nothing at that edge, and the pool dims as the pointer goes up to it. Its centre
 * stays on the pointer.
 *
 * Nothing reads layout per frame. The hero's and the mark's boxes are measured on resize (and
 * once the entrance has finished), in page coordinates; a pointer move and a scroll are only
 * arithmetic on cached numbers. The loop only runs while the hero is on screen.
 *
 * The little rectangle inside the easel is the canvas, and it shows the editor's spectrum,
 * sliding under the pointer. It is small and crisp: no blur, no glow.
 *
 * The rule is about the input, not about the kind of device: a mouse or a pen drives the
 * light, on any device, including one with no hover; a finger puts the hero back to one
 * still frame behind the wordmark; under `prefers-reduced-motion` the hero draws one
 * still frame and asks for no other.
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
    /** A mouse or a pen has driven the light: it follows, even on a screen with no hover. */
    let driven = false;
    /**
     * With the motion off the hero is one still frame: no pool that follows, no sweep.
     *
     * The motion is off when the viewer asked for it (reduced motion), or when the device
     * has no hover and nothing has driven the light yet (a phone: a finger is not a
     * pointer to follow). A mouse or a pen on a phone is a pointer to follow, and the
     * light follows it: the rule is about the input, not about the kind of device.
     */
    const still = () => reduced.matches || (noHover.matches && !driven);
    const root = getComputedStyle(document.documentElement);
    const colours = SPECTRUM.map((n) => root.getPropertyValue(n).trim() || "#ff3b5c");

    // Everything below is cached by measure(); nothing is read from the DOM per frame.
    let frameW = 1;
    let frameH = 1;
    let fw = 1;
    let fh = 1;
    // The field's own scale: the backing store's pixels for each CSS pixel. It is not exactly
    // SCALE (the store's size is rounded), and using the real one keeps the light where the
    // pointer is, to the pixel.
    let sx = SCALE;
    let sy = SCALE;
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
    /** How far the field reaches past the hero: the frame's fade is drawn over that band. */
    let reach = REACH;
    /**
     * The sticky header's height. It is at the top of the page and sticks to the top of
     * the window, so its bottom edge is always the same height in the window: that edge
     * is where the light has to be gone, or the header's border would cut it and leave a
     * straight line across the hero.
     */
    let headerH = 0;

    // The pointer in the frame's pixels, the light, and the last client position.
    const target = { x: 0, y: 0 };
    const pos = { x: 0, y: 0 };
    let clientX = 0;
    let clientY = 0;
    let seen = false;
    let inside = false;
    /** A finger has been used on the hero: the light is a still frame until a mouse comes. */
    let touched = false;
    /** The hero is on screen: the loop only runs while it is. */
    let visible = true;

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
      sx = fw / frameW;
      sy = fh / frameH;
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
      // How far the field reaches past the hero, from the site's own stylesheet, so the fade
      // at the frame is drawn over exactly the band the field covers.
      const said = parseFloat(getComputedStyle(field).getPropertyValue("--reach"));
      if (Number.isFinite(said) && said > 0) reach = said;
      // The sticky header's height, measured once (the header does not change with the
      // hero's size). Its bottom edge is the line the light must not cross.
      const head = document.querySelector(".site-header");
      const headBox = head?.getBoundingClientRect();
      headerH = headBox ? headBox.height : 0;
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
      // The light's centre IS the pointer: it is never pulled inside the frame, so the pool
      // is not short of the pointer at any edge or corner.
      target.x = clientX + scrollX - framePageX;
      target.y = clientY + scrollY - framePageY;
    };

    const onMove = (e: Event) => {
      const ev = e as PointerEvent;
      if (ev.pointerType === "touch") {
        // A finger is not a pointer to follow: the hero draws one still frame and stops.
        if (!touched) {
          touched = true;
          stop();
          paint1();
        }
        return;
      }
      // A mouse or a pen. It drives the light, on any device: a screen with no hover can
      // still have a mouse on it, and the light follows it.
      if (!driven) {
        driven = true;
        if (!reduced.matches && visible) start();
      }
      if (touched) {
        // A mouse (or a pen) has taken over again.
        touched = false;
        if (!still() && visible) start();
      }
      if (still()) return;
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

    /**
     * The light's own fade, drawn into the field with `destination-out`: the erase's value
     * at a height is the profile's, and it takes the light away in proportion, so the
     * light's edge is soft and never a line. The profile is sampled at its own breaks, so
     * the gradient is exactly it.
     *
     * `breaks` are [height, value] pairs in order, and the value between two of them is
     * linear. The value above the first break is the first one's, and below the last it
     * is the last one's.
     */
    const fade = (breaks: Array<[number, number]>) => {
      const first = breaks[0];
      const last = breaks[breaks.length - 1];
      if (!first || !last) return;
      const value = (y: number) => {
        if (y <= first[0]) return first[1];
        for (let i = 1; i < breaks.length; i++) {
          const b0 = breaks[i - 1];
          const b1 = breaks[i];
          if (!b0 || !b1) continue;
          const [y0, v0] = b0;
          const [y1, v1] = b1;
          if (y <= y1) return v0 + ((v1 - v0) * (y - y0)) / (y1 - y0 || 1);
        }
        return last[1];
      };
      const g = ctx.createLinearGradient(0, 0, 0, fh);
      const seen = new Set<string>();
      for (const y of [0, ...breaks.map((b) => b[0]), fh]) {
        if (y < 0 || y > fh) continue;
        const o = y / fh;
        const key = o.toFixed(6);
        if (seen.has(key)) continue;
        seen.add(key);
        g.addColorStop(o, `rgba(0, 0, 0, ${Math.min(1, Math.max(0, value(y)))})`);
      }
      ctx.fillStyle = g;
      ctx.fillRect(0, 0, fw, fh);
    };

    /**
     * The pool of light: one colour, soft, fading out completely, always round.
     *
     * `k` is the share of the gap to the pointer that this frame closes (0 leaves it alone,
     * 1 puts it straight on the pointer). `cap`, when it is not 0, is the furthest the pool
     * may travel in this frame.
     *
     * It returns the light's strength: how much of it is drawn, 0 to 1.
     */
    const drawField = (t: number, k: number, cap = 0) => {
      const following = inside && seen && !touched;
      const wantX = following ? target.x : markX;
      const wantY = following ? target.y : markY;
      if (k > 0) {
        const mx = (wantX - pos.x) * k;
        const my = (wantY - pos.y) * k;
        const d = Math.hypot(mx, my);
        if (cap > 0 && d > cap) {
          pos.x += (mx / d) * cap;
          pos.y += (my / d) * cap;
        } else {
          pos.x += mx;
          pos.y += my;
        }
      }
      // One colour, the accent's, drifting a little either side of it.
      const hue = 214 + Math.sin(t * 0.11) * 11;
      // The light's own strength: the accent's, the same as it always was.
      const a = light ? 0.17 : 0.21;

      ctx.clearRect(0, 0, fw, fh);
      const cx = pos.x * sx;
      const cy = pos.y * sy;
      // Where the sticky header's bottom edge is, in the field's own pixels. The header is
      // opaque and lies over the top of the hero, so the light has to be gone by the time
      // it gets there: otherwise the header's border would cut it and leave a straight
      // line across the hero. The header is at the top of the page and sticks to the top
      // of the window, so its bottom edge is always the same height in the window, and
      // the light's own edge moves with the page.
      const cut = headerH > 0 ? (scrollY + headerH - framePageY) * sy : -1e6;
      // How far below that edge the light's centre is, and how deep the light's own band
      // is: the band is the field's reach, the same band the light fades out over.
      const depth = cy - cut;
      const band = Math.max(1, reach * sy);
      // The light's strength near the header: it is taken to nothing by the header's edge,
      // so the pool dims as the pointer goes up to it, rather than being cut by it. The
      // pool's centre stays on the pointer: the fade below reaches the centre exactly.
      const strength = depth <= 0 ? 0 : Math.min(1, depth / band);
      if (strength > 0) {
        const r = rad * sx;
        ctx.globalAlpha = strength;
        const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
        g.addColorStop(0, `hsla(${hue}, 78%, ${light ? 58 : 56}%, ${a})`);
        g.addColorStop(0.35, `hsla(${hue}, 78%, ${light ? 60 : 58}%, ${a * 0.45})`);
        g.addColorStop(0.75, `hsla(${hue}, 78%, ${light ? 62 : 60}%, 0)`);
        g.addColorStop(1, "hsla(0, 0%, 0%, 0)");
        ctx.fillStyle = g;
        ctx.fillRect(0, 0, fw, fh);
        ctx.globalAlpha = 1;
      }
      // The frame's fade, drawn into the field itself: the light is taken to nothing over
      // the last pixels before the field's edge, so the field never cuts it. The band is
      // the field's own reach past the hero, so the fade happens outside the hero and the
      // light's centre, which is the pointer, is never dimmed by it: the light is
      // brightest where the pointer is, right to the edge. Drawn in the canvas rather than
      // with a CSS mask, so every browser gets the same soft edge. The field is as wide as
      // the hero, so the light runs off the left and right of the screen, where there is
      // nothing to see.
      ctx.globalCompositeOperation = "destination-out";
      if (headerH > 0) {
        // The header's edge: the light is erased over the band that reaches it, so that it
        // is gone by the time it gets there. The erase runs from the header's edge down to
        // the light's own centre, so the pool's peak stays on the pointer, and the pool's
        // strength is scaled to match, so the edge is soft at every distance. Above the
        // header's edge the light is left alone: that part of the field is behind the
        // opaque header and nobody sees it.
        const ramp = Math.min(Math.max(depth, 1), band);
        fade([
          [0, 0],
          [cut, 1],
          [cut + ramp, 0],
        ]);
      }
      // The field's own bottom edge: the same fade, over the field's own reach, so the
      // light is taken to nothing before the field's edge and the field never cuts it.
      fade([
        [fh - band, 0],
        [fh, 1],
      ]);
      ctx.globalCompositeOperation = "source-over";
      return strength;
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
      // A little hue from the pointer, and no more. Not every browser has the canvas filter
      // (WebKit does not), and then the band is simply drawn without the shift: that is a
      // colour and not an edge, so the soft edge of the light is the same everywhere.
      const hasFilter = typeof (paneCtx as unknown as { filter?: unknown }).filter === "string";
      if (hasFilter) paneCtx.filter = `hue-rotate(${hueShift.toFixed(1)}deg)`;
      paneCtx.fillStyle = g;
      paneCtx.fillRect(0, 0, w, h);
      if (hasFilter) paneCtx.filter = "none";
    };

    let raf = 0;
    // The entrance is timed from the first paint, so the sweep in the easel's canvas starts
    // with the words settling in and not when the script happens to run.
    const paint = performance
      .getEntriesByType("paint")
      .find((e) => e.name === "first-contentful-paint");
    const t0 = paint ? paint.startTime : 0;
    let last = 0;
    /**
     * The site's own QA hook: it is called once per frame with the frame's timing and the
     * light's place, so that a frame can be measured from the inside. Nothing in the site
     * listens to it; the harness does.
     */
    const hook = (globalThis as { __heroGlow?: (f: GlowFrame) => void }).__heroGlow;
    const frame = (now: number) => {
      const t = (now - t0) / 1000;
      // Time, not frames: the same gap closes in the same seconds at any refresh rate. The
      // elapsed time is capped, so a frame that is late (a long task, a hidden tab, a laptop
      // waking up) eases the light back into place over the next frames instead of snapping
      // it there. There is no cliff at any gap: the light's own move is capped too, so every
      // frame moves it by at most the same distance.
      const gap = last ? (now - last) / 1000 : 0;
      const dt = Math.min(gap || 1 / 60, DT_MAX);
      last = now;
      const following = inside && seen && !touched;
      const tau = following ? TAU_FOLLOW : TAU_SETTLE;
      const k = 1 - Math.exp(-dt / tau);
      // The light never crosses more than a sixteenth of its own radius in one frame: it
      // slides back into place instead of jumping.
      const strength = drawField(t, k, rad * MOVE_SHARE);
      drawPane(t, true);
      hook?.({ t, dt, gap, x: pos.x, y: pos.y, alpha: strength });
      raf = requestAnimationFrame(frame);
    };
    /** One still frame: the light rests on the wordmark, and the easel's canvas does not move. */
    const paint1 = () => {
      seen = false;
      inside = false;
      drawField(0, 1);
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
      if (still() || touched) paint1();
      else if (visible) start();
    };

    const ro = new ResizeObserver(() => {
      // Measuring resizes the canvases, and that clears them. With the loop stopped there is
      // nothing to paint them again, so the still frame is drawn here.
      measure();
      place();
      if (!raf) begin();
    });
    ro.observe(hero);
    // The hero off screen: the loop stops, so the page asks for no frames while the hero is
    // not there. It starts again when the hero comes back.
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          visible = entry.isIntersecting;
          if (!visible) stop();
          else begin();
        }
      },
      { threshold: 0 },
    );
    io.observe(hero);
    // The wordmark rises into place on load; its box is measured again once it has landed.
    const onLanded = (e: Event) => {
      if (e.target !== wrap) return;
      measure();
      place();
      if (!raf && (still() || touched)) paint1();
    };
    wrap.addEventListener("animationend", onLanded);

    begin();
    reduced.addEventListener("change", begin);
    noHover.addEventListener("change", begin);

    return () => {
      stop();
      ro.disconnect();
      io.disconnect();
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
