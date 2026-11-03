"use client";

// The builder's picture (D95.5.2): the stack's own frames at 320 × 200, fitted to the stage
// without ever changing its ratio, with the picked layer's area drawn on it when `Show areas` is
// on. Frames are drawn straight into the canvas, so a 15 fps feed never re-renders the page.

import { useEffect, useRef, type KeyboardEvent, type PointerEvent } from "react";
import { cancelPress, refuseDragStart } from "@/lib/pressDrag";
import { hitTest, type HandleDef, type HitPart, type PartRef, type PointerKind } from "@/lib/hitTest";
import { onStackFrame, setStackPreviewStack, startStackPreview, stopStackPreview } from "@/lib/stackPreview";
import type { Layout } from "@/proto/Layout";
import type { Stack } from "@/proto/Stack";
import type { LayerArea } from "@/proto/LayerArea";
import type { StackLayer } from "@/proto/StackLayer";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import {
  areaBounds,
  areaName,
  CANVAS_H,
  CANVAS_W,
  moveBounds,
  pointerToCanvas,
  resizeBounds,
  type Bounds,
} from "@/views/effects/stackDraft";

// The one part the hit-test sees: the picked layer's rectangle.
const AREA_REF: PartRef = { kind: "zone", index: 0 };
const VIEW = { zoom: 1, panX: 0, panY: 0 };
/** An arrow key nudges the area this many canvas units; with Shift, the big step. */
const NUDGE = 1;
const NUDGE_BIG = 10;

const HANDLE_CURSOR: Record<HandleDef["id"], string> = {
  nw: "nwse-resize",
  se: "nwse-resize",
  ne: "nesw-resize",
  sw: "nesw-resize",
  n: "ns-resize",
  s: "ns-resize",
  e: "ew-resize",
  w: "ew-resize",
};

type Drag =
  | { mode: "move"; id: number; from: { x: number; y: number }; start: Bounds }
  | { mode: "resize"; id: number; from: { x: number; y: number }; start: Bounds; handle: HandleDef };

export function StackPreviewCanvas({
  stack,
  layout,
  devices,
  picked,
  showAreas,
  onAreaChange,
}: {
  stack: Stack;
  layout: Layout | null;
  devices: DeviceInfo[] | null;
  picked: StackLayer | null;
  showAreas: boolean;
  /** The picked layer's rectangle, dragged or nudged on the picture; the caller edits the draft. */
  onAreaChange?: (area: LayerArea) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
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

  // A layer that is switched off paints nothing, so its area is neither drawn nor dragged.
  const bounds = picked && picked.visible && showAreas ? areaBounds(picked.area, layout) : null;
  // Only a rectangle is dragged here. A whole-canvas, device or zone area follows the layout;
  // turning it into a rectangle is a choice the area picker asks for by name, so a stray drag
  // never does it.
  const editable = !!onAreaChange && picked?.area.kind === "rect";

  /** Where a press lands on the picked rectangle, by the Layout page's own hit-test. */
  const hitAt = (e: PointerEvent<HTMLDivElement>, area: Bounds) => {
    const box = stageRef.current?.getBoundingClientRect();
    if (!box) return null;
    const point = pointerToCanvas(e.clientX, e.clientY, box, VIEW);
    const part: HitPart = { ref: AREA_REF, ...area, rotation_deg: 0, isCase: false, locked: false };
    const pointer = (e.pointerType || "mouse") as PointerKind;
    const target = hitTest({
      point,
      parts: [part],
      selection: [AREA_REF],
      k: box.width / CANVAS_W,
      pointer,
    });
    return { box, point, target };
  };

  const emit = (area: Bounds) => onAreaChange?.({ kind: "rect", ...area });

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (!editable || !picked || picked.area.kind !== "rect" || e.button > 0) return;
    const start = { x: picked.area.x, y: picked.area.y, width: picked.area.width, height: picked.area.height };
    const hit = hitAt(e, start);
    if (!hit) return;
    const { target, point } = hit;
    let next: Drag | null = null;
    if (target.kind === "handle" && target.handle !== "rotate") {
      next = { mode: "resize", id: e.pointerId, from: point, start, handle: target.handle };
    } else if (target.kind === "part") {
      next = { mode: "move", id: e.pointerId, from: point, start };
    }
    if (!next) return;
    // The press is ours: no native drag, text selection or focus theft (lib/pressDrag.ts).
    cancelPress(e, { focus: true });
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // A pointer that is already gone has nothing to capture.
    }
    drag.current = next;
  };

  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!editable || !picked || picked.area.kind !== "rect") return;
    const d = drag.current;
    if (!d) {
      const hit = hitAt(e, picked.area);
      const t = hit?.target;
      e.currentTarget.style.cursor =
        t?.kind === "handle" && t.handle !== "rotate"
          ? HANDLE_CURSOR[t.handle.id]
          : t?.kind === "part"
            ? "move"
            : "";
      return;
    }
    if (d.id !== e.pointerId) return;
    // A mouse that moves with no button down means the release was missed: the drag is over.
    if (e.pointerType === "mouse" && e.buttons === 0) {
      drag.current = null;
      return;
    }
    const box = stageRef.current?.getBoundingClientRect();
    if (!box) return;
    const p = pointerToCanvas(e.clientX, e.clientY, box, VIEW);
    const dx = p.x - d.from.x;
    const dy = p.y - d.from.y;
    emit(
      d.mode === "move"
        ? moveBounds(d.start, dx, dy)
        : resizeBounds(d.start, d.handle.sx, d.handle.sy, dx, dy, e.shiftKey),
    );
  };

  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    drag.current = null;
    // A cancelled press (a call, a system gesture) puts the area back where it started.
    if (e.type === "pointercancel") emit(d.start);
  };

  // Capture lost without a pointerup (the window lost focus, the element went away): the drag
  // ends where it is.
  const onLost = (e: PointerEvent<HTMLDivElement>) => {
    if (drag.current?.id === e.pointerId) drag.current = null;
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!editable || !picked || picked.area.kind !== "rect") return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const step = e.shiftKey ? NUDGE_BIG : NUDGE;
    const dx = e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0;
    const dy = e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0;
    if (dx === 0 && dy === 0) return;
    e.preventDefault();
    emit(moveBounds(picked.area, dx, dy));
  };

  return (
    <div className="rd-canvas-wrap">
      <div className="rd-canvas" ref={stageRef} data-empty={empty ? "true" : "false"}>
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
            data-editable={editable ? "true" : "false"}
            {...(editable
              ? {
                  role: "group",
                  tabIndex: 0,
                  "aria-label": `Area of ${picked.name}. Drag to move, drag a corner to resize, arrow keys nudge.`,
                  onPointerDown: onDown,
                  onPointerMove: onMove,
                  onPointerUp: onUp,
                  onPointerCancel: onUp,
                  onLostPointerCapture: onLost,
                  onDragStart: refuseDragStart,
                  draggable: false,
                  onKeyDown: onKey,
                }
              : {})}
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
            {editable && (
              <>
                <span className="handle nw" aria-hidden="true" />
                <span className="handle ne" aria-hidden="true" />
                <span className="handle sw" aria-hidden="true" />
                <span className="handle se" aria-hidden="true" />
                {/* The edges: nothing drawn (the knobs are the mark), a 24px hit span each with
                    the resize cursor, so the outer half of the hit-test's reach lands too. */}
                <span className="edge n" aria-hidden="true" />
                <span className="edge e" aria-hidden="true" />
                <span className="edge s" aria-hidden="true" />
                <span className="edge w" aria-hidden="true" />
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
