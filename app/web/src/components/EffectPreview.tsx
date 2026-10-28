"use client";

import { useEffect, useRef, useState } from "react";
import { useDocumentVisible } from "@/lib/motion";
import { useMotionOff } from "@/components/Motion";
import { onFrame } from "@/lib/preview";
import { useSnap } from "@/lib/store";
import { loadThumb, type Thumb } from "@/lib/thumbs";
import type { PropValue } from "@/proto/PropValue";

/**
 * The preview of one effect, drawn from what the daemon renders and nothing else.
 *
 * - The running effect shows the daemon's live preview feed (the WebSocket frames), so a
 *   property change appears the moment the daemon applies it.
 * - Any other effect shows its daemon-rendered thumbnail: a loop at the effect's default
 *   properties, because the daemon renders only the running effect with live properties.
 *
 * It holds no per-effect code. Each effect declares its properties once; the daemon draws it.
 */

export type PreviewValues = { [key: string]: PropValue };

/** The loop the daemon's thumbnails run on. */
const THUMB_FPS = 12;

type Surface = { ctx: CanvasRenderingContext2D; canvas: HTMLCanvasElement; img: ImageData | null };

/** Copies one RGB frame (3 bytes a pixel, starting at `offset`) onto the canvas. */
function blit(s: Surface, rgb: Uint8Array, offset: number, w: number, h: number): void {
  if (s.canvas.width !== w || s.canvas.height !== h || !s.img) {
    s.canvas.width = w;
    s.canvas.height = h;
    s.img = s.ctx.createImageData(w, h);
  }
  const out = s.img.data;
  for (let i = 0, o = 0, p = offset; i < w * h; i++, o += 4, p += 3) {
    out[o] = rgb[p] ?? 0;
    out[o + 1] = rgb[p + 1] ?? 0;
    out[o + 2] = rgb[p + 2] ?? 0;
    out[o + 3] = 255;
  }
  s.ctx.putImageData(s.img, 0, 0);
}

export function EffectPreview({ id, className }: { id: string; className?: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const activeId = useSnap((s) => s.status?.active_effect ?? null);
  const live = activeId === id;
  const [thumb, setThumb] = useState<{ id: string; data: Thumb } | null>(null);
  const motionOff = useMotionOff();
  const visible = useDocumentVisible();

  // The thumbnail: fetched once per effect (loadThumb keeps the cache).
  useEffect(() => {
    let current = true;
    void loadThumb(id).then((t) => {
      if (current && t) setThumb({ id, data: t });
    });
    return () => {
      current = false;
    };
  }, [id]);

  // Paint. The thumbnail loop runs until the first live frame arrives (or for good, when the
  // effect is not the running one); a live frame takes over and stops the loop.
  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    const surface: Surface = { ctx, canvas, img: null };
    let timer: ReturnType<typeof setInterval> | null = null;
    let off: (() => void) | null = null;
    let ended = false;

    const stopThumb = () => {
      if (timer) clearInterval(timer);
      timer = null;
    };
    const endLive = () => {
      ended = true;
      if (off) off();
      off = null;
    };

    if (thumb && thumb.id === id) {
      const t = thumb.data;
      const bytes = t.w * t.h * 3;
      let frame = 0;
      const draw = () => blit(surface, t.data, frame * bytes, t.w, t.h);
      draw();
      if (visible && !motionOff) {
        timer = setInterval(() => {
          frame = (frame + 1) % t.frames;
          draw();
        }, 1000 / THUMB_FPS);
      }
    }

    if (live && visible) {
      off = onFrame((f) => {
        if (ended) return;
        stopThumb();
        blit(surface, f.rgb, 0, f.width, f.height);
        // Reduced motion: one live frame, then stillness.
        if (motionOff) queueMicrotask(endLive);
      });
      if (ended) off = null;
    }

    return () => {
      stopThumb();
      endLive();
    };
  }, [id, live, thumb, motionOff, visible]);

  return <canvas ref={canvasRef} className={className ? `preview-canvas ${className}` : "preview-canvas"} aria-hidden="true" />;
}
