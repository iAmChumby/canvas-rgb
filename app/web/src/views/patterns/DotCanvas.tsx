"use client";

// The device drawn as its LEDs, large: one dot per LED, in the pattern's own colours, painted on
// a canvas. A press paints the dot under it, a drag paints everything it crossed (the segment
// between two pointer events, so a fast drag skips nothing), and the pointer never leaves the
// surface mid-stroke because the surface takes pointer capture.
//
// The dots' geometry is `dotLayout`'s: the drawing and the hit test use the same numbers, so the
// dot under the pointer is the dot that changes colour. The canvas is repainted when the colours,
// the box or the theme change, and never otherwise; it never re-renders React while painting.

import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { cancelPress, refuseDragStart } from "@/lib/pressDrag";
import { BLACK_HEX, parseHex } from "@/lib/color";
import { motionOffNow } from "@/lib/motion";
import type { Lab } from "@/lib/oklab";
import { evalPattern, frameLabs, timelineOf } from "@/lib/patternPlay";
import { labsOf, spinColours } from "@/lib/spin";
import { dotAt, dotsBetween, layoutDots, type Dot } from "@/views/patterns/dotLayout";
import type { BuiltOn } from "@/proto/BuiltOn";
import type { Pattern } from "@/proto/Pattern";
import type { Rgb8 } from "@/proto/Rgb8";
import s from "@/views/patterns/patterns.module.css";

/** The colours a dot is drawn from, read off the surface so the canvas speaks the theme's tokens. */
interface Inks {
  off: string;
  line: string;
}

/** The fallbacks are the dark theme's own values, for a surface outside the stylesheet's reach. */
function readInks(el: HTMLElement): Inks {
  const style = getComputedStyle(el);
  return {
    off: style.getPropertyValue("--led-off").trim() || "rgba(255,255,255,0.14)",
    line: style.getPropertyValue("--line-2").trim() || "rgba(255,255,255,0.14)",
  };
}

/** The ink of a colour, or null when the LED is off. */
function inkOf(colour: Rgb8 | undefined): string | null {
  const hex = colour === undefined ? null : parseHex(colour);
  if (hex === null) return null;
  // Black is off: the LED is drawn as an unlit socket, not as a black circle.
  return hex === BLACK_HEX ? null : hex;
}

export function DotCanvas({
  builtOn,
  colours,
  spin,
  play,
  playClock,
  overlay,
  ledOrder,
  label,
  onBegin,
  onStroke,
  onEnd,
}: {
  builtOn: BuiltOn;
  colours: Rgb8[];
  /**
   * Laps a second the picture turns at (#468), drawn with the daemon's own maths (`lib/spin.ts`)
   * from a clock that starts when the surface mounts. It stands still under reduced motion, and
   * while a press is down, so the dot under the pointer is the LED that is painted.
   */
  spin?: number | null;
  /**
   * The whole document, played (#468): the frames in time, fades in OKLab, then the spin of the
   * blended picture, drawn with the daemon's own maths (`lib/patternPlay.ts`) from a clock that
   * starts when it is set. Null shows `colours`, the frame being painted. It stands still under
   * reduced motion and while a press is down, as the spin does.
   */
  play?: Pick<Pattern, "frames" | "playback"> | null;
  /**
   * When the lights' clock started, in the page's `performance.now()` time (#481). The played
   * document runs from there, so the dots and the lights are in phase: the daemon starts a new
   * document's clock when its PUT lands. `null` is a PUT not yet answered: the dots wait on the first
   * moment. Left out, the played document runs from the first frame the dots draw, as before.
   */
  playClock?: number | null;
  /** Drawn over the surface, in its box (the Gradient tool's handles). */
  overlay?: ReactNode;
  /**
   * The LED-order notch and direction arrow (#468), drawn over the surface with the same dots the
   * LEDs use, so the notch lands on the physical LED it names. A render prop, because its geometry
   * is this surface's own measured layout.
   */
  ledOrder?: (dots: Dot[], box: { width: number; height: number }) => ReactNode;
  /** What a screen reader hears for the device. */
  label: string;
  /** A press landed: the next change is one history entry. */
  onBegin: () => void;
  onStroke: (indices: number[]) => void;
  onEnd: () => void;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [box, setBox] = useState({ width: 0, height: 0 });
  // Bumped when the theme changes, so the dots are read off the new tokens.
  const [theme, setTheme] = useState(0);
  /** Where the pointer was at the last event, in surface pixels. */
  const last = useRef<{ x: number; y: number } | null>(null);
  /** True while a press is down: a spinning picture holds still so the dot hit is the LED painted. */
  const held = useRef(false);
  const started = useRef<number | null>(null);

  const dots = useMemo(() => layoutDots(builtOn, box.width, box.height), [builtOn, box.width, box.height]);

  /** The surface's box now. Called before every pointer use, so a resized box is never stale. */
  const measure = useCallback(() => {
    const canvas = ref.current;
    if (!canvas) return { left: 0, top: 0, width: 0, height: 0 };
    const rect = canvas.getBoundingClientRect();
    setBox((was) =>
      was.width === rect.width && was.height === rect.height
        ? was
        : { width: rect.width, height: rect.height },
    );
    return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
  }, []);

  useEffect(() => {
    measure();
    if (typeof ResizeObserver !== "function") return;
    const canvas = ref.current;
    if (!canvas) return;
    const observer = new ResizeObserver(measure);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [measure]);

  useEffect(() => {
    const bump = () => setTheme((n) => n + 1);
    const root = document.documentElement;
    let observer: MutationObserver | null = null;
    if (typeof MutationObserver === "function") {
      observer = new MutationObserver(bump);
      observer.observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    }
    // The theme follows the OS until the owner picks one: the setting can change under the page.
    const media = typeof window.matchMedia === "function" ? window.matchMedia("(prefers-color-scheme: dark)") : null;
    media?.addEventListener?.("change", bump);
    return () => {
      observer?.disconnect();
      media?.removeEventListener?.("change", bump);
    };
  }, []);

  const labs = useMemo<Lab[] | null>(() => (spin ? labsOf(colours) : null), [colours, spin]);
  // The played document's timeline and OKLab values, made once per change to its frames.
  const played = useMemo(
    () => (play ? { timeline: timelineOf(play), labs: frameLabs(play.frames) } : null),
    [play],
  );
  // A new run of the preview starts from the top.
  const playing = play !== null && play !== undefined;
  useEffect(() => {
    started.current = null;
  }, [playing]);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    // No drawing context (a test, or a browser without canvas): the geometry and the pointer
    // still work, there is just nothing to see.
    if (!ctx) return;
    const { width, height } = canvas.getBoundingClientRect();
    if (width <= 0 || height <= 0) return;
    const dpr = Math.min(3, window.devicePixelRatio || 1);
    const pixelsW = Math.max(1, Math.round(width * dpr));
    const pixelsH = Math.max(1, Math.round(height * dpr));
    if (canvas.width !== pixelsW || canvas.height !== pixelsH) {
      canvas.width = pixelsW;
      canvas.height = pixelsH;
    }
    const inks = readInks(canvas);
    const draw = (shown: Rgb8[]) => {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);
      for (const dot of dots) {
        const ink = inkOf(shown[dot.index]);
        ctx.beginPath();
        ctx.arc(dot.x, dot.y, Math.max(0.5, dot.r - 0.5), 0, Math.PI * 2);
        if (ink !== null) {
          ctx.fillStyle = ink;
          ctx.fill();
        } else {
          // An unlit LED is a socket: a dim dot, with an edge once it is big enough to show one.
          ctx.fillStyle = inks.off;
          ctx.fill();
          if (dot.r >= 5) {
            ctx.strokeStyle = inks.line;
            ctx.lineWidth = 1;
            ctx.stroke();
          }
        }
      }
    };
    draw(colours);
    // A spinning pattern is redrawn every animation frame from its own clock. Reduced motion draws
    // the picture where it stands, and so does a press (see `held`).
    if ((!spin && !played) || motionOffNow() || typeof requestAnimationFrame !== "function") return;
    let raf = 0;
    const tick = (now: number) => {
      started.current ??= now;
      const ms = now - started.current;
      if (held.current) draw(colours);
      else if (played && play) {
        const origin = playClock === undefined ? started.current : playClock;
        draw(evalPattern(play, origin === null ? 0 : Math.max(0, now - origin), spin ?? null, played.timeline, played.labs));
      } else if (spin) draw(spinColours(colours, spin, ms / 1000, labs ?? undefined));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
    // `dots` carries the box; `theme` re-reads the tokens when it changes.
  }, [dots, colours, theme, spin, labs, played, play, playClock]);

  /** The pointer's place in the surface, and the dots of the box it is in. */
  const place = (event: ReactPointerEvent<HTMLCanvasElement>): { x: number; y: number; dots: Dot[] } => {
    const rect = event.currentTarget.getBoundingClientRect();
    return {
      x: event.clientX - rect.left,
      y: event.clientY - rect.top,
      dots: layoutDots(builtOn, rect.width, rect.height),
    };
  };

  const down = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    // A press on the device is a press on a surface, not the start of a text selection or a drag.
    cancelPress(event, { focus: true });
    event.currentTarget.setPointerCapture?.(event.pointerId);
    const at = place(event);
    last.current = { x: at.x, y: at.y };
    held.current = true;
    onBegin();
    const index = dotAt(at.dots, at.x, at.y);
    if (index !== null) onStroke([index]);
  };

  const move = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (!event.currentTarget.hasPointerCapture?.(event.pointerId)) return;
    const at = place(event);
    const from = last.current ?? { x: at.x, y: at.y };
    last.current = { x: at.x, y: at.y };
    const crossed = dotsBetween(at.dots, from, { x: at.x, y: at.y });
    if (crossed.length > 0) onStroke(crossed);
  };

  const up = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (last.current !== null) onEnd();
    last.current = null;
    held.current = false;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  return (
    <div className={s.surfaceBox}>
      <canvas
        ref={ref}
        className={s.surface}
        role="img"
        aria-label={label}
        draggable={false}
        onDragStart={refuseDragStart}
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerCancel={up}
      />
      {overlay}
      {ledOrder?.(dots, box)}
    </div>
  );
}
