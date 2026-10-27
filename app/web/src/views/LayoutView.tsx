"use client";

import { useMemo, useState, type DragEvent } from "react";
import { ArrowCounterClockwise, FloppyDisk, Plus, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Panel, Field, StateDot } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { NumberField } from "@/components/NumberField";
import { Toggle } from "@/components/Toggle";
import { RowButton } from "@/components/Chip";
import { PreviewStage } from "@/views/PreviewStage";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { usePreviewStatus } from "@/lib/preview";
import { refreshLayouts, useSnap } from "@/lib/store";
import { defaultRect, findZoneInfo } from "@/lib/zones";
import { clamp } from "@/lib/format";
import type { Layout } from "@/proto/Layout";
import type { PlacedZone } from "@/proto/PlacedZone";
import desk from "./desk.module.css";
import type { RenderScale } from "@/proto/RenderScale";
import type { SamplingMode } from "@/proto/SamplingMode";

const DROP_TYPE = "application/x-canvas-zone";
const CANVAS_W = 320;
const CANVAS_H = 200;

/** The layout without its effect: the effect is owned by the Effects page, not by this draft. */
function withoutEffect(l: Layout): Layout {
  return { ...l, effect: null };
}

/** A short, readable name for a device the daemon does not list any more. */
function shortDeviceId(id: string): string {
  const vid = id.match(/VID_([0-9A-Fa-f]{4})/);
  const pid = id.match(/PID_([0-9A-Fa-f]{4})/);
  if (vid && pid) return `VID ${vid[1]} · PID ${pid[1]}`;
  const hex = id.match(/0x[0-9a-fA-F]+/g)?.pop();
  if (hex) return hex;
  const tail = id.split(/[#:]/).filter(Boolean).pop() ?? id;
  return tail.length > 20 ? `${tail.slice(0, 20)}…` : tail;
}


export function LayoutView() {
  const server = useSnap((s) => s.layouts?.find((l) => l.id === s.activeLayout) ?? null);
  const devices = useSnap((s) => s.devices);
  const effects = useSnap((s) => s.effects);
  const layoutsError = useSnap((s) => s.layoutsError);

  const [draft, setDraft] = useState<Layout | null>(null);
  const [base, setBase] = useState<Layout | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  /** True while a zone is being dragged: the server's layout must not replace the draft then. */
  const [dragging, setDragging] = useState(false);
  /**
   * Where the dragged zone is right now. The canvas reports it once per animation frame, so the
   * inspector follows the pointer without the drag re-rendering the canvas every move.
   */
  const [liveZone, setLiveZone] = useState<{ index: number; zone: PlacedZone } | null>(null);

  const serverKey = server ? JSON.stringify(server) : "";
  const dirty =
    draft !== null && base !== null && JSON.stringify(withoutEffect(draft)) !== JSON.stringify(withoutEffect(base));

  // Take the server's layout whenever it changes and there are no unsaved edits. This runs during
  // render (React's pattern for state that follows a prop), so the page never shows a stale frame.
  // A drag in progress holds it off: the zone would jump back under the pointer.
  const [syncedKey, setSyncedKey] = useState("");
  if (serverKey !== syncedKey && server && !dirty && !dragging) {
    setSyncedKey(serverKey);
    setBase(server);
    setDraft(server);
  }

  const effectName = useMemo(() => {
    const id = server?.effect?.id;
    if (!id) return "no effect";
    return effects?.find((e) => e.id === id)?.name ?? id;
  }, [server, effects]);

  const unplaced = useMemo(() => {
    const placed = new Set((draft?.zones ?? []).map((z) => `${z.device_id}#${z.zone_index}`));
    const out: { deviceId: string; deviceName: string; zoneIndex: number; zoneName: string; ledCount: number; connected: boolean }[] = [];
    for (const d of devices ?? []) {
      for (const z of d.zones) {
        if (placed.has(`${d.id}#${z.index}`)) continue;
        out.push({
          deviceId: d.id,
          deviceName: d.name,
          zoneIndex: z.index,
          zoneName: z.name,
          ledCount: z.led_count,
          connected: d.connected,
        });
      }
    }
    return out;
  }, [draft, devices]);

  const zones = draft?.zones ?? [];
  const selectedZone = selected !== null ? zones[selected] ?? null : null;
  const selectedInfo = selectedZone ? findZoneInfo(devices, selectedZone.device_id, selectedZone.zone_index) : null;
  const preview = usePreviewStatus();
  // While a zone is being dragged the inspector shows where it is, not where it was.
  const shownZone = liveZone && liveZone.index === selected ? liveZone.zone : selectedZone;

  const patchDraft = (fn: (l: Layout) => Layout) => {
    setDraft((prev) => (prev ? fn(prev) : prev));
  };

  const changeZone = (index: number, patch: Partial<PlacedZone>) => {
    patchDraft((l) => ({
      ...l,
      zones: l.zones.map((z, i) => (i === index ? { ...z, ...patch } : z)),
    }));
  };

  const place = (deviceId: string, zoneIndex: number, at?: { x: number; y: number }) => {
    const info = findZoneInfo(devices, deviceId, zoneIndex);
    if (!info || !draft) return;
    const r = defaultRect(info.zone);
    const x = clamp((at?.x ?? CANVAS_W / 2) - r.width / 2, 0, CANVAS_W - r.width);
    const y = clamp((at?.y ?? CANVAS_H / 2) - r.height / 2, 0, CANVAS_H - r.height);
    const placedZone: PlacedZone = {
      device_id: deviceId,
      zone_index: zoneIndex,
      x,
      y,
      width: r.width,
      height: r.height,
      rotation_deg: 0,
      reversed: false,
      visible: true,
    };
    patchDraft((l) => ({ ...l, zones: [...l.zones, placedZone] }));
    setSelected(draft.zones.length);
  };

  const unplace = (index: number) => {
    patchDraft((l) => ({ ...l, zones: l.zones.filter((_, i) => i !== index) }));
    setSelected(null);
  };

  const revert = () => {
    if (base) setDraft(base);
    setSaveError(null);
  };

  const save = async () => {
    if (!draft || !server) return;
    setSaving(true);
    setSaveError(null);
    try {
      const body: Layout = { ...draft, effect: server.effect };
      await api.putLayout(draft.id, body);
      setBase(body);
      setDraft(body);
      await refreshLayouts();
    } catch (e) {
      setSaveError(messageOf(e));
    } finally {
      setSaving(false);
    }
  };

  const onDropUnplaced = (deviceId: string, zoneIndex: number, x: number, y: number) => {
    place(deviceId, zoneIndex, { x, y });
  };

  const dragStart = (deviceId: string, zoneIndex: number, e: DragEvent<HTMLElement>) => {
    e.dataTransfer.setData(DROP_TYPE, JSON.stringify({ deviceId, zoneIndex }));
    e.dataTransfer.effectAllowed = "copy";
  };

  if (!server || !draft) {
    return (
      <div className="stack">
        <div className="page-head">
          <h1>Layout</h1>
        </div>
        <Panel>
          <p className="mute">
            {layoutsError ? `The layout could not be read: ${layoutsError}` : "Reading the layout from the daemon."}
          </p>
        </Panel>
      </div>
    );
  }

  return (
    <div className="stack">
      <div className="page-head">
        <h1>{draft.name}</h1>
        <p>
          The desk canvas: every placed zone over the live preview of{" "}
          <span style={{ color: "var(--ink)" }}>{effectName}</span>.
        </p>
      </div>

      <div className="layout-grid">
        <div className="layout-stage-col stack">
          <PreviewStage
            zones={zones}
            devices={devices}
            selected={selected}
            onSelect={setSelected}
            onChange={changeZone}
            onDrop={onDropUnplaced}
            onDraggingChange={(d) => {
              setDragging(d);
              if (!d) setLiveZone(null);
            }}
            onLiveChange={(index, zone) => setLiveZone({ index, zone })}
            layoutId={draft.id}
            dropPayloadType={DROP_TYPE}
          />
          <div className="stage-meta">
            <span className="dropzone-hint">
              Drag a zone to move it, its corner to resize, its top knob to rotate (Shift snaps to 15°). Arrow keys nudge.
            </span>
            <span className="row-wrap" style={{ alignItems: "center", gap: "var(--s3)" }}>
              <StateDot tone={preview.state === "live" ? "ok" : preview.state === "refused" ? "down" : "work"}>
                {preview.state === "live" ? `preview ${preview.fps.toFixed(1)} fps` : `preview ${preview.state}`}
              </StateDot>
              <span className="mute fig">
                {zones.filter((z) => z.visible).length} of {zones.length} placed zones visible
              </span>
            </span>
          </div>
          {preview.state === "refused" && preview.error && (
            <p className="fig" style={{ color: "var(--down)", marginTop: 6 }}>
              {preview.error}
            </p>
          )}

          <Panel title="Render">
            <div className="render-grid">
              <Field label="Render scale" hint="Share of the 320 × 200 canvas the effect is drawn at">
                <Segmented<`${RenderScale}`>
                  label="Render scale"
                  value={`${draft.render_scale}` as `${RenderScale}`}
                  options={[
                    { value: "25", label: "25%" },
                    { value: "50", label: "50%" },
                    { value: "100", label: "100%" },
                  ]}
                  onChange={(v) => patchDraft((l) => ({ ...l, render_scale: Number(v) as RenderScale }))}
                />
              </Field>
              <Field label="Sampling" hint="How each LED reads the canvas">
                <Segmented<SamplingMode>
                  label="Sampling"
                  value={draft.sampling}
                  options={[
                    { value: "nearest", label: "Nearest" },
                    { value: "bilinear", label: "Bilinear" },
                    { value: "area_average", label: "Area average" },
                  ]}
                  onChange={(v) => patchDraft((l) => ({ ...l, sampling: v }))}
                />
              </Field>
            </div>
          </Panel>
        </div>

        <div className="stack">
          <Panel title={selectedZone ? "Zone" : "Zones"}>
            {shownZone && selected !== null ? (
              <ZoneInspector
                zone={shownZone}
                name={selectedInfo?.zone.name ?? `Zone ${shownZone.zone_index} · device not found`}
                deviceName={selectedInfo?.device.name ?? shortDeviceId(shownZone.device_id)}
                ledCount={selectedInfo?.zone.led_count ?? 0}
                missing={!selectedInfo}
                onChange={(patch) => changeZone(selected, patch)}
                onRemove={() => unplace(selected)}
              />
            ) : (
              <p className="mute">Select a zone on the preview, or in the list below.</p>
            )}
            <div className="stack" style={{ gap: 0, marginTop: 12 }}>
              {zones.map((z, i) => {
                const info = findZoneInfo(devices, z.device_id, z.zone_index);
                return (
                  <RowButton
                    key={`${z.device_id}#${z.zone_index}`}
                    current={selected === i}
                    label={`Select ${info?.zone.name ?? "the zone whose device is gone"}`}
                    onSelect={() => setSelected(i)}
                  >
                    <span className="row-title">
                      {info?.zone.name ?? "Device not found"}
                    </span>
                    <span className="row-sub one-line">{info ? info.device.name : "not connected"}</span>
                    <span className="fig mute">{info?.zone.led_count ?? "—"}</span>
                  </RowButton>
                );
              })}
            </div>
          </Panel>

          <Panel title="Unplaced zones" actions={<span className="mute fig">{unplaced.length}</span>}>
            {unplaced.length === 0 && <p className="mute">Every zone is placed.</p>}
            <div className="unplaced">
              {unplaced.map((u) => (
                <div
                  key={`${u.deviceId}#${u.zoneIndex}`}
                  className="unplaced-item"
                  draggable
                  onDragStart={(e) => dragStart(u.deviceId, u.zoneIndex, e)}
                >
                  <span style={{ minWidth: 0 }}>
                    <span className="row-title" style={{ display: "block" }}>
                      {u.zoneName} <span className="fig mute">{u.ledCount}</span>
                    </span>
                    <span className="row-sub one-line" style={{ display: "block" }}>
                      {u.deviceName}
                      {!u.connected && " · disconnected"}
                    </span>
                  </span>
                  <IconButton
                    label={`Place ${u.zoneName}`}
                    icon={<Plus size={16} weight="bold" />}
                    size="sm"
                    onClick={() => place(u.deviceId, u.zoneIndex)}
                  />
                </div>
              ))}
            </div>
          </Panel>

          <div className="savebar">
            <span className={dirty ? "warn-text fig" : "mute"}>{dirty ? "Unsaved layout changes" : "Saved"}</span>
            <div className="row-wrap">
              <Button variant="ghost" icon={<ArrowCounterClockwise size={16} />} disabled={!dirty || saving} onClick={revert}>
                Revert
              </Button>
              <Button variant="primary" icon={<FloppyDisk size={16} weight="bold" />} disabled={!dirty || saving} onClick={save}>
                {saving ? "Saving" : "Save layout"}
              </Button>
            </div>
            {saveError && <span className="error-line">{saveError}</span>}
          </div>
        </div>
      </div>
    </div>
  );
}

function ZoneInspector({
  zone,
  name,
  deviceName,
  ledCount,
  missing,
  onChange,
  onRemove,
}: {
  zone: PlacedZone;
  name: string;
  deviceName: string;
  ledCount: number;
  missing: boolean;
  onChange: (patch: Partial<PlacedZone>) => void;
  onRemove: () => void;
}) {
  return (
    <div className="stack" style={{ gap: 8 }}>
      <div className="row-title" style={{ justifyContent: "space-between" }}>
        <span className="one-line">{name}</span>
        <span className="fig mute">{missing ? "—" : `${ledCount} LEDs`}</span>
      </div>
      <p className="row-sub one-line">{deviceName}</p>
      {missing && (
        <div className={desk.stale}>
          <span className={desk.staleReason}>This device is not connected, so this zone draws nothing.</span>
          <Button size="sm" variant="ghost" icon={<X size={14} />} onClick={onRemove}>
            Remove from layout
          </Button>
        </div>
      )}
      <div className="grid-2" style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
        <Field label="X">
          <NumberField label="X" value={zone.x} min={0} max={CANVAS_W} onCommit={(v) => onChange({ x: v })} />
        </Field>
        <Field label="Y">
          <NumberField label="Y" value={zone.y} min={0} max={CANVAS_H} onCommit={(v) => onChange({ y: v })} />
        </Field>
        <Field label="Width">
          <NumberField label="Width" value={zone.width} min={2} max={CANVAS_W} onCommit={(v) => onChange({ width: v })} />
        </Field>
        <Field label="Height">
          <NumberField label="Height" value={zone.height} min={2} max={CANVAS_H} onCommit={(v) => onChange({ height: v })} />
        </Field>
      </div>
      <Field label="Rotation">
        <NumberField
          label="Rotation"
          value={zone.rotation_deg}
          min={0}
          max={359.9}
          decimals={1}
          unit="°"
          onCommit={(v) => onChange({ rotation_deg: v })}
        />
      </Field>
      <Field label="Visible">
        <Toggle label="Visible" checked={zone.visible} onChange={(v) => onChange({ visible: v })} />
      </Field>
      <Field label="Reversed" hint="Runs the LEDs the other way">
        <Toggle label="Reversed" checked={zone.reversed} onChange={(v) => onChange({ reversed: v })} />
      </Field>
      {!missing && (
        <div className="row-wrap" style={{ justifyContent: "flex-end" }}>
          <Button size="sm" variant="ghost" icon={<X size={14} />} onClick={onRemove}>
            Remove from layout
          </Button>
        </div>
      )}
    </div>
  );
}
