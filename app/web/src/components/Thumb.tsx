"use client";

import { useEffect, useRef, useState } from "react";
import { loadThumb, type Thumb as ThumbData } from "@/lib/thumbs";
import { useDocumentVisible, usePrefersReducedMotion } from "@/lib/motion";

export type ThumbSize = "sm" | "md" | "lg";

const FPS = 12;

/**
 * A looping effect thumbnail, drawn on a small canvas and scaled up with pixelated rendering.
 * It loads when it scrolls into view, animates only while it is in view and the window is
 * visible, and under prefers-reduced-motion it shows frame 0 and stops.
 */
export function Thumb({ id, size = "sm", label }: { id: string; size?: ThumbSize; label: string }) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [inView, setInView] = useState(false);
  const [thumb, setThumb] = useState<ThumbData | null>(null);
  const docVisible = useDocumentVisible();
  const reduced = usePrefersReducedMotion();
  const animate = inView && docVisible && !reduced && thumb !== null;

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

  // Fetch once the tile is first in view.
  useEffect(() => {
    if (!inView || thumb) return;
    let live = true;
    void loadThumb(id).then((t) => {
      if (live) setThumb(t);
    });
    return () => {
      live = false;
    };
  }, [inView, id, thumb]);

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
    <div ref={wrapRef} className={`thumb thumb-${size}`} role="img" aria-label={`${label} preview`}>
      <canvas ref={canvasRef} aria-hidden="true" />
    </div>
  );
}
