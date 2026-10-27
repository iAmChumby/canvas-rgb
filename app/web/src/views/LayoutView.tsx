"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { ArrowCounterClockwise, ArrowUUpLeft, ArrowUUpRight, ArrowUp, FloppyDisk, Plus, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Panel, Field } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { NumberField } from "@/components/NumberField";
import { Toggle } from "@/components/Toggle";
import { RowButton } from "@/components/Chip";
import { useMediaQuery } from "@/components/MediaQuery";
import { PreviewStage } from "@/views/PreviewStage";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { usePreviewStatus } from "@/lib/preview";
import { usePrefersReducedMotion } from "@/lib/motion";
import { refreshLayouts, useSnap } from "@/lib/store";
import { defaultRect, findZoneInfo } from "@/lib/zones";
import { clamp } from "@/lib/format";
import type { Layout } from "@/proto/Layout";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { PlacedZone } from "@/proto/PlacedZone";
import desk from "./desk.module.css";
import type { RenderScale } from "@/proto/RenderScale";
import type { SamplingMode } from "@/proto/SamplingMode";

const DROP_TYPE = "application/x-canvas-zone";
const CANVAS_W = 320;
const CANVAS_H = 200;

/**
 * The layout as it is on the page, and the steps that led to it. The past is what undo takes
 * back; the future is what redo takes forward.
 */
interface Edit {
  draft: Layout | null;
  past: Layout[];
  future: Layout[];
}

/**
 * What the render scale does, in words, and the precise value beside it. The words are what
 * the reader acts on; the value is there for the reader who wants the number.
 */
const RENDER_SCALE_NOTE: Record<string, { text: string; value: string }> = {
  "25": {
    text: "A quarter of the desk: the effect is drawn small and then scaled up, so the edges are softer and the work is lighter.",
    value: "25 %",
  },
  "50": {
    text: "Half of the desk: a balance between the detail of the picture and the work of drawing it.",
    value: "50 %",
  },
  "100": {
    text: "The whole desk: every LED reads the picture at the desk's own size, which is the sharpest and the most work.",
    value: "100 %",
  },
};

/** What each way of reading the picture does, in words, and its name. */
const SAMPLING_NOTE: Record<string, { text: string; value: string }> = {
  nearest: {
    text: "Each LED takes the colour of the one pixel it sits on: the sharpest, and the lightest.",
    value: "Nearest",
  },
  bilinear: {
    text: "Each LED blends the four pixels around it, so the edge between them fades.",
    value: "Bilinear",
  },
  area_average: {
    text: "Each LED takes the average of every pixel it covers, so it is lit evenly.",
    value: "Area average",
  },
};

/** The layout without its effect: the effect is owned by the Effects page, not by this draft. */
function withoutEffect(l: Layout): Layout {
  return { ...l, effect: null };
}

/**
 * A name for a device the daemon does not list: the same VID/PID as a listed device, if there is
 * one. The stale zone on this PC is OpenRGB's copy of the keyboard, and the native adapter's id
 * carries the same numbers, so the editor can say what it is instead of "zone 0".
 */
function staleDeviceName(deviceId: string, devices: DeviceInfo[] | null): string {
  const vid = /VID_([0-9A-Fa-f]{4})/.exec(deviceId)?.[1];
  const pid = /PID_([0-9A-Fa-f]{4})/.exec(deviceId)?.[1];
  if (vid && pid) {
    const key = `${vid.toLowerCase()}:${pid.toLowerCase()}`;
    const same = devices?.find(
      (d) => d.id.toLowerCase().includes(key) || d.location.toLowerCase().includes(key),
    );
    if (same) return `${same.name} (OpenRGB copy)`;
  }
  return "Device not connected";
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

  const [base, setBase] = useState<Layout | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  /**
   * The layout as it is on the page, and the history of how it got there. Every edit is a step:
   * the layout as it was goes on the history, and undo takes it back. The two stacks live with
   * the draft in one piece of state, so an edit and its step can never come apart.
   */
  const [edit, setEdit] = useState<Edit>({ draft: null, past: [], future: [] });
  const { draft, past, future } = edit;
  /** True while a zone is being dragged: the server's layout must not replace the draft then. */
  const [dragging, setDragging] = useState(false);
  /**
   * Where the dragged zone is right now. The canvas reports it once per animation frame, so the
   * inspector follows the pointer without the drag re-rendering the canvas every move.
   */
  const [liveZone, setLiveZone] = useState<{ index: number; zone: PlacedZone } | null>(null);
  /**
   * On a phone the canvas is at the top of the page and the inspector is under it: picking a
   * zone has to bring the inspector into view, and the reader needs a way back to the canvas.
   */
  const phone = useMediaQuery("(max-width: 720px)");
  const reduced = usePrefersReducedMotion();
  const stageRef = useRef<HTMLDivElement>(null);
  const inspectorRef = useRef<HTMLDivElement>(null);

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
    // A layout that arrives from the daemon starts a new history: the old steps belong to the
    // layout they were made on.
    setEdit({ draft: server, past: [], future: [] });
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
  // The running effect: the editor draws it itself when the daemon's preview is not connected.
  const runningEffect = server?.effect ?? null;
  const effectNeedsInput = useMemo(() => {
    if (!runningEffect) return false;
    const summary = effects?.find((e) => e.id === runningEffect.id);
    return !!(summary?.needs.screen || summary?.needs.audio);
  }, [runningEffect, effects]);
  // The editor draws the effect itself when the daemon's frames are not coming.
  const editorDraws = preview.state !== "live" && !!runningEffect && !effectNeedsInput;
  // The daemon's frames are the honest picture. Without them the editor draws the effect, and
  // says so in one quiet line. Red is for when there is no picture at all.
  const previewLine = useMemo(() => {
    if (preview.state === "live") return null;
    if (editorDraws) {
      return {
        text: "Preview drawn by the editor · the daemon's live preview is not available yet",
        down: false,
      };
    }
    if (preview.state === "refused") {
      return {
        text: `The live preview is not connected. ${preview.error ?? ""}`.trim(),
        down: true,
      };
    }
    if (effectNeedsInput) {
      return {
        text: "This effect reads the screen or the audio, so the preview is the daemon's frame.",
        down: true,
      };
    }
    if (!runningEffect) {
      return { text: "No effect is running on this layout.", down: true };
    }
    return { text: "The daemon's live preview is not available yet.", down: false };
  }, [preview.state, preview.error, editorDraws, effectNeedsInput, runningEffect]);

  // The plain-language note under each setting: what it does in words, and the precise value.
  const renderNote = RENDER_SCALE_NOTE[`${draft?.render_scale ?? 50}`] ?? RENDER_SCALE_NOTE["50"]!;
  const samplingNote = SAMPLING_NOTE[draft?.sampling ?? "nearest"] ?? SAMPLING_NOTE.nearest!;

  /**
   * An edit to the layout: the layout as it was goes on the history, so ctrl+z can take it
   * back. Every change on this page goes through here - a move, a resize, a turn, a zone
   * taken off, a zone put on, and the two settings under the canvas.
   */
  const commit = useCallback((fn: (l: Layout) => Layout) => {
    setEdit((e) => {
      if (!e.draft) return e;
      return { draft: fn(e.draft), past: [...e.past, e.draft], future: [] };
    });
  }, []);

  const undo = useCallback(() => {
    setEdit((e) => {
      const prev = e.past[e.past.length - 1];
      if (!e.draft || !prev) return e;
      return { draft: prev, past: e.past.slice(0, -1), future: [e.draft, ...e.future] };
    });
  }, []);

  const redo = useCallback(() => {
    setEdit((e) => {
      const next = e.future[0];
      if (!e.draft || !next) return e;
      return { draft: next, past: [...e.past, e.draft], future: e.future.slice(1) };
    });
  }, []);

  // Ctrl+Z and ctrl+shift+Z (and ctrl+Y) take a step back and take it again. A text field
  // keeps its own undo: the reader there is undoing what they typed.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      const key = e.key.toLowerCase();
      if (key === "z" && !e.shiftKey) {
        e.preventDefault();
        undo();
        return;
      }
      if ((key === "z" && e.shiftKey) || key === "y") {
        e.preventDefault();
        redo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo]);

  const changeZone = (index: number, patch: Partial<PlacedZone>) => {
    commit((l) => ({
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
    commit((l) => ({ ...l, zones: [...l.zones, placedZone] }));
    setSelected(draft.zones.length);
  };

  const unplace = useCallback(
    (index: number) => {
      commit((l) => ({ ...l, zones: l.zones.filter((_, i) => i !== index) }));
      setSelected(null);
    },
    [commit],
  );

  const revert = () => {
    if (base) commit(() => base);
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
      setEdit((e) => ({ ...e, draft: body }));
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

  /** Scrolls a panel into view. */
  const reveal = useCallback(
    (el: HTMLElement | null) => {
      if (!el) return;
      el.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "start" });
    },
    [reduced],
  );

  // Picking a zone reveals its inspector: on a phone the two are a screen apart.
  useEffect(() => {
    if (!phone || selected === null) return;
    reveal(inspectorRef.current);
  }, [phone, selected, reveal]);

  /** The way back to the canvas. */
  const backToCanvas = () => {
    const el = stageRef.current;
    if (!el) return;
    el.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "start" });
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
        <p className={desk.subtitle}>
          The desk canvas: every placed zone over the live preview of{" "}
          <span style={{ color: "var(--ink)" }}>{effectName}</span>.
        </p>
      </div>

      <div className="layout-grid">
        <div className="layout-stage-col stack" ref={stageRef}>
          <PreviewStage
            zones={zones}
            devices={devices}
            selected={selected}
            onSelect={setSelected}
            onChange={changeZone}
            onDrop={onDropUnplaced}
            onRemoveZone={unplace}
            onDraggingChange={(d) => {
              setDragging(d);
              if (!d) setLiveZone(null);
            }}
            onLiveChange={(index, zone) => setLiveZone({ index, zone })}
            layoutId={draft.id}
            dropPayloadType={DROP_TYPE}
            effect={runningEffect}
            effectNeedsInput={effectNeedsInput}
            sampling={draft.sampling}
          />
          <div className="stage-meta">
            <span className="dropzone-hint">
              {phone
                ? "Tap a zone to pick it. Drag it to move it, the corner to resize it."
                : "Drag to move, the corner to resize, the knob to rotate. Arrow keys nudge."}
            </span>
            <span className="row-wrap" style={{ alignItems: "center", gap: "var(--s3)" }}>
              <span className="row-wrap" style={{ alignItems: "center", gap: "var(--s1)" }}>
                <IconButton
                  label="Undo the last change"
                  icon={<ArrowUUpLeft size={15} />}
                  size="sm"
                  disabled={past.length === 0}
                  onClick={undo}
                />
                <IconButton
                  label="Redo the change that was undone"
                  icon={<ArrowUUpRight size={15} />}
                  size="sm"
                  disabled={future.length === 0}
                  onClick={redo}
                />
              </span>
              {previewLine && (
                <span className={previewLine.down ? desk.noteDown : desk.drawn}>{previewLine.text}</span>
              )}
              <span className="mute fig">
                {zones.filter((z) => z.visible).length} of {zones.length} zones showing
              </span>
            </span>
          </div>

          <Panel title="Render">
            <div className="render-grid">
              <div className={desk.renderCell}>
                <Field label="How much of the desk" hint="The desk is 320 × 200 units">
                  <Segmented<`${RenderScale}`>
                    label="How much of the desk the effect is drawn at"
                    value={`${draft.render_scale}` as `${RenderScale}`}
                    options={[
                      { value: "25", label: "A quarter" },
                      { value: "50", label: "Half" },
                      { value: "100", label: "All of it" },
                    ]}
                    onChange={(v) => commit((l) => ({ ...l, render_scale: Number(v) as RenderScale }))}
                  />
                </Field>
                <p className={desk.plainNote}>
                  {renderNote.text} · <span className="fig">{renderNote.value}</span>
                </p>
              </div>
              <div className={desk.renderCell}>
                <Field label="How each LED reads it" hint="What one LED takes from the picture">
                  <Segmented<SamplingMode>
                    label="How each LED reads the picture"
                    value={draft.sampling}
                    options={[
                      { value: "nearest", label: "One pixel" },
                      { value: "bilinear", label: "Four pixels" },
                      { value: "area_average", label: "Every pixel" },
                    ]}
                    onChange={(v) => commit((l) => ({ ...l, sampling: v }))}
                  />
                </Field>
                <p className={desk.plainNote}>
                  {samplingNote.text} · <span className="fig">{samplingNote.value}</span>
                </p>
              </div>
            </div>
          </Panel>
        </div>

        <div className="stack">
          <Panel title={selectedZone ? "Zone" : "Zones"}>
            {shownZone && selected !== null ? (
              <div ref={inspectorRef}>
                <ZoneInspector
                  key={selected}
                  zone={shownZone}
                  name={
                    selectedInfo?.zone.name ??
                    staleDeviceName(shownZone.device_id, devices)
                  }
                  deviceName={selectedInfo?.device.name ?? shortDeviceId(shownZone.device_id)}
                  ledCount={selectedInfo?.zone.led_count ?? 0}
                  missing={!selectedInfo}
                  onChange={(patch) => changeZone(selected, patch)}
                  onRemove={() => unplace(selected)}
                  onBack={phone ? backToCanvas : null}
                />
              </div>
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

          <div className="savebar" data-dirty={dirty ? "true" : "false"}>
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
  onBack,
}: {
  zone: PlacedZone;
  name: string;
  deviceName: string;
  ledCount: number;
  missing: boolean;
  onChange: (patch: Partial<PlacedZone>) => void;
  onRemove: () => void;
  /** On a phone: the way back to the canvas the zone was picked on. */
  onBack: (() => void) | null;
}) {
  return (
    <div className={`stack ${desk.inspector}`} style={{ gap: 8 }}>
      {onBack && (
        <div>
          <Button size="sm" variant="ghost" icon={<ArrowUp size={14} />} onClick={onBack}>
            Back to the canvas
          </Button>
        </div>
      )}
      <div className="row-title" style={{ justifyContent: "space-between" }}>
        <span className={missing ? "" : "one-line"}>{name}</span>
        <span className="fig mute">{missing ? "not connected" : `${ledCount} LEDs`}</span>
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
      <Field label="Rotation" hint="Shift while turning snaps to 15°">
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
