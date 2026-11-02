"use client";

// The builder's picture (D95.5.2): the stack's own frames at 320 × 200, fitted to the stage
// without ever changing its ratio, with the picked layer's area drawn on it when `Show areas` is
// on. Frames are drawn straight into the canvas, so a 15 fps feed never re-renders the page.

import { useEffect, useRef } from "react";
import { onStackFrame, setStackPreviewStack, startStackPreview, stopStackPreview } from "@/lib/stackPreview";
import type { Layout } from "@/proto/Layout";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import {
  areaBounds,
  areaName,
  CANVAS_H,
  CANVAS_W,
} from "@/views/effects/stackDraft";

export function StackPreviewCanvas({
  stack,
  layout,
  devices,
  picked,
  showAreas,
}: {
  stack: Stack;
  layout: Layout | null;
  devices: DeviceInfo[] | null;
  picked: StackLayer | null;
  showAreas: boolean;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const empty = stack.layers.length === 0;

  useEffect(() => {
    startStackPreview();
    return () => stopStackPreview();
  }, []);

  useEffect(() => {
    setStackPreviewStack(empty ? null : stack, layout);
  }, [stack, layout, empty]);

  useEffect(
    () =>
      onStackFrame((frame) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        if (canvas.width !== frame.width) canvas.width = frame.width;
        if (canvas.height !== frame.height) canvas.height = frame.height;
        // The feed is RGB; a canvas image is RGBA.
        const image = ctx.createImageData(frame.width, frame.height);
        const out = image.data;
        for (let i = 0, o = 0; i < frame.rgb.length; i += 3, o += 4) {
          out[o] = frame.rgb[i] ?? 0;
          out[o + 1] = frame.rgb[i + 1] ?? 0;
          out[o + 2] = frame.rgb[i + 2] ?? 0;
          out[o + 3] = 255;
        }
        ctx.putImageData(image, 0, 0);
      }),
    [],
  );

  const bounds = picked && showAreas ? areaBounds(picked.area, layout) : null;
  return (
    <div className="rd-canvas-wrap">
      <div className="rd-canvas" data-empty={empty ? "true" : "false"}>
        {!empty && <canvas ref={canvasRef} width={CANVAS_W} height={CANVAS_H} aria-label="The stack's preview" />}
        {empty && (
          <div className="rd-canvas-empty">
            <strong>Nothing to show yet</strong>
            <span>Add a layer and its picture appears here.</span>
          </div>
        )}
        {bounds && picked && (
          <div
            className="rd-area-frame"
            style={{
              left: `${(bounds.x / CANVAS_W) * 100}%`,
              top: `${(bounds.y / CANVAS_H) * 100}%`,
              width: `${(bounds.width / CANVAS_W) * 100}%`,
              height: `${(bounds.height / CANVAS_H) * 100}%`,
            }}
          >
            <span className="rd-area-label">
              {picked.name} · {areaName(picked.area, layout, devices)}
            </span>
            <span className="handle nw" aria-hidden="true" />
            <span className="handle ne" aria-hidden="true" />
            <span className="handle sw" aria-hidden="true" />
            <span className="handle se" aria-hidden="true" />
          </div>
        )}
      </div>
    </div>
  );
}
