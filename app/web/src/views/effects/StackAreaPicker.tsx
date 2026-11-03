"use client";

// The area picker (D95.5.6): a modal on the palette's scrim, the desk drawn at 320 × 200, the
// four ways to say where a layer paints, and the rectangle drawn by dragging. The desk zooms
// with the wheel and pans with the right button (or Alt drag) through the same view transform
// the Layout page uses (D95.2.6), so a small part can be drawn on precisely.

import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Check, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/Select";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { LayerArea } from "@/proto/LayerArea";
import type { Layout } from "@/proto/Layout";
import type { StackLayer } from "@/proto/StackLayer";
import { cancelPress } from "@/lib/pressDrag";
import {
  areaIsOnDesk,
  areaReadout,
  CANVAS_H,
  CANVAS_W,
  pointerToCanvas,
  rectFromDrag,
  type Bounds,
  type PickerView,
} from "@/views/effects/stackDraft";

type Mode = "canvas" | "device" | "zone" | "rect";

/** What Tab reaches inside the picker, in document order. */
const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

const MODES: { value: Mode; label: string }[] = [
  { value: "canvas", label: "Whole canvas" },
  { value: "device", label: "Device" },
  { value: "zone", label: "Zone" },
  { value: "rect", label: "Rectangle" },
];

function modeOf(area: LayerArea): Mode {
  return area.kind;
}

export function StackAreaPicker({
  layer,
  layout,
  devices,
  onUse,
  onClose,
}: {
  layer: StackLayer;
  layout: Layout | null;
  devices: DeviceInfo[] | null;
  onUse: (area: LayerArea) => void;
  onClose: () => void;
}) {
  const [mode, setMode] = useState<Mode>(modeOf(layer.area));
  const [view, setView] = useState<PickerView>({ zoom: 1, panX: 0, panY: 0 });
  const [rect, setRect] = useState<Bounds | null>(
    layer.area.kind === "rect" ? layer.area : null,
  );
  const initialDevice = layer.area.kind === "device" || layer.area.kind === "zone" ? layer.area.device : undefined;
  const [device, setDevice] = useState<string>(
    initialDevice ?? layout?.zones.find((zone) => zone.visible)?.device_id ?? "",
  );
  const [zoneIndex, setZoneIndex] = useState<number>(
    layer.area.kind === "zone" ? layer.area.zone : layout?.zones.find((zone) => zone.visible)?.zone_index ?? 0,
  );
  const svgRef = useRef<SVGSVGElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ kind: "draw"; from: { x: number; y: number } } | { kind: "pan"; clientX: number; clientY: number; panX: number; panY: number } | null>(null);

  // The dialog takes focus once, when it opens, and keeps the keyboard inside it: Tab wraps from
  // the last control to the first, and comes back in when focus is outside. The effect depends on
  // nothing that changes while the picker is open, so a re-render of the builder (its status poll
  // is one a second) never takes focus back from a control inside it.
  const closeRef = useRef(onClose);
  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);
  useEffect(() => {
    dialogRef.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const dialog = dialogRef.current;
      if (!dialog) return;
      const items = Array.from(dialog.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (items.length === 0) {
        event.preventDefault();
        dialog.focus();
        return;
      }
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const active = document.activeElement;
      if (!dialog.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && (active === first || active === dialog)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (active === last || active === dialog)) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  const deviceOptions = useMemo(() => {
    const ids: string[] = [];
    for (const zone of layout?.zones ?? []) {
      if (zone.visible && !ids.includes(zone.device_id)) ids.push(zone.device_id);
    }
    return ids.map((id) => ({
      id,
      name: devices?.find((entry) => entry.id === id)?.name ?? id,
    }));
  }, [layout, devices]);

  const zoneOptions = useMemo(
    () =>
      (layout?.zones ?? [])
        .filter((zone) => zone.visible && zone.device_id === device)
        .map((zone) => ({
          zone: zone.zone_index,
          label: zone.label ?? `Zone ${zone.zone_index + 1}`,
        })),
    [layout, device],
  );

  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const box = svg.getBoundingClientRect();
      const at = pointerToCanvas(event.clientX, event.clientY, box, view);
      const factor = event.deltaY < 0 ? 1.18 : 1 / 1.18;
      const zoom = Math.max(1, Math.min(8, view.zoom * factor));
      if (zoom === 1) {
        setView({ zoom: 1, panX: 0, panY: 0 });
        return;
      }
      const base = box.width / CANVAS_W;
      const leastX = -(CANVAS_W - CANVAS_W / zoom);
      const leastY = -(CANVAS_H - CANVAS_H / zoom);
      const panX = (event.clientX - box.left) / (base * zoom) - at.x;
      const panY = (event.clientY - box.top) / (base * zoom) - at.y;
      setView({
        zoom,
        panX: Math.min(0, Math.max(leastX, panX)),
        panY: Math.min(0, Math.max(leastY, panY)),
      });
    };
    svg.addEventListener("wheel", onWheel, { passive: false });
    return () => svg.removeEventListener("wheel", onWheel);
  }, [view]);

  const pan = (event: ReactPointerEvent<SVGSVGElement>): boolean => {
    if (event.button !== 1 && event.button !== 2 && !event.altKey) return false;
    drag.current = { kind: "pan", clientX: event.clientX, clientY: event.clientY, panX: view.panX, panY: view.panY };
    event.currentTarget.setPointerCapture(event.pointerId);
    return true;
  };

  const down = (event: ReactPointerEvent<SVGSVGElement>) => {
    // A press on the desk never starts a native drag or a text selection (#105).
    cancelPress(event);
    if (pan(event)) return;
    if (mode !== "rect") return;
    const box = event.currentTarget.getBoundingClientRect();
    const from = pointerToCanvas(event.clientX, event.clientY, box, view);
    drag.current = { kind: "draw", from };
    setRect(rectFromDrag(from, from));
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const move = (event: ReactPointerEvent<SVGSVGElement>) => {
    const state = drag.current;
    if (!state) return;
    const box = event.currentTarget.getBoundingClientRect();
    if (state.kind === "draw") {
      const to = pointerToCanvas(event.clientX, event.clientY, box, view);
      setRect(rectFromDrag(state.from, to));
      return;
    }
    const base = box.width / CANVAS_W;
    const k = base * view.zoom || 1;
    const leastX = -(CANVAS_W - CANVAS_W / view.zoom);
    const leastY = -(CANVAS_H - CANVAS_H / view.zoom);
    setView({
      zoom: view.zoom,
      panX: Math.min(0, Math.max(leastX, state.panX + (event.clientX - state.clientX) / k)),
      panY: Math.min(0, Math.max(leastY, state.panY + (event.clientY - state.clientY) / k)),
    });
  };

  const up = (event: ReactPointerEvent<SVGSVGElement>) => {
    drag.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const chosen: LayerArea | null = useMemo(() => {
    if (mode === "canvas") return { kind: "canvas" };
    if (mode === "device") return device ? { kind: "device", device } : null;
    if (mode === "zone") return device ? { kind: "zone", device, zone: zoneIndex } : null;
    return rect ? { kind: "rect", ...rect } : null;
  }, [mode, device, zoneIndex, rect]);

  const readout = chosen ? areaReadout(chosen, layout, devices) : null;
  const litDevice = mode === "device" ? device : mode === "zone" ? device : null;
  const litZone = mode === "zone" ? zoneIndex : null;

  const pickZone = (deviceId: string, zone: number) => {
    if (mode !== "device" && mode !== "zone") return;
    setDevice(deviceId);
    if (mode === "zone") setZoneIndex(zone);
  };

  return (
    <div className="rd-scrim" role="presentation" onPointerDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="rd-picker" role="dialog" aria-modal="true" aria-label={`Area for ${layer.name}`} ref={dialogRef} tabIndex={-1}>
        <div className="rd-picker-head">
          <div>
            <h2>Area for {layer.name}</h2>
            <p>Drag on the desk to draw a rectangle, or pick a part.</p>
          </div>
          <IconButton label="Close the area picker" icon={<X size={16} />} onClick={onClose} />
        </div>

        <Segmented<Mode> label="Where the layer paints" value={mode} options={MODES} onChange={setMode} />

        {(mode === "device" || mode === "zone") && (
          <div className="rd-picker-choice">
            <div className="layer-field">
              <span>Device</span>
              <Select
                label="Device"
                value={device}
                options={
                  deviceOptions.length > 0
                    ? deviceOptions.map((option) => ({ value: option.id, label: option.name }))
                    : [{ value: "", label: "No device on the desk" }]
                }
                onChange={(next) => {
                  setDevice(next);
                  const first = (layout?.zones ?? []).find((zone) => zone.visible && zone.device_id === next);
                  setZoneIndex(first?.zone_index ?? 0);
                }}
              />
            </div>
            {mode === "zone" && (
              <div className="layer-field">
                <span>Zone</span>
                <Select
                  label="Zone"
                  value={String(zoneIndex)}
                  options={
                    zoneOptions.length > 0
                      ? zoneOptions.map((option) => ({ value: String(option.zone), label: option.label }))
                      : [{ value: "0", label: "No zone on this device" }]
                  }
                  onChange={(next) => setZoneIndex(Number(next))}
                />
              </div>
            )}
          </div>
        )}

        <svg
          ref={svgRef}
          className="rd-desk"
          viewBox={`0 0 ${CANVAS_W} ${CANVAS_H}`}
          role="img"
          aria-label="The desk"
          onPointerDown={down}
          onPointerMove={move}
          onPointerUp={up}
          onPointerCancel={up}
          onContextMenu={(event) => event.preventDefault()}
        >
          <g transform={`scale(${view.zoom}) translate(${view.panX} ${view.panY})`}>
            <line className="desk-line" x1={0} y1={190} x2={CANVAS_W} y2={190} />
            {(layout?.landmarks ?? [])
              .filter((landmark) => landmark.visible)
              .map((landmark, i) => (
                <rect
                  key={`landmark-${i}`}
                  className="case"
                  x={landmark.x}
                  y={landmark.y}
                  width={landmark.width}
                  height={landmark.height}
                  rx={3}
                  transform={landmark.rotation_deg ? `rotate(${landmark.rotation_deg} ${landmark.x + landmark.width / 2} ${landmark.y + landmark.height / 2})` : undefined}
                />
              ))}
            {(layout?.zones ?? [])
              .filter((zone) => zone.visible)
              .map((zone, i) => {
                const lit =
                  litDevice !== null &&
                  zone.device_id === litDevice &&
                  (litZone === null || zone.zone_index === litZone);
                const kind = zone.part?.kind;
                const cx = zone.x + zone.width / 2;
                const cy = zone.y + zone.height / 2;
                const radius = Math.max(1, Math.min(zone.width, zone.height) / 2 - 1.5);
                return (
                  <g
                    key={`zone-${i}`}
                    transform={zone.rotation_deg ? `rotate(${zone.rotation_deg} ${cx} ${cy})` : undefined}
                    onPointerDown={(event) => {
                      if (mode === "device" || mode === "zone") {
                        event.stopPropagation();
                        pickZone(zone.device_id, zone.zone_index);
                      }
                    }}
                  >
                    <rect
                      className="part"
                      data-lit={lit ? "true" : "false"}
                      x={zone.x}
                      y={zone.y}
                      width={zone.width}
                      height={zone.height}
                      rx={2}
                    />
                    {kind === "fan" && (
                      <>
                        <circle className="part-line" cx={cx} cy={cy} r={radius * 0.82} />
                        <circle className="part-line" cx={cx} cy={cy} r={radius * 0.2} />
                      </>
                    )}
                    {kind === "keyboard" && (
                      <>
                        {[0.2, 0.35, 0.5, 0.65, 0.8].map((at) => (
                          <line
                            key={at}
                            className="part-line"
                            x1={zone.x + zone.width * at}
                            y1={zone.y + zone.height * 0.2}
                            x2={zone.x + zone.width * at}
                            y2={zone.y + zone.height * 0.8}
                          />
                        ))}
                      </>
                    )}
                    {kind === "mouse" && <ellipse className="part-line" cx={cx} cy={cy} rx={zone.width * 0.3} ry={zone.height * 0.36} />}
                    {(kind === "ram" || kind === "gpu" || kind === "strip") && (
                      <line className="part-line" x1={zone.x + zone.width * 0.15} y1={cy} x2={zone.x + zone.width * 0.85} y2={cy} />
                    )}
                    {kind === "microphone" && <circle className="part-line" cx={cx} cy={zone.y + zone.height * 0.3} r={zone.width * 0.24} />}
                  </g>
                );
              })}
            {mode === "rect" && rect && (
              <g>
                <rect className="marquee-tint" x={rect.x} y={rect.y} width={rect.width} height={rect.height} />
                <rect className="marquee" x={rect.x} y={rect.y} width={rect.width} height={rect.height} />
                {([
                  [rect.x, rect.y],
                  [rect.x + rect.width, rect.y],
                  [rect.x, rect.y + rect.height],
                  [rect.x + rect.width, rect.y + rect.height],
                ] as const).map(([x, y], i) => (
                  <rect key={i} className="marquee-handle" x={x - 2} y={y - 2} width={4} height={4} />
                ))}
                <g transform={`translate(${rect.x} ${Math.max(0, rect.y - 12)})`}>
                  <rect
                    className="size-tag"
                    width={Math.max(30, `${Math.round(rect.width)} × ${Math.round(rect.height)}`.length * 4.8 + 6)}
                    height={10}
                    rx={2}
                  />
                  <text className="size-text" x={4} y={7.4}>
                    {Math.round(rect.width)} × {Math.round(rect.height)}
                  </text>
                </g>
              </g>
            )}
          </g>
        </svg>

        <div className="rd-picker-readout" aria-live="polite">
          {readout ? (
            <>
              <span>{readout.title}</span>
              {readout.detail && <span className="mute">{readout.detail}</span>}
            </>
          ) : (
            <span className="mute">
              {mode === "rect" ? "Drag on the desk to draw a rectangle." : "Pick a part on the desk."}
            </span>
          )}
        </div>

        <div className="rd-picker-foot">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            icon={<Check size={15} />}
            disabled={!chosen || (chosen.kind !== "canvas" && chosen.kind !== "rect" && !areaIsOnDesk(chosen, layout))}
            onClick={() => chosen && onUse(chosen)}
          >
            Use this area
          </Button>
        </div>
      </div>
    </div>
  );
}
