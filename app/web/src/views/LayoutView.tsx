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
  CompassTool,
  CircleDashed,
  DotOutline,
  DotsThree,
  FloppyDisk,
  GridFour,
  SquaresFour,
  Lightbulb,
  MagicWand,
  Plus,
  Rectangle,
  X,
} from "@phosphor-icons/react";
import { Button, IconButton, Swap } from "@/components/Button";
import { Panel, Field } from "@/components/Panel";
import { EmptyState } from "@/components/EmptyState";
import { Details, ErrorPanel, LoadFrame } from "@/components/Loading";
import { LayoutShape } from "@/components/LoadShapes";
import { Segmented } from "@/components/Segmented";
import { NumberField, TextField } from "@/components/NumberField";
import { Select } from "@/components/Select";
import { Slider } from "@/components/Slider";
import { Toggle } from "@/components/Toggle";
import { Chip, RowButton } from "@/components/Chip";
import { EffectPreview } from "@/components/EffectPreview";
import { Popover } from "@/components/Popover";
import { Thumb } from "@/components/Thumb";
import { useMediaQuery } from "@/components/MediaQuery";
import { KindIcon } from "@/components/KindIcon";
import { PreviewStage, type CaseGuide, type StageMode } from "@/views/PreviewStage";
import { Onboarding } from "@/views/editor/Onboarding";
import { putOffUntilNextLaunch, shouldOpen, type Scope } from "@/views/editor/flow";
import { useNewLights } from "@/views/editor/newLights";
import { PropertyControls, type Values } from "@/views/PropertyControls";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { usePreviewStatus } from "@/lib/preview";
import { usePrefersReducedMotion } from "@/lib/motion";
import { getSnap, patchSnap, refreshAll, refreshLayouts, refreshSettings, retryLayouts, useSnap } from "@/lib/store";
import {
  defaultRect,
  defaultShape,
  findZoneInfo,
  formatUnits,
  initialUnit,
  rememberUnit,
  lengthNumber,
  mmFromLength,
  placementName,
  plainName,
  reshapeBox,
  resolveShape,
  SHAPE_LABEL,
  SHAPE_ORDER,
  unitWord,
  zoneKey,
  type LengthUnit,
} from "@/lib/zones";
import { clamp } from "@/lib/format";
import {
  FACINGS,
  FACING_LABEL,
  LANDMARK_KINDS,
  PART_LABEL,
  PART_SIZE,
  type Sel,
} from "@/views/editor/parts";
import type { Facing } from "@/proto/Facing";
import type { Landmark } from "@/proto/Landmark";
import type { Layout } from "@/proto/Layout";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { PartKind } from "@/proto/PartKind";
import type { PlacedZone } from "@/proto/PlacedZone";
import type { PropValue } from "@/proto/PropValue";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import type { ZoneShape } from "@/proto/ZoneShape";
import desk from "./desk.module.css";
import type { RenderScale } from "@/proto/RenderScale";
import type { SamplingMode } from "@/proto/SamplingMode";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { PageTitle, PageActions } from "@/shell/PageHeader";

const DROP_TYPE = "application/x-canvas-zone";
const CANVAS_W = 320;
const CANVAS_H = 200;
/** A value of the running effect reaches the daemon this long after the last change (ms). */
const PROPS_DEBOUNCE_MS = 60;
/** Where the unit and the mode are remembered. */
const MODE_KEY = "canvas-rgb.mode";

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
  const layoutsLoaded = useSnap((s) => s.layouts !== null);
  // A load that is in flight after a Try again: the frame shows its placeholder until it ends.
  const retrying = useSnap((s) => s.pending.includes("layouts"));

  const [base, setBase] = useState<Layout | null>(null);
  const [selection, setSelection] = useState<Sel[]>([]);
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
  /** The unit every size is read in: centimetres or inches. */
  const [unit, setUnitState] = useState<LengthUnit>(initialUnit);
  /** The owner picked a unit: it is used now and remembered. */
  const setUnit = (u: LengthUnit) => {
    setUnitState(u);
    rememberUnit(u);
  };
  /** What the canvas shows: the effect under the parts, or the lights themselves. */
  const [mode, setMode] = useState<StageMode>(() => {
    try {
      const m = window.localStorage.getItem(MODE_KEY);
      if (m === "effect" || m === "lights") return m;
    } catch {
      // Not remembered, still used for this session.
    }
    return "effect";
  });
  /** The onboarding that maps the owner's PC: open, and the part it is limited to (if any). */
  const [onboarding, setOnboarding] = useState<{ scope: Scope | null; again: boolean } | null>(null);
  /** The LED the setup's walker has lit: the desk marks it. */
  const [litLed, setLitLed] = useState<{ deviceId: string; zoneIndex: number; led: number; count?: number } | null>(null);
  /**
   * On a phone the canvas is at the top of the page and the inspector is under it: picking a
   * zone has to bring the inspector into view, and the reader needs a way back to the canvas.
   */
  const phone = useMediaQuery("(max-width: 720px)");
  const reduced = usePrefersReducedMotion();
  const stageRef = useRef<HTMLDivElement>(null);
  const inspectorRef = useRef<HTMLDivElement>(null);
  /** The "add a part" menu hangs under its button. */
  const addRef = useRef<HTMLSpanElement>(null);
  const [adding, setAdding] = useState(false);

  const serverKey = server ? JSON.stringify(server) : "";
  const dirty =
    draft !== null && base !== null && JSON.stringify(withoutEffect(draft)) !== JSON.stringify(withoutEffect(base));

  // The unit, the mode and the locked parts are the reader's own choices: they are remembered
  // on this machine, not in the layout.
  useEffect(() => {
    try {
      window.localStorage.setItem(MODE_KEY, mode);
    } catch {
      // Not remembered, still used for this session.
    }
  }, [mode]);

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
  /**
   * Shows where a part is. The engine shows nothing else while it runs, so a chase (one LED
   * running along the part, to read its direction) or the whole part lit is easy to find. The
   * engine ends it a few seconds after the last request, so the whole-part light asks again.
   */
  const startIdentify = (index: number, how: "chase" | "whole" = "chase") => {
    stopIdentify();
    const z = draft?.zones[index];
    const info = z ? findZoneInfo(devices, z.device_id, z.zone_index) : null;
    if (!z || !info) return;
    const count = Math.max(1, info.zone.led_count);
    const whole = how === "whole";
    let tick = 0;
    const step = () => {
      if (tick >= (whole ? 30 : 40)) {
        stopIdentify();
        return;
      }
      void api
        .identify(
          z.device_id,
          whole
            ? { zone: z.zone_index, led: 0, ...(count > 1 ? { count } : {}), stop: false }
            : { zone: z.zone_index, led: tick % count, stop: false },
        )
        .catch(() => undefined);
      tick += 1;
    };
    step();
    identifyRef.current = { timer: setInterval(step, whole ? 2000 : 125), deviceId: z.device_id, zone: z.zone_index };
    setIdentifying(index);
  };
  // Leaving the page ends the walk: no LED stays lit.
  useEffect(() => stopIdentify, [stopIdentify]);
  // Picking another part ends it too.
  const select = (sel: Sel[]) => {
    const first = sel[0];
    if (identifying !== null && (!first || first.kind !== "zone" || first.index !== identifying)) stopIdentify();
    setSelection(sel);
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
  const landmarks = draft?.landmarks ?? [];
  /** The part that is picked: the first of the selection. */
  const primary = selection[0] ?? null;
  const selectedZone = primary?.kind === "zone" ? zones[primary.index] ?? null : null;
  const selectedLandmark = primary?.kind === "landmark" ? landmarks[primary.index] ?? null : null;
  const selectedInfo = selectedZone ? findZoneInfo(devices, selectedZone.device_id, selectedZone.zone_index) : null;
  const selectedPlain = plainName(devices, selectedZone?.device_id ?? "", selectedZone?.zone_index ?? 0);
  const preview = usePreviewStatus();
  // While a zone is being dragged the inspector shows where it is, not where it was.
  const shownZone = liveZone && primary?.kind === "zone" && liveZone.index === primary.index ? liveZone.zone : selectedZone;
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
        text: "Preview drawn by the editor · the live preview is not available yet",
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
        text: "This effect reads the screen or the audio, so the preview is the frame from canvas-rgb.",
        down: true,
      };
    }
    if (!runningEffect) {
      return { text: "No effect is running on this layout.", down: true };
    }
    return { text: "The live preview is not available yet.", down: false };
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

  /**
   * A change to one or more zones, as one undo step: a drag that moved three parts is one step,
   * not three.
   */
  const changeZones = useCallback(
    (patches: { index: number; patch: Partial<PlacedZone> }[]) => {
      if (patches.length === 0) return;
      commit((l) => ({
        ...l,
        zones: l.zones.map((z, i) => {
          const hit = patches.find((p) => p.index === i);
          return hit ? { ...z, ...hit.patch } : z;
        }),
      }));
    },
    [commit],
  );

  /**
   * A change to zones and landmarks together, as one undo step: a drag of a case that carries
   * zones and landmarks, or of a pick that mixes the two, is one step, not one per part.
   */
  const changeParts = useCallback(
    (
      zonePatches: { index: number; patch: Partial<PlacedZone> }[],
      landmarkPatches: { index: number; patch: Partial<Landmark> }[],
    ) => {
      if (zonePatches.length === 0 && landmarkPatches.length === 0) return;
      commit((l) => ({
        ...l,
        zones: l.zones.map((z, i) => {
          const hit = zonePatches.find((p) => p.index === i);
          return hit ? { ...z, ...hit.patch } : z;
        }),
        landmarks: (l.landmarks ?? []).map((m, i) => {
          const hit = landmarkPatches.find((p) => p.index === i);
          return hit ? { ...m, ...hit.patch } : m;
        }),
      }));
    },
    [commit],
  );

  /** A change to a landmark: the case, a CPU, a box. */
  const changeLandmark = useCallback(
    (index: number, patch: Partial<Landmark>) => {
      commit((l) => ({
        ...l,
        landmarks: (l.landmarks ?? []).map((m, i) => (i === index ? { ...m, ...patch } : m)),
      }));
    },
    [commit],
  );

  /** Puts a part on the desk that has no lights of its own: a CPU, a dark GPU, a box, a case. */
  const addLandmark = useCallback(
    (kind: PartKind) => {
      const size = PART_SIZE[kind];
      const at = { x: CANVAS_W / 2 - size.width / 2, y: CANVAS_H / 2 - size.height / 2 };
      const landmark: Landmark = {
        kind,
        x: at.x,
        y: at.y,
        width: size.width,
        height: size.height,
        rotation_deg: 0,
        facing: "viewer",
        visible: true,
      };
      commit((l) => ({ ...l, landmarks: [...(l.landmarks ?? []), landmark] }));
      setSelection([{ kind: "landmark", index: (draft?.landmarks ?? []).length }]);
    },
    [commit, draft],
  );

  /** Takes a landmark off the desk. */
  const removeLandmark = useCallback(
    (index: number) => {
      commit((l) => ({ ...l, landmarks: (l.landmarks ?? []).filter((_, i) => i !== index) }));
      setSelection([]);
    },
    [commit],
  );

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
    setSelection([{ kind: "zone", index: draft.zones.length }]);
  };

  const unplace = useCallback(
    (index: number) => {
      stopIdentify();
      commit((l) => ({ ...l, zones: l.zones.filter((_, i) => i !== index) }));
      setSelection([]);
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

  // While the onboarding is open every change is saved a moment after it is made, so what the
  // owner has mapped survives a closed window. A failed save is not retried in a loop.
  const saveRef = useRef(save);
  useEffect(() => {
    saveRef.current = save;
  });
  useEffect(() => {
    if (!onboarding || !dirty || saving || dragging || saveError) return;
    const t = setTimeout(() => void saveRef.current(), 600);
    return () => clearTimeout(t);
  }, [onboarding, dirty, saving, dragging, saveError, draft]);

  /** Every part with lights is unconfirmed again, so the flow walks all of them. */
  const remapAll = useCallback(() => {
    commit((l) => ({
      ...l,
      zones: l.zones.map((z) => (z.part ? { ...z, part: { ...z.part, mapped: false } } : z)),
    }));
    setSelection([]);
    setOnboarding({ scope: null, again: true });
  }, [commit]);

  /** One device again, from its row. */
  const remapOne = useCallback(
    (index: number) => {
      const z = draft?.zones[index];
      if (!z) return;
      const scope: Scope = { deviceId: z.device_id, zoneIndex: z.zone_index };
      commit((l) => ({
        ...l,
        zones: l.zones.map((p) =>
          p.device_id === scope.deviceId && p.zone_index === scope.zoneIndex && p.part
            ? { ...p, part: { ...p.part, mapped: false } }
            : p,
        ),
      }));
      setSelection([]);
      setOnboarding({ scope, again: true });
    },
    [commit, draft],
  );

  // The onboarding opens by itself, once per page load: while the daemon says it is not finished
  // (and the owner has not said "Set up later" in this launch), or when the owner came here to
  // map the PC again (from Settings).
  const status = useSnap((st) => st.status);
  const newLights = useNewLights(devices, status?.onboarding_complete === true);
  const decided = useRef(false);
  useEffect(() => {
    if (decided.current || !draft || !devices || !status) return;
    decided.current = true;
    let again = false;
    try {
      const q = new URLSearchParams(window.location.search).get("map") === "1";
      const kept = window.sessionStorage.getItem("canvas-rgb.onboarding-remap") === "1";
      window.sessionStorage.removeItem("canvas-rgb.onboarding-remap");
      again = q || kept;
      if (q) window.history.replaceState(null, "", window.location.pathname);
    } catch {
      // Not asked for; the automatic rule decides.
    }
    if (again) remapAll();
    // A one-time decision when the layout first arrives, so it is made in this effect.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    else if (shouldOpen(status)) setOnboarding({ scope: null, again: false });
  }, [draft, devices, status, remapAll]);

  /** Closes the flow, and saves what is unsaved. */
  const closeOnboarding = () => {
    stopIdentify();
    setLitLed(null);
    setSelection([]);
    setOnboarding(null);
    if (dirty && !saving) void save();
  };

  /** "Set up later": the window stays shut until the next app start. */
  const laterOnboarding = () => {
    putOffUntilNextLaunch();
    closeOnboarding();
  };

  /** The case box the flow sizes: made at a typical mid-size tower when the layout has none. */
  const ensureCase = () => {
    const lms = draft?.landmarks ?? [];
    let i = lms.findIndex((l) => l.kind === "case");
    if (i < 0) {
      const size = PART_SIZE.case;
      commit((l) => ({
        ...l,
        landmarks: [
          ...(l.landmarks ?? []),
          { kind: "case", x: 100, y: 27, width: size.width, height: size.height, rotation_deg: 0, facing: "viewer", visible: true },
        ],
      }));
      i = lms.length;
    }
    setSelection([{ kind: "landmark", index: i }]);
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

  // Picking a part reveals its inspector: on a phone the two are a screen apart. Not while the
  // part is held: the page scrolling under a drag would carry the part away from the finger.
  const revealedFor = useRef<Sel[] | null>(null);
  useEffect(() => {
    if (!phone || selection.length === 0) {
      revealedFor.current = null;
      return;
    }
    if (dragging || revealedFor.current === selection) return;
    revealedFor.current = selection;
    reveal(inspectorRef.current);
  }, [phone, selection, reveal, dragging]);

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
      <div className="rd-page rd-layout">
        <PageTitle title="Layout" />
        <LoadFrame
          state={
            retrying
              ? "loading"
              : layoutsError
                ? "error"
                : layoutsLoaded && !server
                  ? "empty"
                  : "loading"
          }
          shape={<LayoutShape />}
          label="Reading the layout"
          error={
            <ErrorPanel
              title="The layout could not be read"
              summary="canvas-rgb could not read the layout. Nothing was changed."
              message={layoutsError}
              onRetry={() => void retryLayouts()}
            />
          }
          empty={
            <Panel>
              <EmptyState icon={<SquaresFour size={24} />} title="There is no layout yet">
                Make one here: the desk canvas is this page, and nothing can run until it has a
                zone in it.
              </EmptyState>
            </Panel>
          }
        />
      </div>
    );
  }

  const renderStage = (
    pulse: Sel[] = [],
    onSelectStage: (sel: Sel[]) => void = select,
    extra: {
      spotlight?: Sel[] | null;
      onHoverPart?: (sel: Sel | null) => void;
      guides?: CaseGuide[];
      unlockBadge?: boolean;
    } = {},
  ) => (
    <PreviewStage
      zones={zones}
      landmarks={landmarks}
      devices={devices}
      selection={selection}
      onSelect={onSelectStage}
      onChangeZones={changeZones}
      onChangeLandmark={changeLandmark}
      onChangeParts={changeParts}
      onDrop={onDropUnplaced}
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
      mode={mode}
      onMode={setMode}
      unit={unit}
      caseForm={draft.case_form}
      pulse={pulse}
      litLed={litLed}
      spotlight={extra.spotlight ?? null}
      onHoverPart={extra.onHoverPart}
      guides={extra.guides}
      unlockBadge={extra.unlockBadge}
    />
  );

  if (onboarding) {
    return (
      <div className="onboarding-page">
        <Onboarding
          layout={draft}
          devices={devices}
          scope={onboarding.scope}
          again={onboarding.again}
          unit={unit}
          onUnit={setUnit}
          selection={selection}
          canvas={({ pulse, onSelect, ...extra }) => renderStage(pulse, onSelect, extra)}
          onChange={(fn) => commit(fn)}
          onSelect={select}
          onEnsureCase={ensureCase}
          onAddPart={addLandmark}
          onChase={(index) => startIdentify(index)}
          onFlash={(index) => startIdentify(index, "whole")}
          onLit={setLitLed}
          onLater={laterOnboarding}
          onFinished={closeOnboarding}
          saveError={saveError}
        />
      </div>
    );
  }

  return (
    <div className="rd-page rd-layout">
      {/* The page keeps one name in every state: the layout's own name stands beside it once
          it is known. */}
      <PageTitle title="Layout" context={draft.name} />
      <PageActions>
        <Button variant="ghost" icon={<CompassTool size={16} />} onClick={remapAll}>
          Map my PC again
        </Button>
      </PageActions>

      {layoutsError && (
        // A refresh that failed after the layout was read does not take the page away: the desk
        // stays and the page says it may be old. The raw words are behind the disclosure.
        <div className="rule rule-warn" role="status">
          The layout may be out of date.
          <Details>{layoutsError}</Details>
        </div>
      )}

      {newLights.fresh.length > 0 && (
        <div className="new-light" role="status">
          <span className="new-light-dot" aria-hidden="true" />
          <span className="new-light-text">
            <strong>
              {newLights.fresh.length} new {newLights.fresh.length === 1 ? "light" : "lights"}
            </strong>
            <span className="mute one-line">{newLights.fresh.map((d) => d.name).join(", ")}</span>
          </span>
          <Button
            size="sm"
            onClick={() => {
              const d = newLights.fresh[0];
              const z = d?.zones.find((x) => x.led_count > 0);
              if (d && z) {
                place(d.id, z.index);
                newLights.dismiss(d.id);
              }
            }}
          >
            Place it
          </Button>
          <IconButton
            label="Dismiss"
            size="sm"
            icon={<X size={14} />}
            onClick={() => newLights.fresh.forEach((d) => newLights.dismiss(d.id))}
          />
        </div>
      )}

      <div className="layout-grid">
        <div className="layout-stage-col stack">
          <div className="layout-canvas" ref={stageRef}>
            {renderStage()}
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
            <Panel
              title={selectedZone ? "Zone" : selectedLandmark ? "Part" : "Parts"}
              actions={
                <span className="row-wrap" style={{ alignItems: "center", gap: "var(--s2)" }}>
                  <Segmented<LengthUnit>
                    label="The unit every size is read in"
                    value={unit}
                    options={[
                      { value: "cm", label: "cm" },
                      { value: "in", label: "in" },
                    ]}
                    onChange={setUnit}
                  />
                  <span ref={addRef} className="row-wrap" style={{ alignItems: "center" }}>
                    <Button
                      size="sm"
                      variant="ghost"
                      icon={<Plus size={14} />}
                      onClick={() => setAdding(true)}
                    >
                      Add part
                    </Button>
                  </span>
                  <Popover
                    anchor={addRef}
                    open={adding}
                    onClose={() => setAdding(false)}
                    label="Add a part to the desk"
                  >
                    <div className={desk.nowPicker}>
                      {LANDMARK_KINDS.map((kind) => (
                        <Chip
                          key={kind}
                          label={`Add a ${PART_LABEL[kind]}`}
                          onClick={() => {
                            setAdding(false);
                            addLandmark(kind);
                          }}
                        >
                          <span className={desk.nowPickerName}>{PART_LABEL[kind]}</span>
                        </Chip>
                      ))}
                    </div>
                  </Popover>
                </span>
              }
            >
              {shownZone && primary?.kind === "zone" ? (
                <div ref={inspectorRef}>
                  <ZoneInspector
                    key={`zone-${primary.index}`}
                    zone={shownZone}
                    unit={unit}
                    name={selectedInfo ? selectedPlain.name : staleDeviceName(shownZone.device_id, devices)}
                    icon={selectedInfo ? <KindIcon kind={selectedPlain.kind} fan={selectedPlain.fan} size={18} /> : null}
                    deviceName={
                      selectedInfo
                        ? selectedPlain.hardware && selectedPlain.hardware !== selectedInfo.device.name
                          ? `${selectedPlain.hardware} · ${selectedInfo.device.name}`
                          : selectedInfo.device.name
                        : shortDeviceId(shownZone.device_id)
                    }
                    ledCount={
                      shownZone.leds ? shownZone.leds.count : (selectedInfo?.zone.led_count ?? 0)
                    }
                    zoneCount={selectedInfo?.zone.led_count ?? 0}
                    info={selectedInfo?.zone ?? null}
                    identifying={identifying === primary.index}
                    onIdentify={() => (identifying === primary.index ? stopIdentify() : startIdentify(primary.index))}
                    missing={!selectedInfo}
                    onChange={(patch) => changeZones([{ index: primary.index, patch }])}
                    onRemove={() => unplace(primary.index)}
                    locked={shownZone.locked === true}
                    onToggleLock={() => changeZones([{ index: primary.index, patch: { locked: !shownZone.locked } }])}
                    onRemap={() => remapOne(primary.index)}
                    onBack={phone ? backToCanvas : null}
                  />
                </div>
              ) : selectedLandmark && primary?.kind === "landmark" ? (
                <div ref={inspectorRef}>
                  <LandmarkInspector
                    key={`lm-${primary.index}`}
                    landmark={selectedLandmark}
                    unit={unit}
                    locked={selectedLandmark.locked === true}
                    onToggleLock={() => changeLandmark(primary.index, { locked: !selectedLandmark.locked })}
                    onChange={(patch) => changeLandmark(primary.index, patch)}
                    onRemove={() => removeLandmark(primary.index)}
                    onBack={phone ? backToCanvas : null}
                  />
                </div>
              ) : null}
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
                  // A split zone: the part owns a run of the zone's LEDs, and the run is what
                  // it has. The parts of one zone are numbered, so they can be told apart.
                  const friendly = placementName(devices, z, zones);
                  const leds = z.leds ? z.leds.count : (info?.zone.led_count ?? 0);
                  return (
                    <RowButton
                      key={zoneKey(z)}
                      current={primary?.kind === "zone" && primary.index === i}
                      label={`Select ${friendly || pn.name}`}
                      onSelect={() => select([{ kind: "zone", index: i }])}
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
                      <span className="fig mute">{info ? <AnimatedNumber value={leds} /> : "—"}</span>
                    </RowButton>
                  );
                })}
                {landmarks.map((l, i) => (
                  <RowButton
                    key={`lm-${i}`}
                    current={primary?.kind === "landmark" && primary.index === i}
                    label={`Select ${l.label?.trim() || PART_LABEL[l.kind]}`}
                    onSelect={() => select([{ kind: "landmark", index: i }])}
                  >
                    <span className="row-title">{l.label?.trim() || PART_LABEL[l.kind]}</span>
                    <span className="row-sub one-line">no lights of its own</span>
                    <span className="fig mute">{formatUnits(l.width, unit)}</span>
                  </RowButton>
                ))}
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
  unit,
  name,
  icon,
  deviceName,
  ledCount,
  zoneCount,
  info,
  identifying,
  onIdentify,
  missing,
  onChange,
  onRemove,
  locked,
  onToggleLock,
  onRemap,
  onBack,
}: {
  zone: PlacedZone;
  unit: LengthUnit;
  name: string;
  icon: ReactNode;
  deviceName: string;
  ledCount: number;
  /** The whole zone's count, when this part owns only a run of it. */
  zoneCount: number;
  info: ZoneInfo | null;
  identifying: boolean;
  onIdentify: () => void;
  missing: boolean;
  onChange: (patch: Partial<PlacedZone>) => void;
  onRemove: () => void;
  locked: boolean;
  onToggleLock: () => void;
  /** Maps this one device again, with the onboarding limited to it. */
  onRemap: () => void;
  /** On a phone: the way back to the canvas the zone was picked on. */
  onBack: (() => void) | null;
}) {
  const shapeType = zone.shape.type;
  const resolved = info ? resolveShape(zone.shape, info.kind) : null;
  const kind = zone.part?.kind ?? "generic";
  const facing = zone.part?.facing ?? "viewer";
  const pickShape = (type: ZoneShape["type"]) => {
    if (type === shapeType) return;
    const next: ZoneShape =
      type === "grid"
        ? { type: "grid", columns: Math.max(1, Math.min(ledCount || 1, Math.ceil((ledCount || 1) / 6))) }
        : ({ type } as ZoneShape);
    // One change, so one undo step: the shape and the box that suits it.
    onChange(reshapeBox(zone, next));
  };
  const decimals = unit === "in" ? 2 : 1;
  const toMm = (v: number) => v * 4;
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
        <span className="fig mute">
          {missing ? (
            "not connected"
          ) : (
            <>
              <AnimatedNumber value={ledCount} /> LEDs
              {zoneCount > ledCount ? ` of ${zoneCount}` : ""}
            </>
          )}
        </span>
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
      <Button
        size="sm"
        variant="ghost"
        icon={<CompassTool size={14} />}
        disabled={missing}
        onClick={onRemap}
      >
        Map this one again
      </Button>
      <Field label="Part" hint={<span className="fig">{PART_LABEL[kind]}</span>}>
        <Select
          label="What this part is"
          value={kind}
          options={(Object.keys(PART_LABEL) as PartKind[]).map((k) => ({ value: k, label: PART_LABEL[k] }))}
          onChange={(v) => onChange({ part: { kind: v as PartKind, facing, mapped: zone.part?.mapped ?? false } })}
        />
      </Field>
      <Field label="Facing" hint={<span className="fig">{FACING_LABEL[facing]}</span>}>
        <Select
          label="Which way this part points"
          value={facing}
          options={FACINGS.map((f) => ({ value: f, label: FACING_LABEL[f] }))}
          onChange={(v) => onChange({ part: { kind, facing: v as Facing, mapped: zone.part?.mapped ?? false } })}
        />
      </Field>
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
        <Field label="From the left">
          <NumberField
            label="From the left"
            value={lengthNumber(toMm(zone.x), unit)}
            min={lengthNumber(-CANVAS_W * 4, unit)}
            max={lengthNumber(CANVAS_W * 4, unit)}
            decimals={decimals}
            unit={unitWord(unit)}
            onCommit={(v) => onChange({ x: mmFromLength(v, unit) / 4 })}
          />
        </Field>
        <Field label="From the top">
          <NumberField
            label="From the top"
            value={lengthNumber(toMm(zone.y), unit)}
            min={lengthNumber(-CANVAS_H * 4, unit)}
            max={lengthNumber(CANVAS_H * 4, unit)}
            decimals={decimals}
            unit={unitWord(unit)}
            onCommit={(v) => onChange({ y: mmFromLength(v, unit) / 4 })}
          />
        </Field>
        <Field label="Width">
          <NumberField
            label="Width"
            value={lengthNumber(toMm(zone.width), unit)}
            min={lengthNumber(8, unit)}
            max={lengthNumber(CANVAS_W * 4, unit)}
            decimals={decimals}
            unit={unitWord(unit)}
            onCommit={(v) => onChange({ width: mmFromLength(v, unit) / 4 })}
          />
        </Field>
        <Field label="Height">
          <NumberField
            label="Height"
            value={lengthNumber(toMm(zone.height), unit)}
            min={lengthNumber(8, unit)}
            max={lengthNumber(CANVAS_H * 4, unit)}
            decimals={decimals}
            unit={unitWord(unit)}
            onCommit={(v) => onChange({ height: mmFromLength(v, unit) / 4 })}
          />
        </Field>
      </div>
      <p className={desk.plainNote} style={{ marginTop: 0 }}>
        The frame is {formatUnits(320, unit)} by {formatUnits(200, unit)}. A 120 mm fan is {formatUnits(30, unit)}.
      </p>
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
          {!missing && zoneCount > ledCount ? ` of ${zoneCount}` : ""}
        </span>
      </Field>
      <Field label="Reverse direction">
        <Toggle label="Reverse direction" checked={zone.reversed} onChange={(v) => onChange({ reversed: v })} />
      </Field>
      <Field label="Lock in place">
        <Toggle label="Lock in place" checked={locked} onChange={onToggleLock} />
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

/**
 * The inspector of a landmark: a part with no lights of its own. The case box, a CPU, a
 * graphics card without RGB, a plain box. Its size is in the reader's unit, like everything
 * else.
 */
function LandmarkInspector({
  landmark,
  unit,
  locked,
  onToggleLock,
  onChange,
  onRemove,
  onBack,
}: {
  landmark: Landmark;
  unit: LengthUnit;
  locked: boolean;
  onToggleLock: () => void;
  onChange: (patch: Partial<Landmark>) => void;
  onRemove: () => void;
  onBack: (() => void) | null;
}) {
  const decimals = unit === "in" ? 2 : 1;
  const toMm = (v: number) => v * 4;
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
        <span className="one-line">{landmark.label?.trim() || PART_LABEL[landmark.kind]}</span>
        <span className="fig mute">no lights</span>
      </div>
      <p className="row-sub one-line">A landmark is drawn, never sampled and never lit.</p>
      <Field label="Name">
        <TextField
          label="Name"
          value={landmark.label ?? ""}
          placeholder={PART_LABEL[landmark.kind]}
          allowEmpty
          maxLength={32}
          onCommit={(v) => onChange({ label: v.trim() === "" ? undefined : v.trim().slice(0, 32) })}
        />
      </Field>
      <Field label="Kind" hint={<span className="fig">{PART_LABEL[landmark.kind]}</span>}>
        <Select
          label="What this part is"
          value={landmark.kind}
          options={LANDMARK_KINDS.map((k) => ({ value: k, label: PART_LABEL[k] }))}
          onChange={(v) => onChange({ kind: v as PartKind })}
        />
      </Field>
      <Field label="Facing" hint={<span className="fig">{FACING_LABEL[landmark.facing]}</span>}>
        <Select
          label="Which way this part points"
          value={landmark.facing}
          options={FACINGS.map((f) => ({ value: f, label: FACING_LABEL[f] }))}
          onChange={(v) => onChange({ facing: v as Facing })}
        />
      </Field>
      <div className="grid-2" style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
        <Field label="From the left">
          <NumberField
            label="From the left"
            value={lengthNumber(toMm(landmark.x), unit)}
            min={lengthNumber(-CANVAS_W * 4, unit)}
            max={lengthNumber(CANVAS_W * 4, unit)}
            decimals={decimals}
            unit={unitWord(unit)}
            onCommit={(v) => onChange({ x: mmFromLength(v, unit) / 4 })}
          />
        </Field>
        <Field label="From the top">
          <NumberField
            label="From the top"
            value={lengthNumber(toMm(landmark.y), unit)}
            min={lengthNumber(-CANVAS_H * 4, unit)}
            max={lengthNumber(CANVAS_H * 4, unit)}
            decimals={decimals}
            unit={unitWord(unit)}
            onCommit={(v) => onChange({ y: mmFromLength(v, unit) / 4 })}
          />
        </Field>
        <Field label="Width">
          <NumberField
            label="Width"
            value={lengthNumber(toMm(landmark.width), unit)}
            min={lengthNumber(8, unit)}
            max={lengthNumber(CANVAS_W * 4, unit)}
            decimals={decimals}
            unit={unitWord(unit)}
            onCommit={(v) => onChange({ width: mmFromLength(v, unit) / 4 })}
          />
        </Field>
        <Field label="Height">
          <NumberField
            label="Height"
            value={lengthNumber(toMm(landmark.height), unit)}
            min={lengthNumber(8, unit)}
            max={lengthNumber(CANVAS_H * 4, unit)}
            decimals={decimals}
            unit={unitWord(unit)}
            onCommit={(v) => onChange({ height: mmFromLength(v, unit) / 4 })}
          />
        </Field>
      </div>
      <Field label="Lock in place">
        <Toggle label="Lock in place" checked={locked} onChange={onToggleLock} />
      </Field>
      <Field label="Visible">
        <Toggle label="Visible" checked={landmark.visible !== false} onChange={(v) => onChange({ visible: v })} />
      </Field>
      <div className="row-wrap" style={{ justifyContent: "flex-end" }}>
        <Button size="sm" variant="ghost" icon={<X size={14} />} onClick={onRemove}>
          Remove from layout
        </Button>
      </div>
    </div>
  );
}
