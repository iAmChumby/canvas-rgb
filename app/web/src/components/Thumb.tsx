"use client";

import { useEffect, useRef, useState } from "react";
import { loadThumb, type Thumb as ThumbData } from "@/lib/thumbs";
import { useDocumentVisible } from "@/lib/motion";
import { useMotionOff } from "@/components/Motion";

export type ThumbSize = "sm" | "md" | "lg";

/** The loop the daemon's thumbnails run on. */
const FPS = 12;

/**
 * A looping effect thumbnail, drawn on a small canvas and scaled up with pixelated rendering.
 *
 * It fetches its picture as soon as it is on the page: the gallery shows its whole library,
 * and a tile that scrolls into view already has its picture. (It used to wait for the tile to
 * scroll into view, which left the last row of the grid showing placeholders whenever the
 * row sat at the bottom edge of the window.)
 *
 * It runs the loop only while it is worth running: the pointer is on it, it has the keyboard
 * focus, or it is the effect that is selected (`live`). A gallery of twelve tiles therefore
 * runs one loop, not twelve, and the frame rate of the page is the frame rate of the page.
 *
 * Under reduced motion - the OS setting or the editor's own switch - it shows the first
 * frame and stops.
 */
export function Thumb({
  id,
  size = "sm",
  label,
  live = false,
}: {
  id: string;
  size?: ThumbSize;
  label: string;
  /** The tile runs its loop all the time, whether or not the pointer is on it. */
  live?: boolean;
}) {
  const wrapRef = useRef<HTMLSpanElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [inView, setInView] = useState(false);
  const [thumb, setThumb] = useState<ThumbData | null>(null);
  // The pointer is on the tile, or it has the keyboard focus.
  const [awake, setAwake] = useState(false);
  const docVisible = useDocumentVisible();
  const reduced = useMotionOff();
  const animate = inView && docVisible && !reduced && thumb !== null && (awake || live);

  // In view or not: observed once per tile.
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) setInView(e.isIntersecting);
      },
      { threshold: 0.01 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);

  // Fetch the picture straight away. `loadThumb` keeps one request per effect, so a tile that
  // mounts again does not ask the daemon for a second render.
  useEffect(() => {
    let live = true;
    void loadThumb(id).then((t) => {
      if (live) setThumb(t);
    });
    return () => {
      live = false;
    };
  }, [id]);

  // Paint: frame 0 whenever idle, and the frame sequence while animating.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !thumb) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    canvas.width = thumb.w;
    canvas.height = thumb.h;
    const img = ctx.createImageData(thumb.w, thumb.h);
    const px = thumb.w * thumb.h;
    let frame = 0;
    const paint = () => {
      const base = frame * px * 3;
      const out = img.data;
      const src = thumb.data;
      for (let i = 0, o = 0; i < px; i++, o += 4) {
        const s = base + i * 3;
        out[o] = src[s] ?? 0;
        out[o + 1] = src[s + 1] ?? 0;
        out[o + 2] = src[s + 2] ?? 0;
        out[o + 3] = 255;
      }
      ctx.putImageData(img, 0, 0);
    };
    paint();
    if (!animate) return;
    const timer = setInterval(() => {
      frame = (frame + 1) % thumb.frames;
      paint();
    }, 1000 / FPS);
    return () => clearInterval(timer);
  }, [thumb, animate]);

  return (
    <span
      ref={wrapRef}
      className={`thumb thumb-${size}`}
      data-loaded={thumb ? "true" : "false"}
      data-running={animate ? "true" : "false"}
      role="img"
      aria-label={`${label} preview`}
      onPointerEnter={() => setAwake(true)}
      onPointerLeave={() => setAwake(false)}
      onFocus={() => setAwake(true)}
      onBlur={() => setAwake(false)}
    >
      {thumb ? (
        <canvas ref={canvasRef} aria-hidden="true" />
      ) : (
        <span className="thumb-pending-label" aria-hidden="true">
          &middot;&middot;&middot;
        </span>
      )}
    </span>
  );
}
