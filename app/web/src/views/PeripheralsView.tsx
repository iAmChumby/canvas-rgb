"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import {
  ArrowCounterClockwise,
  BatteryCharging,
  BatteryFull,
  CaretLeft,
  GameController,
  Keyboard,
  Mouse,
} from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Panel, StateDot } from "@/components/Panel";
import { ErrorPanel, LoadFrame, type LoadState } from "@/components/Loading";
import { PeripheralDetailShape, PeripheralListShape } from "@/components/LoadShapes";
import { Toggle } from "@/components/Toggle";
import { Select } from "@/components/Select";
import { ControllerOutline, type PadButton } from "@/components/ControllerOutline";
import { PropertyControls, type Values } from "@/views/PropertyControls";
import { useMediaQuery } from "@/components/MediaQuery";
import { useToast } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { choiceWord, kindWord, nounOf, readStage, type PeripheralRow } from "@/lib/peripherals";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { ReadProgress } from "@/proto/ReadProgress";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Props } from "@/proto/props";

/** The icon each kind of peripheral wears. */
const KIND_GLYPH = {
  mouse: Mouse,
  game_controller: GameController,
  keyboard: Keyboard,
} as const;

/** A label without the profile it belongs to: the panel above it names the profile. */
function plainLabel(label: string): string {
  return label.replace(/ \(profile \d+\)$/, "").replace(/, profile \d+\)$/, ")");
}

/** A schema with only the properties asked for, so a panel can hold part of a section. */
function schemaOf(id: string, name: string, defs: PropertyDef[]): PropertySchema {
  // The panel's title already says which profile: the labels drop the repeat.
  const properties = defs.map((d) => ({ ...d, label: plainLabel(d.label) }));
  return {
    id,
    name,
    description: "",
    needs: { screen: false, audio: false },
    properties,
    presets: [],
  };
}

/** The value a key holds, as a plain thing to print. */
function plain(value: PropValue | undefined): string | number | boolean | null {
  if (!value) return null;
  if (value.type === "number") return value.value;
  if (value.type === "bool") return value.value;
  return value.value;
}

/** True when two values are the same setting. */
function sameValue(a: PropValue | undefined, b: PropValue): boolean {
  if (!a) return false;
  if (a.type !== b.type) return false;
  return a.value === b.value;
}

/** How often the stage of an open read is asked for while the placeholder stands. */
const STAGE_POLL_MS = 300;

/**
 * The last read of each device in this session. A device that is picked again shows these values
 * at once, marked as refreshing, while the new read runs.
 */
const lastRead = new Map<string, PeripheralDetail>();

/**
 * The stage of the read of `id` that is open now, asked for every `STAGE_POLL_MS` while `active`.
 * The daemon answers at once, also while the read of the device is still waiting for it.
 */
function useReadProgress(id: string | null, active: boolean): ReadProgress | null {
  const [seen, setSeen] = useState<{ id: string; progress: ReadProgress } | null>(null);
  useEffect(() => {
    if (!id || !active) return;
    let stop = false;
    const ask = () => {
      api
        .peripheralRead(id)
        .then((progress) => {
          if (!stop) setSeen({ id, progress });
        })
        .catch(() => {
          // A daemon without the route still reads: the stage line stays "Reading settings".
        });
    };
    const first = setTimeout(ask, 100);
    const timer = setInterval(ask, STAGE_POLL_MS);
    return () => {
      stop = true;
      clearTimeout(first);
      clearInterval(timer);
      setSeen(null);
    };
  }, [id, active]);
  return active && seen && seen.id === id ? seen.progress : null;
}

export function PeripheralsView() {
  const [list, setList] = useState<PeripheralRow[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [pickedId, setSelectedId] = useState<string | null>(null);
  // The detail of the device that is picked. It carries the id it belongs to, so a slow answer
  // never lands on the wrong device.
  const [loaded, setLoaded] = useState<{ id: string; detail: PeripheralDetail } | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  // The edits that are not on the device yet.
  const [staged, setStaged] = useState<Props>({});
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [pickedStage, setPickedStage] = useState<number | null>(null);
  const [pickedButton, setPickedButton] = useState<string | null>(null);
  const narrow = useMediaQuery("(max-width: 720px)");
  const toast = useToast();

  // On a wide page the first device is open already; on a phone the list comes first.
  const selectedId = pickedId ?? (narrow ? null : (list?.[0]?.id ?? null));

  // What this device held when it was read: from this page, or from an earlier visit. While the
  // device is read again these stay on screen, marked as refreshing.
  const fresh = loaded && loaded.id === selectedId ? loaded.detail : null;
  const detail = fresh ?? (selectedId ? (lastRead.get(selectedId) ?? null) : null);
  const [reloading, setReloading] = useState(false);
  const reading = selectedId !== null && !detailError && (fresh === null || reloading);
  const progress = useReadProgress(selectedId, reading);

  // Reads one device again: after a write, or after the access changed.
  const loadDetail = useCallback(async (id: string) => {
    setReloading(true);
    try {
      const d = await api.peripheral(id);
      lastRead.set(id, d);
      setLoaded({ id, detail: d });
      setStaged({});
      setSaveError(null);
      setDetailError(null);
    } catch (e) {
      setLoaded(null);
      setDetailError(messageOf(e));
    } finally {
      setReloading(false);
    }
  }, []);

  // Lists the devices. The first run and "Try again" both come through here.
  const loadList = useCallback(() => {
    let stop = false;
    api
      .peripherals()
      .then((r) => {
        if (stop) return;
        setList(r.peripherals as PeripheralRow[]);
        setListError(null);
      })
      .catch((e: unknown) => {
        if (!stop) setListError(messageOf(e));
      });
    return () => {
      stop = true;
    };
  }, []);
  useEffect(() => loadList(), [loadList]);

  useEffect(() => {
    if (!selectedId) return;
    let stop = false;
    api
      .peripheral(selectedId)
      .then((d) => {
        lastRead.set(selectedId, d);
        if (stop) return;
        setLoaded({ id: selectedId, detail: d });
        setStaged({});
        setSaveError(null);
        setDetailError(null);
        setPickedStage(null);
        setPickedButton(null);
      })
      .catch((e: unknown) => {
        if (stop) return;
        setLoaded(null);
        setDetailError(messageOf(e));
      });
    return () => {
      stop = true;
    };
  }, [selectedId]);

  // The values the controls show: what the device holds, with the staged edits on top.
  const values: Values = useMemo(
    () => ({ ...(detail?.values ?? {}), ...staged }) as Values,
    [detail, staged],
  );

  const changed = useMemo(() => {
    const out: Props = {};
    for (const [key, value] of Object.entries(staged)) {
      if (!sameValue(detail?.values[key], value)) out[key] = value;
    }
    return out;
  }, [staged, detail]);

  const onChange = (key: string, value: PropValue) => {
    setStaged((prev) => ({ ...prev, [key]: value }));
    setSaveError(null);
  };

  const apply = async () => {
    if (!detail) return;
    setSaving(true);
    setSaveError(null);
    try {
      await api.patchPeripheral(detail.info.id, changed);
      await loadDetail(detail.info.id);
      toast("The settings are on the device");
    } catch (e) {
      setSaveError(messageOf(e));
      toast(messageOf(e), "down");
    } finally {
      setSaving(false);
    }
  };

  const revert = () => {
    setStaged({});
    setSaveError(null);
  };

  const setAccess = async (next: { reads?: boolean; writes?: boolean }) => {
    if (!detail) return;
    const reads = next.reads ?? detail.access.reads;
    const writes = next.writes ?? detail.access.writes;
    try {
      await api.putPeripheralAccess(detail.info.id, { reads, writes });
      await loadDetail(detail.info.id);
    } catch (e) {
      setSaveError(messageOf(e));
      toast(messageOf(e), "down");
    }
  };

  const rows = list ?? [];
  const noun = nounOf(rows.find((r) => r.id === selectedId)?.kind ?? "mouse");
  // The detail pane is one frame in every state: the placeholder, the error and the empty state
  // are as tall as the panels they stand for, and a device read before keeps its values on
  // screen, marked as refreshing, while it is read again.
  const detailState: LoadState = detailError ? "error" : detail ? "ready" : "loading";
  // With no device to show (the list is empty or could not be read) the list says so, and there is
  // no page to hold a place for.
  const noDetail = listError !== null || (list !== null && rows.length === 0);
  const pickedKind = rows.find((r) => r.id === selectedId)?.kind ?? rows[0]?.kind ?? "mouse";
  const detailPane = noDetail ? null : (
    <LoadFrame
      state={detailState}
      shape={<PeripheralDetailShape kind={pickedKind} />}
      stage={reading ? readStage(progress, selectedId ? noun : "device") : null}
      refreshing={detail !== null && reading}
      label="Reading the device"
      error={
        <ErrorPanel
          title="The device could not be read"
          message={detailError}
          onRetry={() => {
            if (!selectedId) return;
            setDetailError(null);
            void loadDetail(selectedId);
          }}
        />
      }
    >
      {detail && (
        <PeripheralDetail
          detail={detail}
          values={values}
          changedCount={Object.keys(changed).length}
          saving={saving}
          saveError={saveError}
          pickedStage={pickedStage}
          pickedButton={pickedButton}
          onPickStage={setPickedStage}
          onPickButton={setPickedButton}
          onChange={onChange}
          onApply={() => void apply()}
          onRevert={revert}
          onAccess={(next) => void setAccess(next)}
        />
      )}
    </LoadFrame>
  );

  const listState: LoadState = listError ? "error" : list === null ? "loading" : rows.length === 0 ? "empty" : "ready";

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Peripherals</h1>
      </div>

      <div className="peripheral-grid" data-step={narrow && selectedId ? "detail" : "list"}>
        <div className="peripheral-list" hidden={narrow && !!selectedId}>
          <LoadFrame
            className="load-col"
            state={listState}
            shape={<PeripheralListShape />}
            label="Looking for devices"
            error={
              <ErrorPanel
                title="The devices could not be listed"
                message={listError}
                onRetry={() => {
                  setListError(null);
                  loadList();
                }}
              />
            }
            empty={
              <Panel title="No device found">
                <p className="mute">None that canvas-rgb can configure is plugged in.</p>
              </Panel>
            }
          >
            {rows.map((p) => {
              const Glyph = KIND_GLYPH[p.kind];
              const battery = p.battery ?? null;
              return (
                <button
                  key={p.id}
                  type="button"
                  className="peripheral-row"
                  data-on={p.id === selectedId ? "true" : "false"}
                  aria-current={p.id === selectedId ? "true" : undefined}
                  onClick={() => {
                    setDetailError(null);
                    setSelectedId(p.id);
                  }}
                >
                  <span className="peripheral-icon" aria-hidden="true">
                    <Glyph size={18} />
                  </span>
                  <span className="peripheral-text">
                    <span className="peripheral-name">{p.name}</span>
                    <span className="peripheral-sub">
                      {kindWord(p.kind)} · {p.vendor} ·{" "}
                      <StateDot tone={p.connected ? "ok" : "mute"}>
                        {p.connected ? "Connected" : "Not connected"}
                      </StateDot>
                    </span>
                  </span>
                  {battery !== null && (
                    <span className="peripheral-battery fig">
                      {p.charging ? <BatteryCharging size={14} /> : <BatteryFull size={14} />}
                      {battery}%
                    </span>
                  )}
                </button>
              );
            })}
          </LoadFrame>
        </div>

        <div className="peripheral-detail" hidden={narrow && !selectedId}>
          {narrow && selectedId && (
            <div className="peripheral-back">
              <IconButton label="Back to the peripherals" icon={<CaretLeft size={18} />} onClick={() => {
                  // The next pick reads the device again: what it held stays on screen as known.
                  setLoaded(null);
                  setSelectedId(null);
                }}
              />
              <span>Peripherals</span>
            </div>
          )}
          {detailPane}
        </div>
      </div>
    </div>
  );
}

/** The detail of one device: what it is, what may be done with it, and its settings. */
function PeripheralDetail({
  detail,
  values,
  changedCount,
  saving,
  saveError,
  pickedStage,
  pickedButton,
  onPickStage,
  onPickButton,
  onChange,
  onApply,
  onRevert,
  onAccess,
}: {
  detail: PeripheralDetail;
  values: Values;
  changedCount: number;
  saving: boolean;
  saveError: string | null;
  pickedStage: number | null;
  pickedButton: string | null;
  onPickStage: (n: number | null) => void;
  onPickButton: (key: string | null) => void;
  onChange: (key: string, value: PropValue) => void;
  onApply: () => void;
  onRevert: () => void;
  onAccess: (next: { reads?: boolean; writes?: boolean }) => void;
}) {
  const info = detail.info;
  const Glyph = KIND_GLYPH[info.kind];
  const readOnly = detail.schema.read_only;
  const writes = detail.access.writes;
  const reads = detail.access.reads;
  const isController = detail.schema.kind === "game_controller";
  const isMouse = detail.schema.kind === "mouse";

  return (
    <div className="stack">
      <Panel>
        <div className="peripheral-head">
          <span className="peripheral-head-icon" aria-hidden="true">
            <Glyph size={22} />
          </span>
          <div className="peripheral-head-text">
            <h2 className="peripheral-head-name">{info.name}</h2>
            <p className="peripheral-head-sub">
              {kindWord(info.kind)} · {info.vendor} · <span className="fig">{info.location}</span>
            </p>
          </div>
          <StateDot tone={info.connected ? "ok" : "mute"}>{info.connected ? "Connected" : "Not connected"}</StateDot>
        </div>
      </Panel>

      <Panel title="Access">
        <div className="access-list">
          <div className="access-row">
            <span className="access-label">Read settings from this device</span>
            <Toggle
              label="Read settings from this device"
              checked={reads}
              onChange={(v) => onAccess({ reads: v, writes: v ? undefined : false })}
            />
            <p className="access-note">canvas-rgb opens the device and asks for its settings; it changes nothing</p>
          </div>
          <div className="access-row">
            <span className="access-label">Change settings on the device</span>
            <Toggle
              label="Change settings on the device"
              checked={writes}
              disabled={!reads}
              onChange={(v) => onAccess({ writes: v })}
            />
            <p className="access-note">lets canvas-rgb change settings on the device</p>
          </div>
        </div>
        {detail.problem && (
          <p className="rule rule-warn" data-role="problem">
            {detail.problem}
          </p>
        )}
      </Panel>

      {detail.schema.sections.map((section) => {
        if (isMouse && section.id === "dpi") {
          return (
            <DpiPanel
              key={section.id}
              section={section}
              values={values}
              readOnly={readOnly}
              disabled={!writes}
              picked={pickedStage}
              onPick={onPickStage}
              onChange={onChange}
            />
          );
        }
        if (isController && section.id === "buttons") {
          return (
            <ControllerPanel
              key={section.id}
              detail={detail}
              values={values}
              readOnly={readOnly}
              disabled={!writes}
              picked={pickedButton}
              onPick={onPickButton}
              onChange={onChange}
            />
          );
        }
        // One profile shows at a time: the active one.
        if (isController && section.id.startsWith("profile-")) {
          const active = String(plain(values["active-profile"]) ?? "1");
          if (section.id !== `profile-${active}`) return null;
        }
        // The paddles are in the mapping list of the plan; the profile panels hold the rest.
        const properties =
          isController && section.id.startsWith("profile-")
            ? section.properties.filter((p) => !p.key.includes("-paddle-"))
            : section.properties;
        return (
          <Panel key={section.id} title={section.title}>
            <PropertyControls
              schema={schemaOf(`peripheral-${section.id}`, section.title, properties)}
              values={values}
              readOnly={readOnly}
              disabled={!writes}
              optionLabel={choiceWord}
              onChange={onChange}
            />
          </Panel>
        );
      })}

      {isController && (
        <Panel title="RGB rings">
          <p className="mute">The rings on the pad are not read yet: this build cannot name them.</p>
        </Panel>
      )}

      <div className="layout-save">
        <div className="savebar" data-dirty={changedCount > 0 ? "true" : "false"}>
          {!writes ? (
            <span className="mute">Turn on writes to change settings on the device</span>
          ) : (
            <>
              <span className={changedCount > 0 ? "warn-text fig" : "mute"}>
                {changedCount > 0 ? `${changedCount} change${changedCount === 1 ? "" : "s"} not sent` : "On the device"}
              </span>
              <div className="row-wrap">
                <Button
                  variant="ghost"
                  icon={<ArrowCounterClockwise size={16} />}
                  disabled={changedCount === 0 || saving}
                  onClick={onRevert}
                >
                  Revert
                </Button>
                <Button variant="primary" disabled={changedCount === 0 || saving} onClick={onApply}>
                  {saving ? "Sending" : "Apply"}
                </Button>
              </div>
            </>
          )}
          {saveError && <span className="error-line">{saveError}</span>}
        </div>
      </div>
    </div>
  );
}

/** The mouse's DPI section: the stages as chips, and the picked stage's two settings. */
function DpiPanel({
  section,
  values,
  readOnly,
  disabled,
  picked,
  onPick,
  onChange,
}: {
  section: PeripheralDetail["schema"]["sections"][number];
  values: Values;
  readOnly: string[];
  disabled: boolean;
  picked: number | null;
  onPick: (n: number | null) => void;
  onChange: (key: string, value: PropValue) => void;
}) {
  const count = Number(plain(values["dpi-stage-count"]) ?? 6);
  const active = String(plain(values["dpi-active-stage"]) ?? "1");
  const stages: { n: number; dpi: number; colour: string }[] = [];
  for (let i = 1; i <= count; i++) {
    stages.push({
      n: i,
      dpi: Number(plain(values[`dpi-stage-${i}`]) ?? 0),
      colour: String(plain(values[`dpi-stage-${i}-colour`]) ?? "#ffffff"),
    });
  }
  const others = section.properties.filter((p) => !p.key.startsWith("dpi-stage-"));
  const pickedDefs = picked
    ? section.properties.filter(
        (p) => p.key === `dpi-stage-${picked}` || p.key === `dpi-stage-${picked}-colour`,
      )
    : [];

  return (
    <Panel title={section.title}>
      <PropertyControls
        schema={schemaOf("dpi-count", "DPI", others)}
        values={values}
        readOnly={readOnly}
        disabled={disabled}
        optionLabel={choiceWord}
        onChange={onChange}
      />
      <div className="dpi-stages" role="group" aria-label="DPI stages">
        {stages.map((s) => (
          <button
            key={s.n}
            type="button"
            className="dpi-chip"
            data-on={picked === s.n ? "true" : "false"}
            data-active={active === String(s.n) ? "true" : "false"}
            aria-pressed={picked === s.n}
            aria-label={`DPI stage ${s.n}, ${s.dpi}${active === String(s.n) ? ", the active stage" : ""}`}
            onClick={() => onPick(picked === s.n ? null : s.n)}
          >
            <span className="dpi-chip-dot" style={{ "--c": s.colour } as CSSProperties} aria-hidden="true" />
            <span className="fig">{s.dpi}</span>
          </button>
        ))}
      </div>
      {picked !== null && pickedDefs.length > 0 && (
        <div className="dpi-stage-edit">
          <PropertyControls
            schema={schemaOf(`dpi-stage-${picked}`, `DPI stage ${picked}`, pickedDefs)}
            values={values}
            readOnly={readOnly}
            disabled={disabled}
            onChange={onChange}
          />
        </div>
      )}
    </Panel>
  );
}

/** The controller: the plan of the pad, and the mapping of every button. */
function ControllerPanel({
  detail,
  values,
  readOnly,
  disabled,
  picked,
  onPick,
  onChange,
}: {
  detail: PeripheralDetail;
  values: Values;
  readOnly: string[];
  disabled: boolean;
  picked: string | null;
  onPick: (key: string | null) => void;
  onChange: (key: string, value: PropValue) => void;
}) {
  const buttons = detail.schema.sections.find((s) => s.id === "buttons");
  const profile = String(plain(values["active-profile"]) ?? "1");
  const profileSection = detail.schema.sections.find((s) => s.id === `profile-${profile}`);
  const paddles = (profileSection?.properties ?? []).filter((p) => p.key.includes("-paddle-"));
  const rows: PropertyDef[] = [...(buttons?.properties ?? []), ...paddles];
  const rowRefs = useRef(new Map<string, HTMLDivElement | null>());

  const planButtons: PadButton[] = rows.map((p) => ({
    key: p.key,
    label: p.label,
    print: p.label.length <= 5 ? p.label : undefined,
  }));

  const pick = (key: string) => {
    onPick(key);
    const row = rowRefs.current.get(key);
    row?.scrollIntoView({ block: "nearest" });
    // The mapping opens where the button is, so the reader sees the two together.
    row?.querySelector<HTMLButtonElement>(".select-trigger")?.click();
  };

  return (
    <Panel title={detail.info.name}>
      <div className="pad-grid">
        <div className="pad-plan-wrap">
          <ControllerOutline buttons={planButtons} selected={picked} onPick={pick} />
        </div>
        <div className="pad-map">
          {rows.map((p) => {
            const options =
              p.kind.type === "choice"
                ? p.kind.options.map((o) => ({ value: o, label: choiceWord(o) }))
                : [];
            const current = String(plain(values[p.key]) ?? "");
            const fixed = readOnly.includes(p.key);
            return (
              <div
                key={p.key}
                className="pad-map-row"
                data-on={picked === p.key ? "true" : "false"}
                ref={(el) => {
                  if (el) rowRefs.current.set(p.key, el);
                  else rowRefs.current.delete(p.key);
                }}
              >
                <span className="pad-map-label">{plainLabel(p.label)}</span>
                {fixed ? (
                  <span className="read-only">{choiceWord(current)}</span>
                ) : (
                  <Select
                    label={`${plainLabel(p.label)} mapping`}
                    value={current}
                    options={options}
                    disabled={disabled}
                    onChange={(v) => onChange(p.key, { type: "choice", value: v })}
                  />
                )}
              </div>
            );
          })}
        </div>
      </div>
    </Panel>
  );
}
