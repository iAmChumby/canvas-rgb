"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type ReactNode } from "react";
import {
  ArrowCounterClockwise,
  ArrowsClockwise,
  ArrowUUpLeft,
  ArrowUUpRight,
  ArrowUp,
  CaretDown,
  CaretUp,
  CircleDashed,
  Cube,
  DotOutline,
  DotsThree,
  FloppyDisk,
  GridFour,
  Lightbulb,
  MagicWand,
  Plus,
  Rectangle,
  X,
} from "@phosphor-icons/react";
import { Button, IconButton, Swap } from "@/components/Button";
import { Panel, Field } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { NumberField, TextField } from "@/components/NumberField";
import { Slider } from "@/components/Slider";
import { Toggle } from "@/components/Toggle";
import { Chip, RowButton } from "@/components/Chip";
import { EffectPreview } from "@/components/EffectPreview";
import { Popover } from "@/components/Popover";
import { Thumb } from "@/components/Thumb";
import { useMediaQuery } from "@/components/MediaQuery";
import { KindIcon } from "@/components/KindIcon";
import { autoPlace } from "@/lib/caseForms";
import { PreviewStage } from "@/views/PreviewStage";
import { PropertyControls, type Values } from "@/views/PropertyControls";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { usePreviewStatus } from "@/lib/preview";
import { usePrefersReducedMotion } from "@/lib/motion";
import { getSnap, patchSnap, refreshAll, refreshLayouts, refreshSettings, useSnap } from "@/lib/store";
import { defaultRect, defaultShape, findZoneInfo, plainName, reshapeBox, resolveShape, SHAPE_LABEL, SHAPE_ORDER } from "@/lib/zones";
import { clamp } from "@/lib/format";
import type { CaseForm } from "@/proto/CaseForm";
import type { Layout } from "@/proto/Layout";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { PlacedZone } from "@/proto/PlacedZone";
import type { PropValue } from "@/proto/PropValue";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { ZoneShape } from "@/proto/ZoneShape";
import desk from "./desk.module.css";
import type { RenderScale } from "@/proto/RenderScale";
import type { SamplingMode } from "@/proto/SamplingMode";
import { AnimatedNumber } from "@/components/AnimatedNumber";

const DROP_TYPE = "application/x-canvas-zone";
const CANVAS_W = 320;
const CANVAS_H = 200;
/** A value of the running effect reaches the daemon this long after the last change (ms). */
const PROPS_DEBOUNCE_MS = 60;

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


/**
 * The effect's top properties: the first three of the effect's own, plus its speed if it has
 * one and it is not among them. An effect's colours and its speed are what a reader changes
 * while they watch the desk; the rest is for the Effects page.
 */
function topProperties(schema: PropertySchema): PropertySchema {
  const picked = schema.properties.slice(0, 3);
  const speed = schema.properties.find((p) => p.kind.type === "number" && !picked.includes(p));
  if (speed) picked.push(speed);
  return { ...schema, properties: picked };
}

/**
 * The running effect, beside the canvas: its name, a live picture of it, a quick switch to
 * another one, its top properties and the brightness every device shares.
 *
 * The picture and the name are always there - the panel says what is playing. The controls
 * fold away behind the chevron, so the panel takes a corner of the screen and not all of it.
 */
function NowPlaying({
  runningId,
  name,
  description,
  values,
  schema,
  effects,
  brightness,
  error,
  onPick,
  onPropChange,
  onBrightness,
  onBrightnessCommit,
}: {
  runningId: string | null;
  name: string | null;
  description: string | null;
  values: Values;
  schema: PropertySchema | null;
  effects: EffectSummary[] | null;
  brightness: number;
  error: string | null;
  onPick: (id: string) => void;
  onPropChange: (key: string, value: PropValue) => void;
  onBrightness: (v: number) => void;
  onBrightnessCommit: (v: number) => void;
}) {
  // The effect's own controls: the colours, the speed and the brightness. They fold away, so
  // the panel is a corner of the screen and not a wall of it.
  const [controls, setControls] = useState(false);
  const [picker, setPicker] = useState(false);
  const changeRef = useRef<HTMLSpanElement>(null);
  const props = schema ? topProperties(schema) : null;
  // An effect that reads the screen or the audio cannot be drawn by the editor: the picture
  // is the daemon's.
  const needsInput = schema ? schema.needs.screen || schema.needs.audio : false;

  /**
   * The list of effects hangs under the button. It opens upwards when there is no room under
   * it, so the whole list is there to read wherever the panel is.
   */
  const openPicker = () => {
    setPicker(true);
  };

  return (
    <Panel
      title="Now playing"
      actions={
        <span className="row-wrap" style={{ alignItems: "center", gap: "var(--s2)" }}>
          <span ref={changeRef} className={desk.nowChange}>
            <Button
              size="sm"
              variant="ghost"
              icon={<ArrowsClockwise size={14} />}
              disabled={!effects || effects.length === 0}
              onClick={openPicker}
            >
              Change
            </Button>
          </span>
          <IconButton
            label={controls ? "Hide the effect's controls" : "Show the effect's controls"}
            icon={controls ? <CaretUp size={14} /> : <CaretDown size={14} />}
            size="sm"
            onClick={() => setControls((o) => !o)}
          />
        </span>
      }
    >
      <div className={desk.nowBody} data-now-playing="true">
        <div className={desk.nowWho}>
          <span className={desk.nowSwatch}>
            {runningId ? (
              needsInput ? (
                <Thumb id={runningId} size="sm" label={name ?? "Effect"} />
              ) : (
                <EffectPreview id={runningId} />
              )
            ) : null}
          </span>
          <div className={desk.nowWhoText}>
            <p className={desk.nowName}>{name ?? "No effect"}</p>
            <p className={desk.nowDesc}>
              {description ?? "Nothing running"}
            </p>
          </div>
        </div>
        {controls && (
          <>
            {props && (
              <div className={desk.nowProps}>
                <PropertyControls schema={props} values={values} onChange={onPropChange} />
              </div>
            )}
            <Field label="Brightness">
              <span className="row-wrap field-slider">
                <Slider
                  label="Brightness"
                  min={0}
                  max={100}
                  value={brightness}
                  onChange={onBrightness}
                  onCommit={onBrightnessCommit}
                />
                <NumberField
                  label="Brightness value"
                  min={0}
                  max={100}
                  value={brightness}
                  unit="%"
                  onCommit={onBrightnessCommit}
                />
              </span>
            </Field>
          </>
        )}
        {error && <p className="error-line">{error}</p>}
      </div>
      <Popover
        anchor={changeRef}
        open={picker}
        onClose={() => setPicker(false)}
        label="Choose the effect that runs on this desk"
      >
        <div className={desk.nowPicker}>
          {(effects ?? []).map((e) => (
            <Chip
              key={e.id}
              pressed={e.id === runningId}
              label={`Run ${e.name} on this desk`}
              onClick={() => {
                setPicker(false);
                onPick(e.id);
              }}
            >
              <Thumb id={e.id} size="sm" label={e.name} live={e.id === runningId} />
              <span className={desk.nowPickerName}>{e.name}</span>
            </Chip>
          ))}
        </div>
      </Popover>
    </Panel>
  );
}

export function LayoutView() {
  const server = useSnap((s) => s.layouts?.find((l) => l.id === s.activeLayout) ?? null);
  const devices = useSnap((s) => s.devices);
  const effects = useSnap((s) => s.effects);
  const settings = useSnap((s) => s.settings);
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

  /**
   * Zones that still have the shape of a layout saved before shapes existed (auto) where the
   * first run would pick another: a fan that is still a line, a keyboard that is still a strip.
   * One click applies the same per-device defaults as the first run (`default_shape`).
   */
  const suggested = useMemo(() => {
    const out: { index: number; shape: ZoneShape }[] = [];
    (draft?.zones ?? []).forEach((z, index) => {
      if (z.shape.type !== "auto") return;
      const info = findZoneInfo(devices, z.device_id, z.zone_index);
      if (!info || !info.device.connected) return;
      const shape = defaultShape(info.device.kind, info.zone);
      if (shape.type !== "auto") out.push({ index, shape });
    });
    return out;
  }, [draft, devices]);

  const useSuggestedShapes = () => {
    // One change, so one undo step: every shape and the box that suits it.
    commit((l) => ({
      ...l,
      zones: l.zones.map((z, i) => {
        const hit = suggested.find((s) => s.index === i);
        return hit ? { ...z, ...reshapeBox(z, hit.shape) } : z;
      }),
    }));
  };

  /**
   * Identify: the daemon lights one white LED at a time (POST identify walks a single LED of one
   * zone, everything else on that device black), so the owner can see which physical header or
   * strip the zone is. The walk runs for about five seconds, or until it is pressed again, and
   * the zone flashes on the desk meanwhile.
   */
  const [identifying, setIdentifying] = useState<number | null>(null);
  const identifyRef = useRef<{ timer: ReturnType<typeof setInterval>; deviceId: string; zone: number } | null>(null);
  const stopIdentify = useCallback(() => {
    const run = identifyRef.current;
    identifyRef.current = null;
    setIdentifying(null);
    if (!run) return;
    clearInterval(run.timer);
    void api.identify(run.deviceId, { zone: run.zone, led: 0, stop: true }).catch(() => undefined);
  }, []);
  const startIdentify = (index: number) => {
    stopIdentify();
    const z = draft?.zones[index];
    const info = z ? findZoneInfo(devices, z.device_id, z.zone_index) : null;
    if (!z || !info) return;
    const count = Math.max(1, info.zone.led_count);
    let tick = 0;
    const step = () => {
      if (tick >= 40) {
        stopIdentify();
        return;
      }
      void api
        .identify(z.device_id, { zone: z.zone_index, led: tick % count, stop: false })
        .catch(() => undefined);
      tick += 1;
    };
    step();
    identifyRef.current = { timer: setInterval(step, 125), deviceId: z.device_id, zone: z.zone_index };
    setIdentifying(index);
  };
  // Leaving the page ends the walk: no LED stays lit.
  useEffect(() => stopIdentify, [stopIdentify]);
  // Picking another zone ends it too.
  const selectZone = (index: number | null) => {
    if (identifying !== null && identifying !== index) stopIdentify();
    setSelected(index);
  };

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
  const selectedPlain = plainName(devices, selectedZone?.device_id ?? "", selectedZone?.zone_index ?? 0);
  const preview = usePreviewStatus();
  // While a zone is being dragged the inspector shows where it is, not where it was.
  const shownZone = liveZone && liveZone.index === selected ? liveZone.zone : selectedZone;
  // The running effect: the editor draws it itself when the daemon's preview is not connected.
  const runningEffect = server?.effect ?? null;
  const runningId = runningEffect?.id ?? null;
  /**
   * The "Now playing" panel: the running effect's own values, and the values as the reader
   * changes them. The canvas follows the second ones at once, so a colour moves before the
   * daemon has answered.
   */
  const [nowDetail, setNowDetail] = useState<EffectDetail | null>(null);
  const [liveProps, setLiveProps] = useState<Values>({});
  const [nowError, setNowError] = useState<string | null>(null);
  const [brightnessDraft, setBrightnessDraft] = useState<number | null>(null);
  /** The values waiting for the daemon: a drag sends one value, not forty. */
  const pendingProps = useRef<Values>({});
  const propsTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // A different effect running means different values: the old ones go.
  const [propsFor, setPropsFor] = useState<string | null>(null);
  if (runningId !== propsFor) {
    setPropsFor(runningId);
    setNowDetail(null);
    setLiveProps(runningEffect?.props ?? {});
  }
  useEffect(() => {
    if (!runningId) return;
    let stop = false;
    api
      .effect(runningId)
      .then((d) => {
        if (stop) return;
        setNowDetail(d);
        setLiveProps(d.props);
      })
      .catch((e: unknown) => {
        if (!stop) setNowError(messageOf(e));
      });
    return () => {
      stop = true;
    };
  }, [runningId]);
  useEffect(
    () => () => {
      if (propsTimer.current) clearTimeout(propsTimer.current);
    },
    [],
  );
  /** Sends the waiting values to the effect they were edited on, and to no other. */
  const flushProps = async (id: string | null) => {
    if (propsTimer.current) clearTimeout(propsTimer.current);
    propsTimer.current = null;
    const batch = pendingProps.current;
    pendingProps.current = {};
    if (!id || Object.keys(batch).length === 0) return;
    // The edit belongs to the effect that was running when it was made. If another one runs now
    // (picked elsewhere, or by the CLI), the values are dropped, not merged into it.
    if (getSnap().status?.active_effect !== id) return;
    try {
      await api.patchActiveProps(batch);
    } catch (e: unknown) {
      setNowError(messageOf(e));
    }
  };
  // A different effect running: edits that never left belong to the old one.
  useEffect(() => {
    return () => {
      if (propsTimer.current) clearTimeout(propsTimer.current);
      propsTimer.current = null;
      pendingProps.current = {};
    };
  }, [runningId]);
  /** A value of the running effect: the canvas has it at once, the daemon gets it after. */
  const onPropChange = (key: string, value: PropValue) => {
    setNowError(null);
    setLiveProps((prev) => ({ ...prev, [key]: value }));
    pendingProps.current = { ...pendingProps.current, [key]: value };
    if (propsTimer.current) clearTimeout(propsTimer.current);
    const id = runningId;
    propsTimer.current = setTimeout(() => void flushProps(id), PROPS_DEBOUNCE_MS);
  };
  /** Another effect on this desk. */
  const pickEffect = async (id: string) => {
    if (id === runningId) return;
    setNowError(null);
    try {
      // What was edited on the effect that is leaving goes to it first, then the switch.
      await flushProps(runningId);
      await api.activateEffect(id, { props: {}, preset: null });
      await refreshAll();
    } catch (e) {
      setNowError(messageOf(e));
    }
  };
  /** The brightness every device shares. */
  const commitBrightness = async (v: number) => {
    setNowError(null);
    try {
      const out = await api.patchOutput({ power: null, brightness: v });
      const cur = getSnap().settings;
      if (cur) patchSnap({ settings: { ...cur, brightness: out.brightness } });
      setBrightnessDraft(null);
      await refreshSettings();
    } catch (e) {
      setBrightnessDraft(null);
      setNowError(messageOf(e));
    }
  };
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

  /**
   * Picks the PC form factor drawn behind the zones. Choosing a case puts every device in its
   * place in it (fans in the fan positions, RAM in the RAM slots, headers along the case edges,
   * the keyboard and mic on the desk) as one change, so one undo step. "No case" only takes the
   * drawing away and leaves every zone where it is.
   */
  const chooseCase = useCallback(
    (form: CaseForm) => {
      commit((l) => ({ ...l, case_form: form, zones: autoPlace(form, devices, l.zones) }));
    },
    [commit, devices],
  );

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
    const shape = defaultShape(info.device.kind, info.zone);
    const r = defaultRect(info.zone, info.device.kind, shape);
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
      shape,
    };
    commit((l) => ({ ...l, zones: [...l.zones, placedZone] }));
    setSelected(draft.zones.length);
  };

  const unplace = useCallback(
    (index: number) => {
      stopIdentify();
      commit((l) => ({ ...l, zones: l.zones.filter((_, i) => i !== index) }));
      setSelected(null);
    },
    [commit, stopIdentify],
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
            {layoutsError ? `Layout unreadable: ${layoutsError}` : "Loading"}
          </p>
        </Panel>
      </div>
    );
  }

  return (
    <div className="stack">
      <div className="page-head">
        <h1>{draft.name}</h1>
      </div>

      <div className="layout-grid">
        <div className="layout-stage-col stack">
          <div className="layout-canvas" ref={stageRef}>
            <PreviewStage
              zones={zones}
              devices={devices}
              selected={selected}
              onSelect={selectZone}
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
              effect={runningEffect ? { id: runningEffect.id, props: liveProps } : null}
              effectNeedsInput={effectNeedsInput}
              sampling={draft.sampling}
              identifying={identifying}
              caseForm={draft.case_form}
              onCaseForm={chooseCase}
            />
            <div className="stage-meta">
              <span className="dropzone-hint" />
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
                  <AnimatedNumber value={zones.filter((z) => z.visible).length} /> of <AnimatedNumber value={zones.length} /> zones showing
                </span>
              </span>
            </div>
          </div>

          <div className="layout-render">
            <Panel title="Render">
              <div className="render-grid">
                <div className={desk.renderCell}>
                  <Field label="Render area">
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
                    <span className="fig">{renderNote.value}</span>
                  </p>
                </div>
                <div className={desk.renderCell}>
                  <Field label="Sampling">
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
                    <span className="fig">{samplingNote.value}</span>
                  </p>
                </div>
              </div>
            </Panel>
          </div>
        </div>

        <div className="layout-side stack">
          {/* What is playing sits beside the canvas, not under it: the panel is the one thing
              a reader wants to see while they watch the desk. */}
          <div className="layout-now">
            <NowPlaying
              runningId={runningId}
              name={nowDetail?.schema.name ?? effectName}
              description={nowDetail?.schema.description ?? null}
              values={liveProps}
              schema={nowDetail?.schema ?? null}
              effects={effects}
              brightness={brightnessDraft ?? settings?.brightness ?? 100}
              error={nowError}
              onPick={(id) => void pickEffect(id)}
              onPropChange={onPropChange}
              onBrightness={(v) => setBrightnessDraft(v)}
              onBrightnessCommit={(v) => void commitBrightness(v)}
            />
          </div>

          <div className="layout-zones">
            <Panel title={selectedZone ? "Zone" : "Zones"}>
              {shownZone && selected !== null ? (
                <div ref={inspectorRef}>
                  <ZoneInspector
                    key={selected}
                    zone={shownZone}
                    name={selectedInfo ? selectedPlain.name : staleDeviceName(shownZone.device_id, devices)}
                    icon={selectedInfo ? <KindIcon kind={selectedPlain.kind} fan={selectedPlain.fan} size={18} /> : null}
                    deviceName={
                      selectedInfo
                        ? selectedPlain.hardware && selectedPlain.hardware !== selectedInfo.device.name
                          ? `${selectedPlain.hardware} · ${selectedInfo.device.name}`
                          : selectedInfo.device.name
                        : shortDeviceId(shownZone.device_id)
                    }
                    ledCount={selectedInfo?.zone.led_count ?? 0}
                    info={selectedInfo?.zone ?? null}
                    identifying={identifying === selected}
                    onIdentify={() => (identifying === selected ? stopIdentify() : startIdentify(selected))}
                    missing={!selectedInfo}
                    onChange={(patch) => changeZone(selected, patch)}
                    onRemove={() => unplace(selected)}
                    onBack={phone ? backToCanvas : null}
                  />
                </div>
              ) : (
                null
              )}
              {draft.case_form === "none" && (
                <div className={desk.suggest}>
                  <p className={desk.plainNote} style={{ marginTop: 0 }}>
                    Pick your PC&apos;s shape and every device goes where its hardware sits: fans in the fan
                    positions, RAM in the RAM slots, strips along the case edges, the keyboard on the desk.
                  </p>
                  <Button size="sm" variant="ghost" icon={<Cube size={14} />} onClick={() => chooseCase("mid_tower")}>
                    Put my devices in a case
                  </Button>
                </div>
              )}
              {suggested.length > 0 && (
                <div className={desk.suggest}>
                  <p className={desk.plainNote} style={{ marginTop: 0 }}>
                    {suggested.length === 1 ? "One zone has" : `${suggested.length} zones have`} no shape yet. Fans become rings, RAM
                    and headers strips, a single-LED device a point.
                  </p>
                  <Button size="sm" variant="ghost" icon={<MagicWand size={14} />} onClick={useSuggestedShapes}>
                    Use suggested shapes
                  </Button>
                </div>
              )}
              <div className="stack" style={{ gap: 0, marginTop: 12 }}>
                {zones.map((z, i) => {
                  const info = findZoneInfo(devices, z.device_id, z.zone_index);
                  const pn = plainName(devices, z.device_id, z.zone_index);
                  const friendly = z.label?.trim() || null;
                  return (
                    <RowButton
                      key={`${z.device_id}#${z.zone_index}`}
                      current={selected === i}
                      label={`Select ${friendly || pn.name}`}
                      onSelect={() => selectZone(i)}
                    >
                      <span className="row-title">
                        {info && <KindIcon kind={pn.kind} fan={pn.fan} size={15} />}{" "}
                        {friendly || pn.name}
                      </span>
                      <span className="row-sub one-line">
                        {info
                          ? [friendly ? pn.name : pn.hardware, info.device.name]
                              .filter((t, i, arr) => t && t !== (friendly ? "" : pn.name) && arr.indexOf(t) === i)
                              .join(" · ")
                          : "this device is gone"}
                      </span>
                      <span className="fig mute">{info ? <AnimatedNumber value={info.zone.led_count} /> : "—"}</span>
                    </RowButton>
                  );
                })}
              </div>
            </Panel>
          </div>

          <div className="layout-unplaced">
            <Panel title="Unplaced zones" actions={<span className="mute fig"><AnimatedNumber value={unplaced.length} /></span>}>
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
                        {u.zoneName} <span className="fig mute"><AnimatedNumber value={u.ledCount} /></span>
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
          </div>

          <div className="layout-save">
            <div className="savebar" data-dirty={dirty ? "true" : "false"}>
              <span className={dirty ? "warn-text fig" : "mute"}>{dirty ? "Unsaved layout changes" : "Saved"}</span>
              <div className="row-wrap">
                <Button variant="ghost" icon={<ArrowCounterClockwise size={16} />} disabled={!dirty || saving} onClick={revert}>
                  Revert
                </Button>
                <Button variant="primary" icon={<FloppyDisk size={16} weight="bold" />} disabled={!dirty || saving} onClick={save}>
                  <Swap labels={["Save layout", "Saving"]} at={saving ? 1 : 0} />
                </Button>
              </div>
              {saveError && <span className="error-line">{saveError}</span>}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

/** The shape picker's icons: a tiny drawing of each shape, from Phosphor. */
const SHAPE_ICON: Record<ZoneShape["type"], ReactNode> = {
  auto: <MagicWand size={15} />,
  strip: <DotsThree size={15} weight="bold" />,
  ring: <CircleDashed size={15} />,
  outline: <Rectangle size={15} />,
  point: <DotOutline size={15} weight="fill" />,
  grid: <GridFour size={15} />,
};

/** What each shape does, in one line. */
const SHAPE_NOTE: Record<ZoneShape["type"], string> = {
  auto: "Follows the device: a strip for a line of LEDs, a point for one, its own grid for a matrix.",
  strip: "LEDs in a line along the long side.",
  ring: "LEDs around an ellipse, clockwise from the top, like a fan.",
  outline: "LEDs around the box's edge, clockwise from the top-left.",
  point: "Every LED reads the whole box, as one blob of light.",
  grid: "LEDs in rows, left to right, top to bottom.",
};

function ZoneInspector({
  zone,
  name,
  icon,
  deviceName,
  ledCount,
  info,
  identifying,
  onIdentify,
  missing,
  onChange,
  onRemove,
  onBack,
}: {
  zone: PlacedZone;
  name: string;
  icon: ReactNode;
  deviceName: string;
  ledCount: number;
  info: ZoneInfo | null;
  identifying: boolean;
  onIdentify: () => void;
  missing: boolean;
  onChange: (patch: Partial<PlacedZone>) => void;
  onRemove: () => void;
  /** On a phone: the way back to the canvas the zone was picked on. */
  onBack: (() => void) | null;
}) {
  const shapeType = zone.shape.type;
  const resolved = info ? resolveShape(zone.shape, info.kind) : null;
  const pickShape = (type: ZoneShape["type"]) => {
    if (type === shapeType) return;
    const next: ZoneShape =
      type === "grid"
        ? { type: "grid", columns: Math.max(1, Math.min(ledCount || 1, Math.ceil((ledCount || 1) / 6))) }
        : ({ type } as ZoneShape);
    // One change, so one undo step: the shape and the box that suits it.
    onChange(reshapeBox(zone, next));
  };
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
        <span className={missing ? "" : "one-line"} style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
          {icon}
          {name}
        </span>
        <span className="fig mute">{missing ? "not connected" : <><AnimatedNumber value={ledCount} /> LEDs</>}</span>
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
      <Field label="Name" hint={<span className="fig">{zone.label?.trim() ? `${(zone.label ?? "").trim().length} / 32` : "hardware name"}</span>}>
        <div className={desk.nameRow}>
          <TextField
            label="Name"
            value={zone.label ?? ""}
            placeholder={name}
            allowEmpty
            maxLength={32}
            onCommit={(v) => onChange({ label: v.trim() === "" ? undefined : v.trim().slice(0, 32) })}
          />
          <Button
            size="sm"
            variant="ghost"
            icon={<Lightbulb size={14} weight={identifying ? "fill" : "regular"} />}
            disabled={missing}
            onClick={onIdentify}
          >
            {identifying ? "Stop" : "Identify"}
          </Button>
        </div>
      </Field>
      <p className={desk.plainNote} style={{ marginTop: 0 }}>
        Identify walks one white LED along this zone ({name}) for about five seconds, so you can see which header it
        is. It lights one LED of the zone at a time, not the whole device.
      </p>
      <Field label="Shape" hint={<span className="fig">{SHAPE_LABEL[shapeType]}{shapeType === "auto" && resolved ? ` · ${SHAPE_LABEL[resolved === "matrix" ? "grid" : resolved]}` : ""}</span>}>
        <Segmented<ZoneShape["type"]>
          label="How the LEDs sit in the box"
          value={shapeType}
          iconOnly
          options={SHAPE_ORDER.map((type) => ({ value: type, label: SHAPE_LABEL[type], icon: SHAPE_ICON[type] }))}
          onChange={pickShape}
        />
      </Field>
      <p className={desk.plainNote} style={{ marginTop: 0 }}>
        {SHAPE_NOTE[shapeType]} A bigger shape covers more of the effect.
      </p>
      {zone.shape.type === "grid" && (
        <Field label="Columns">
          <NumberField
            label="Columns"
            value={zone.shape.columns}
            min={1}
            max={Math.max(1, ledCount)}
            onCommit={(v) => onChange({ shape: { type: "grid", columns: Math.round(v) } })}
          />
        </Field>
      )}
      <div className="grid-2" style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
        <Field label="X">
          <NumberField label="X" value={zone.x} min={0} max={CANVAS_W} decimals={1} onCommit={(v) => onChange({ x: v })} />
        </Field>
        <Field label="Y">
          <NumberField label="Y" value={zone.y} min={0} max={CANVAS_H} decimals={1} onCommit={(v) => onChange({ y: v })} />
        </Field>
        <Field label="Width">
          <NumberField label="Width" value={zone.width} min={2} max={CANVAS_W} decimals={1} onCommit={(v) => onChange({ width: v })} />
        </Field>
        <Field label="Height">
          <NumberField label="Height" value={zone.height} min={2} max={CANVAS_H} decimals={1} onCommit={(v) => onChange({ height: v })} />
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
      <Field label="LEDs" hint="set on the Devices page">
        <span className="fig" aria-label={`${ledCount} LEDs`}>
          {missing ? "—" : ledCount}
        </span>
      </Field>
      <Field label="Reverse direction">
        <Toggle label="Reverse direction" checked={zone.reversed} onChange={(v) => onChange({ reversed: v })} />
      </Field>
      <Field label="Visible">
        <Toggle label="Visible" checked={zone.visible} onChange={(v) => onChange({ visible: v })} />
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
