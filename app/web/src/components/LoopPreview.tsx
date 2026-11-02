"use client";

import { useEffect, useRef } from "react";
import { useDocumentVisible } from "@/lib/motion";
import { useMotionOff } from "@/components/Motion";
import { blit, type Surface, type Thumb } from "@/lib/thumbs";

/** The loop the daemon's previews and thumbnails run on. */
const LOOP_FPS = 12;

/**
 * A loop the daemon rendered, played on a canvas: the effect editor's live preview, which holds
 * no effect code of its own and draws only what the daemon answered (D2.4). Reduced motion
 * draws one frame and stays still; a hidden window stops the timer.
 */
export function LoopPreview({
  thumb,
  className,
  label,
}: {
  thumb: Thumb | null;
  className?: string;
  /** What the picture is, for a reader: "Preview of Comet". */
  label: string;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const motionOff = useMotionOff();
  const visible = useDocumentVisible();

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx || !thumb) return;
    const surface: Surface = { ctx, canvas, img: null };
    const frameBytes = thumb.w * thumb.h * 3;
    let frame = 0;
    const draw = () => blit(surface, thumb.data, frame * frameBytes, thumb.w, thumb.h);
    draw();
    if (motionOff || !visible) return;
    const timer = setInterval(() => {
      frame = (frame + 1) % thumb.frames;
      draw();
    }, 1000 / LOOP_FPS);
    return () => clearInterval(timer);
  }, [thumb, motionOff, visible]);

  return (
    <canvas
      ref={canvasRef}
      className={className ? `preview-canvas ${className}` : "preview-canvas"}
      role="img"
      aria-label={label}
    />
  );
}
