"use client";

import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent,
  type PointerEvent,
} from "react";
import { onFrame, PREVIEW_H, PREVIEW_W, type Frame } from "@/lib/preview";
import { findZoneInfo, ledPositions, type Point } from "@/lib/zones";
import { clamp } from "@/lib/format";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { PlacedZone } from "@/proto/PlacedZone";

const CANVAS_W = 320;
const CANVAS_H = 200;
const ACCENTS = ["--a1", "--a2", "--a3", "--a4", "--a5"] as const;

type DragKind = "move" | "resize" | "rotate";
interface Drag {
  kind: DragKind;
  index: number;
  start: Point;
  orig: PlacedZone;
}

export interface StageProps {
  zones: PlacedZone[];
  devices: DeviceInfo[] | null;
  selected: number | null;
  onSelect: (index: number | null) => void;
  onChange: (index: number, patch: Partial<PlacedZone>) => void;
  onDrop: (deviceId: string, zoneIndex: number, x: number, y: number) => void;
  dropPayloadType: string;
}

/**
 * The live preview with every placed zone drawn over it. Zones move, resize and rotate by pointer
 * (or by arrow keys, with Shift for ten units). LEDs are dots sampled from the frame each time it
 * arrives, painted on a canvas so the page does not re-render 15 times a second.
 */
export function PreviewStage({
  zones,
  devices,
  selected,
  onSelect,
  onChange,
  onDrop,
  dropPayloadType,
}: StageProps) {
  const stageRef = useRef<HTMLDivElement>(null);
  const previewRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  const imageRef = useRef<ImageData | null>(null);
  const dragRef = useRef<Drag | null>(null);
  const [hasFrame, setHasFrame] = useState(false);

  // LED positions for every visible, resolvable zone, with the zone's device accent index.
  const leds = useMemo(() => {
    const out: { x: number; y: number; accent: number }[] = [];
    zones.forEach((z) => {
      if (!z.visible) return;
      const found = findZoneInfo(devices, z.device_id, z.zone_index);
      if (!found) return;
      const accent = (devices?.findIndex((d) => d.id === z.device_id) ?? 0) % ACCENTS.length;
      for (const p of ledPositions(z, found.zone)) out.push({ x: p.x, y: p.y, accent });
    });
    return out;
  }, [zones, devices]);

  const ledsRef = useRef(leds);
  ledsRef.current = leds;

  // Paint each arriving frame: the preview canvas, then the LED dots over it.
  useEffect(() => {
    const draw = (frame: Frame) => {
      const pv = previewRef.current;
      const ov = overlayRef.current;
      if (!pv || !ov) return;
      if (frame.width !== PREVIEW_W || frame.height !== PREVIEW_H) return;
      const ctx = pv.getContext("2d");
      if (!ctx) return;
      if (!imageRef.current) imageRef.current = ctx.createImageData(PREVIEW_W, PREVIEW_H);
      const img = imageRef.current;
      const px = img.data;
      const src = frame.rgb;
      for (let i = 0, o = 0; i < src.length; i += 3, o += 4) {
        px[o] = src[i] ?? 0;
        px[o + 1] = src[i + 1] ?? 0;
        px[o + 2] = src[i + 2] ?? 0;
        px[o + 3] = 255;
      }
      ctx.putImageData(img, 0, 0);

      const octx = ov.getContext("2d");
      if (!octx) return;
      const sx = ov.width / CANVAS_W;
      const sy = ov.height / CANVAS_H;
      octx.setTransform(1, 0, 0, 1, 0, 0);
      octx.clearRect(0, 0, ov.width, ov.height);
      // Dots scale with the stage: small enough that a 120-LED strip still reads as a strip.
      const r = Math.min(3.2, Math.max(2.2, ov.width / 360));
      for (const led of ledsRef.current) {
        const ix = clamp(Math.floor(led.x), 0, CANVAS_W - 1);
        const iy = clamp(Math.floor(led.y), 0, CANVAS_H - 1);
        const o = (iy * frame.width + ix) * 3;
        const rr = src[o] ?? 0;
        const gg = src[o + 1] ?? 0;
        const bb = src[o + 2] ?? 0;
        octx.fillStyle = `rgb(${rr} ${gg} ${bb})`;
        octx.beginPath();
        octx.arc(led.x * sx, led.y * sy, r, 0, Math.PI * 2);
        octx.fill();
      }
    };
    const off = onFrame((frame) => {
      setHasFrame(true);
      draw(frame);
    });
    return off;
  }, []);

  // Keep the overlay's bitmap the same size as its CSS box, so dots stay round.
  useEffect(() => {
    const stage = stageRef.current;
    const ov = overlayRef.current;
    if (!stage || !ov) return;
    const size = () => {
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      const w = Math.max(1, Math.round(stage.clientWidth * dpr));
      const h = Math.max(1, Math.round(stage.clientHeight * dpr));
      if (ov.width !== w) ov.width = w;
      if (ov.height !== h) ov.height = h;
    };
    size();
    const ro = new ResizeObserver(size);
    ro.observe(stage);
    return () => ro.disconnect();
  }, []);

  const toLogical = (e: { clientX: number; clientY: number }): Point => {
    const box = stageRef.current?.getBoundingClientRect();
    if (!box || box.width === 0) return { x: 0, y: 0 };
    return {
      x: ((e.clientX - box.left) / box.width) * CANVAS_W,
      y: ((e.clientY - box.top) / box.height) * CANVAS_H,
    };
  };

  const beginDrag = (kind: DragKind, index: number, e: PointerEvent<HTMLElement>) => {
    e.stopPropagation();
    e.preventDefault();
    const z = zones[index];
    if (!z) return;
    onSelect(index);
    dragRef.current = { kind, index, start: toLogical(e), orig: { ...z } };
    stageRef.current?.setPointerCapture(e.pointerId);
  };

  const onStageMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = dragRef.current;
    if (!d) return;
    const p = toLogical(e);
    const o = d.orig;
    const dx = p.x - d.start.x;
    const dy = p.y - d.start.y;
    if (d.kind === "move") {
      onChange(d.index, {
        x: clamp(o.x + dx, 0, CANVAS_W - o.width),
        y: clamp(o.y + dy, 0, CANVAS_H - o.height),
      });
    } else if (d.kind === "resize") {
      onChange(d.index, {
        width: clamp(o.width + dx, 2, CANVAS_W - o.x),
        height: clamp(o.height + dy, 2, CANVAS_H - o.y),
      });
    } else {
      const cx = o.x + o.width / 2;
      const cy = o.y + o.height / 2;
      let deg = (Math.atan2(p.y - cy, p.x - cx) * 180) / Math.PI + 90;
      deg = ((deg % 360) + 360) % 360;
      if (e.shiftKey) deg = Math.round(deg / 15) * 15;
      onChange(d.index, { rotation_deg: Math.round(deg * 10) / 10 });
    }
  };

  const endDrag = (e: PointerEvent<HTMLDivElement>) => {
    if (dragRef.current) {
      dragRef.current = null;
      if (stageRef.current?.hasPointerCapture(e.pointerId)) {
        stageRef.current.releasePointerCapture(e.pointerId);
      }
    }
  };

  const onRectKey = (index: number, e: KeyboardEvent<HTMLDivElement>) => {
    const z = zones[index];
    if (!z) return;
    const step = e.shiftKey ? 10 : 1;
    let patch: Partial<PlacedZone> | null = null;
    if (e.key === "ArrowLeft") patch = { x: clamp(z.x - step, 0, CANVAS_W - z.width) };
    if (e.key === "ArrowRight") patch = { x: clamp(z.x + step, 0, CANVAS_W - z.width) };
    if (e.key === "ArrowUp") patch = { y: clamp(z.y - step, 0, CANVAS_H - z.height) };
    if (e.key === "ArrowDown") patch = { y: clamp(z.y + step, 0, CANVAS_H - z.height) };
    if (patch) {
      e.preventDefault();
      onChange(index, patch);
    }
  };

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    if (Array.from(e.dataTransfer.types).includes(dropPayloadType)) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
    }
  };

  const onDropHere = (e: DragEvent<HTMLDivElement>) => {
    const raw = e.dataTransfer.getData(dropPayloadType);
    if (!raw) return;
    e.preventDefault();
    try {
      const payload = JSON.parse(raw) as { deviceId: string; zoneIndex: number };
      const p = toLogical(e);
      onDrop(payload.deviceId, payload.zoneIndex, p.x, p.y);
    } catch {
      // A drag from somewhere else carries nothing we understand; ignore it.
    }
  };

  return (
    <div
      ref={stageRef}
      className="stage"
      onPointerMove={onStageMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onDragOver={onDragOver}
      onDrop={onDropHere}
      onPointerDown={() => onSelect(null)}
    >
      <canvas ref={previewRef} className="stage-canvas" width={PREVIEW_W} height={PREVIEW_H} aria-hidden="true" />
      <canvas ref={overlayRef} className="stage-leds" aria-hidden="true" />
      {!hasFrame && <div className="stage-empty">Waiting for the first frame from the daemon</div>}

      {zones.map((z, i) => {
        const found = findZoneInfo(devices, z.device_id, z.zone_index);
        const accent = (devices?.findIndex((d) => d.id === z.device_id) ?? 0) % ACCENTS.length;
        const isSel = selected === i;
        const style = {
          left: `${(z.x / CANVAS_W) * 100}%`,
          top: `${(z.y / CANVAS_H) * 100}%`,
          width: `${(z.width / CANVAS_W) * 100}%`,
          height: `${(z.height / CANVAS_H) * 100}%`,
          transform: `rotate(${z.rotation_deg}deg)`,
          "--zc": `var(${ACCENTS[accent]})`,
        } as CSSProperties;
        const name = found ? found.zone.name : "missing zone";
        const count = found ? found.zone.led_count : 0;
        return (
          <div
            key={`${z.device_id}#${z.zone_index}`}
            className="zrect"
            role="button"
            tabIndex={0}
            aria-pressed={isSel}
            aria-label={`${found?.device.name ?? z.device_id} ${name}, ${count} LEDs${z.visible ? "" : ", hidden"}`}
            data-hidden={!z.visible}
            style={style}
            onPointerDown={(e) => beginDrag("move", i, e)}
            onKeyDown={(e) => onRectKey(i, e)}
            onFocus={() => onSelect(i)}
          >
            <span className="zlabel" data-edge={z.x + z.width > CANVAS_W * 0.75 ? "right" : undefined}>
              {name} <span className="fig">{count}</span>
            </span>
            {isSel && (
              <>
                <span className="zrot-stem" aria-hidden="true" />
                <span
                  className="zhandle zhandle-rot"
                  aria-hidden="true"
                  onPointerDown={(e) => beginDrag("rotate", i, e)}
                />
                <span
                  className="zhandle zhandle-se"
                  aria-hidden="true"
                  onPointerDown={(e) => beginDrag("resize", i, e)}
                />
              </>
            )}
          </div>
        );
      })}
    </div>
  );
}
