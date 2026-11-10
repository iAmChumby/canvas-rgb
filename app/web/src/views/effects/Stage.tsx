"use client";

// The builder's stage (#257 phase 1): the live picture with every visible layer's frame drawn on
// it. The picked layer's frame is the solid one with handles; every other frame is a faint
// outline, and a press on one picks its layer — the smallest area under the pointer wins, and a
// whole-canvas layer comes last (stackDraft.ts `layerAtPoint`). Tab and Shift+Tab walk the layers
// top first, Escape clears the pick, Delete removes the picked layer and Ctrl+D copies it.
//
// Only a rectangle is dragged or resized here (#104). A whole-canvas, device or zone area follows
// the layout, so turning one into a rectangle is the "Make rectangle" key's job, by name.
// Frames are drawn as DOM over a canvas that the 15 fps feed paints straight into, so a frame
// never re-renders the picture.

import { useEffect, useMemo, useRef, type KeyboardEvent, type PointerEvent } from "react";
import { ArrowClockwise, ArrowCounterClockwise } from "@phosphor-icons/react";
import { cancelPress, refuseDragStart } from "@/lib/pressDrag";
import { hitTest, type HandleDef, type HitPart, type PartRef, type PointerKind } from "@/lib/hitTest";
import { Chip } from "@/components/Chip";
import { IconButton } from "@/components/Button";
import { Toggle } from "@/components/Toggle";
import { onStackFrame, setStackPreviewStack, startStackPreview, stopStackPreview } from "@/lib/stackPreview";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { LayerArea } from "@/proto/LayerArea";
import type { Layout } from "@/proto/Layout";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";
import {
  areaAsRect,
  areaName,
  CANVAS_H,
  CANVAS_W,
  cycleLayer,
  layerAtPoint,
  moveBounds,
  pointerToCanvas,
  resizeBounds,
  stageFrames,
  type Bounds,
} from "@/views/effects/stackDraft";

/** The one part the hit-test sees: the picked layer's rectangle. */
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
  | { mode: "move"; id: number; from: { x: number; y: number }; start: Bounds; run: string }
  | { mode: "resize"; id: number; from: { x: number; y: number }; start: Bounds; handle: HandleDef; run: string };

/**
 * The stage's own keys, in its panel head (#257): undo and redo for the draft, the Parts switch
 * that draws the desk's outlines under the frames, and the picture's own size and rate.
 */
export function StageTools({
  canUndo,
  canRedo,
  onUndo,
  onRedo,
  parts,
  onParts,
}: {
  canUndo: boolean;
  canRedo: boolean;
  onUndo: () => void;
  onRedo: () => void;
  parts: boolean;
  onParts: (next: boolean) => void;
}) {
  return (
    <span className="rd-stage-meta">
      <span className="rd-stage-tools">
        <IconButton
          label="Undo the last change"
          size="sm"
          icon={<ArrowCounterClockwise size={16} />}
          disabled={!canUndo}
          onClick={onUndo}
        />
        <IconButton
          label="Redo the change that was undone"
          size="sm"
          icon={<ArrowClockwise size={16} />}
          disabled={!canRedo}
          onClick={onRedo}
        />
      </span>
      <span className="rd-stage-parts">
        <span className="mute">Parts</span>
        <Toggle label="Show the desk's device and zone outlines" checked={parts} onChange={onParts} />
      </span>
      <span className="fig mute">320 × 200 · 15 fps</span>
    </span>
  );
}

export function Stage({
  stack,
  layout,
  devices,
  picked,
  parts,
  onSelect,
  onAreaChange,
  onAreaChangeEnd,
  onDelete,
  onDuplicate,
  onMakeRect,
}: {
  stack: Stack;
  layout: Layout | null;
  devices: DeviceInfo[] | null;
  picked: StackLayer | null;
  /** Show the desk's device and zone outlines under the frames (the Parts switch). */
  parts: boolean;
  onSelect: (id: string | null) => void;
  /** The picked layer's rectangle, dragged or nudged; `run` names the gesture for undo. */
  onAreaChange?: (area: LayerArea, run: string) => void;
  /** The drag is over: the next change is a new undo step. */
  onAreaChangeEnd?: () => void;
  onDelete?: (id: string) => void;
  onDuplicate?: (id: string) => void;
  onMakeRect?: (id: string) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  /** One number per gesture, so a drag's changes all name the same undo run. */
  const runs = useRef(0);
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

  const frames = useMemo(() => stageFrames(stack.layers, layout), [stack.layers, layout]);
  // Only a rectangle is dragged here. A whole-canvas, device or zone area follows the layout;
  // turning it into a rectangle is a choice the "Make rectangle" key asks for by name, so a
  // stray drag never does it.
  const editable = !!onAreaChange && picked?.area.kind === "rect";
  /** The rectangle the picked area covers, when the chip may offer to make it one. */
  const asRect = picked ? areaAsRect(picked.area, layout) : null;

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

  const emit = (area: Bounds, run: string) => onAreaChange?.({ kind: "rect", ...area }, run);

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (!editable || !picked || picked.area.kind !== "rect" || e.button > 0) return;
    const start = { x: picked.area.x, y: picked.area.y, width: picked.area.width, height: picked.area.height };
    const hit = hitAt(e, start);
    if (!hit) return;
    const { target, point } = hit;
    const run = `area:${picked.id}:${++runs.current}`;
    let next: Drag | null = null;
    if (target.kind === "handle" && target.handle !== "rotate") {
      next = { mode: "resize", id: e.pointerId, from: point, start, handle: target.handle, run };
    } else if (target.kind === "part") {
      next = { mode: "move", id: e.pointerId, from: point, start, run };
    }
    if (!next) return;
    // The press is ours, and not a second pick made by the stage underneath: no native drag, text
    // selection or focus theft (lib/pressDrag.ts).
    e.stopPropagation();
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
      onAreaChangeEnd?.();
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
      d.run,
    );
  };

  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    drag.current = null;
    // A cancelled press (a call, a system gesture) puts the area back where it started.
    if (e.type === "pointercancel") emit(d.start, d.run);
    onAreaChangeEnd?.();
  };

  // Capture lost without a pointerup (the window lost focus, the element went away): the drag
  // ends where it is.
  const onLost = (e: PointerEvent<HTMLDivElement>) => {
    if (drag.current?.id === e.pointerId) {
      drag.current = null;
      onAreaChangeEnd?.();
    }
  };

  /** A press on the picture itself: pick the smallest frame under it, or clear the pick. */
  const onStageDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button > 0) return;
    const box = stageRef.current?.getBoundingClientRect();
    if (!box) return;
    const point = pointerToCanvas(e.clientX, e.clientY, box, VIEW);
    const hit = layerAtPoint(stack.layers, layout, point);
    cancelPress(e, { focus: true });
    if (hit !== (picked?.id ?? null)) onSelect(hit);
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Tab") {
      // Tab walks the layers while one is picked. Escape clears the pick, so Tab then leaves
      // the stage for the next control.
      if (!picked) return;
      e.preventDefault();
      onSelect(cycleLayer(stack.layers, picked.id, e.shiftKey ? -1 : 1));
      return;
    }
    if (e.key === "Escape") {
      if (!picked) return;
      e.preventDefault();
      onSelect(null);
      return;
    }
    if ((e.key === "Delete" || e.key === "Backspace") && picked) {
      e.preventDefault();
      onDelete?.(picked.id);
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d" && picked) {
      e.preventDefault();
      onDuplicate?.(picked.id);
    }
  };

  const onFrameKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!editable || !picked || picked.area.kind !== "rect") return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const step = e.shiftKey ? NUDGE_BIG : NUDGE;
    const dx = e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0;
    const dy = e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0;
    if (dx === 0 && dy === 0) return;
    e.preventDefault();
    // A nudge is a step of its own: each arrow press is one undo.
    const run = `nudge:${picked.id}:${++runs.current}`;
    emit(moveBounds(picked.area, dx, dy), run);
    onAreaChangeEnd?.();
  };

  return (
    <div className="rd-canvas-wrap">
      <div
        className="rd-canvas"
        ref={stageRef}
        data-empty={empty ? "true" : "false"}
        tabIndex={0}
        role="group"
        aria-label="The stage. Tab and Shift+Tab pick a layer, Escape clears the pick."
        onPointerDown={onStageDown}
        onKeyDown={onKey}
        onDragStart={refuseDragStart}
        draggable={false}
      >
        {!empty && <canvas ref={canvasRef} width={CANVAS_W} height={CANVAS_H} aria-label="The stack's preview" />}
        {empty && (
          <div className="rd-canvas-empty">
            <strong>Nothing to show yet</strong>
            <span>Add a layer and its picture appears here.</span>
          </div>
        )}
        {!empty && parts && layout && (
          <div className="rd-parts" aria-hidden="true">
            {layout.zones
              .filter((zone) => zone.visible)
              .map((zone) => (
                <span
                  key={`${zone.device_id}:${zone.zone_index}:${zone.part?.kind ?? ""}`}
                  className="rd-part-outline"
                  style={{
                    left: `${(zone.x / CANVAS_W) * 100}%`,
                    top: `${(zone.y / CANVAS_H) * 100}%`,
                    width: `${(zone.width / CANVAS_W) * 100}%`,
                    height: `${(zone.height / CANVAS_H) * 100}%`,
                  }}
                >
                  {zone.label && <em>{zone.label}</em>}
                </span>
              ))}
          </div>
        )}
        {!empty &&
          frames.map((frame) => {
            const layer = stack.layers[frame.index];
            if (!layer) return null;
            const selected = layer.id === picked?.id;
            const draggable = selected && editable;
            return (
              <div
                key={frame.id}
                className={selected ? "rd-layer-frame rd-area-frame" : "rd-layer-frame"}
                data-layer-id={frame.id}
                data-selected={selected ? "true" : "false"}
                {...(selected ? { "data-editable": draggable ? "true" : "false" } : {})}
                {...(draggable
                  ? {
                      role: "group",
                      tabIndex: 0,
                      "aria-label": `Area of ${layer.name}. Drag to move, drag a corner to resize, arrow keys nudge.`,
                      onPointerDown: onDown,
                      onPointerMove: onMove,
                      onPointerUp: onUp,
                      onPointerCancel: onUp,
                      onLostPointerCapture: onLost,
                      onDragStart: refuseDragStart,
                      draggable: false,
                      onKeyDown: onFrameKey,
                    }
                  : {})}
                style={{
                  left: `${(frame.bounds.x / CANVAS_W) * 100}%`,
                  top: `${(frame.bounds.y / CANVAS_H) * 100}%`,
                  width: `${(frame.bounds.width / CANVAS_W) * 100}%`,
                  height: `${(frame.bounds.height / CANVAS_H) * 100}%`,
                }}
              >
                {selected && (
                  <span className="rd-area-label">
                    {layer.name} · {areaName(layer.area, layout, devices)}
                  </span>
                )}
                {draggable && (
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
            );
          })}
      </div>
      {!empty && picked && asRect && picked.area.kind !== "rect" && (
        <span className="rd-stage-float">
          <Chip
            label={`Make ${picked.name} a rectangle`}
            title={`Make ${picked.name} a rectangle: its area becomes the box it covers now, and it can then be moved and resized.`}
            onClick={() => onMakeRect?.(picked.id)}
          >
            Make rectangle
          </Chip>
        </span>
      )}
    </div>
  );
}
