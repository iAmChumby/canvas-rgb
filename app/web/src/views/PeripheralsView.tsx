"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import {
  ArrowCounterClockwise,
  ArrowsClockwise,
  BatteryCharging,
  BatteryFull,
  GameController,
  Keyboard,
  Mouse,
} from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Panel, StateDot } from "@/components/Panel";
import { ErrorPanel, LoadFrame, type LoadState } from "@/components/Loading";
import { PeripheralDetailShape, PeripheralListShape } from "@/components/LoadShapes";
import { Toggle } from "@/components/Toggle";
import { Select } from "@/components/Select";
import { ControllerOutline, type PadButton } from "@/components/ControllerOutline";
import { PropertyControls, type Values } from "@/views/PropertyControls";
import { notify } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { choiceWord, kindWord, nounOf, readStage, type PeripheralRow } from "@/lib/peripherals";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { ReadProgress } from "@/proto/ReadProgress";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Props } from "@/proto/props";
import { PageActions, PageTitle } from "@/shell/PageHeader";
import { Tooltip } from "@/components/Tooltip";

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

/** The entries of `prev` whose device key is in `live`; `prev` itself when nothing goes. */
function keepLive<T>(prev: Record<string, T>, live: Set<string>): Record<string, T> {
  const kept = Object.entries(prev).filter(([id]) => live.has(id));
  return kept.length === Object.keys(prev).length ? prev : Object.fromEntries(kept);
}

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
  // The change that is on the device already, and whose refresh failed: the page says the
  // change was saved, because it was.
  const [changeSaved, setChangeSaved] = useState(false);
  // The edits that are not on the device yet, per device key: an edit made on one device is
  // never shown on, or sent to, another (#156).
  const [stagedBy, setStagedBy] = useState<Record<string, Props>>({});
  const setStagedFor = useCallback((id: string, next: Props | ((prev: Props) => Props)) => {
    setStagedBy((prev) => {
      const value = typeof next === "function" ? next(prev[id] ?? {}) : next;
      const out = { ...prev };
      if (Object.keys(value).length > 0) out[id] = value;
      else delete out[id];
      return out;
    });
  }, []);
  const [saving, setSaving] = useState(false);
  // A failed write's message, per device key: one device's read must not clear another
  // device's failure (#156). Only the selected device's own message is shown.
  const [saveErrors, setSaveErrors] = useState<Record<string, string>>({});
  const setDeviceSaveError = useCallback((id: string, message: string | null) => {
    setSaveErrors((prev) => {
      const next = { ...prev };
      if (message) next[id] = message;
      else delete next[id];
      return next;
    });
  }, []);
  const [pickedStage, setPickedStage] = useState<number | null>(null);
  const [pickedButton, setPickedButton] = useState<string | null>(null);

  // The first device is open already: the keys are the way to the others.
  const selectedId = pickedId ?? (list?.[0]?.id ?? null);
  // This device's own failed write's message, if any.
  const saveError = selectedId ? (saveErrors[selectedId] ?? null) : null;
  const staged = useMemo(() => (selectedId ? (stagedBy[selectedId] ?? {}) : {}), [stagedBy, selectedId]);

  // What this device held when it was read: from this page, or from an earlier visit. While the
  // device is read again these stay on screen, marked as refreshing.
  const fresh = loaded && loaded.id === selectedId ? loaded.detail : null;
  const detail = fresh ?? (selectedId ? (lastRead.get(selectedId) ?? null) : null);
  // The device whose reload is out. It is an id, not a flag: a reload of one device that is
  // still out when another is picked marks only the device it belongs to.
  const [reloadingId, setReloadingId] = useState<string | null>(null);
  const reloading = reloadingId !== null && reloadingId === selectedId;
  const reading = selectedId !== null && !detailError && (fresh === null || reloading);
  const progress = useReadProgress(selectedId, reading);

  // The number of the read that is on its way. An answer that is older than the newest one is
  // dropped: a slow reload of one device must not put its error on another.
  const detailRequest = useRef(0);

  // A Read again that is out: a second press while it runs is the same read, not another one.
  const rereading = useRef(false);

  // Per device: the number of edits the user has made, and that number when a write to the
  // device last failed. A read that lands keeps the device's unsent edits unless a write failed
  // after the last of them: then the edit the failed write carried goes, so the controls show
  // what the device holds (#151). Unsent edits otherwise survive reads and device switches
  // (#156); a write that lands takes off only the values it sent. A read the page makes never clears a write's message: it knows what the
  // device holds, not how the write ended. The next attempt, an edit, Revert or the user's own
  // Read again clears it.
  const editCount = useRef<Record<string, number>>({});
  const editsAtFailure = useRef<Record<string, number>>({});
  const editsOf = (id: string) => editCount.current[id] ?? 0;
  const keepsStaged = (id: string) => editsOf(id) > (editsAtFailure.current[id] ?? 0);
  // Per device: the number of writes that failed. A Read again clears the message only when it
  // started after the last failure (#151).
  const failures = useRef<Record<string, number>>({});
  const failuresOf = (id: string) => failures.current[id] ?? 0;
  // `carriedEdits`: the failed write sent the staged edits (Apply). An access change that
  // fails carried none of them, so it leaves them staged.
  const noteFailure = (id: string, carriedEdits: boolean) => {
    if (carriedEdits) editsAtFailure.current[id] = editsOf(id);
    failures.current[id] = failuresOf(id) + 1;
  };

  // Asks for one device's detail again: after a write, or after the access changed (the daemon
  // answers those from the values it keeps), or on the Read again key (`forceRead`: the daemon
  // reads the device). `landed` says that the change is already on the device, so a failed
  // refresh is not a failed change.
  const loadDetail = useCallback(async (id: string, landed = false, forceRead = false) => {
    if (forceRead) {
      if (rereading.current) return;
      rereading.current = true;
    }
    const n = ++detailRequest.current;
    const failed = failuresOf(id);
    setReloadingId(id);
    try {
      const d = forceRead ? await api.peripheral(id, { fresh: true }) : await api.peripheral(id);
      if (n !== detailRequest.current) return;
      lastRead.set(id, d);
      setLoaded({ id, detail: d });
      if (!keepsStaged(id)) setStagedFor(id, {});
      // Read again is the user's own new action on this device: it clears the device's failed
      // write's message, when it started after that failure (#151). No other read does.
      if (forceRead && failuresOf(id) === failed) setDeviceSaveError(id, null);
      setDetailError(null);
      setChangeSaved(false);
    } catch (e) {
      if (n !== detailRequest.current) return;
      setLoaded(null);
      setDetailError(messageOf(e));
      setChangeSaved(landed);
    } finally {
      if (forceRead) rereading.current = false;
      if (n === detailRequest.current) setReloadingId(null);
    }
    // editsOf / keepsStaged read refs only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [setStagedFor, setDeviceSaveError]);

  // Lists the devices. The first run and "Try again" both come through here. The stop handle is
  // kept, so a retry that is still running when the page goes away is dropped with it.
  const listStop = useRef<(() => void) | null>(null);
  const [listBusy, setListBusy] = useState(false);

  const loadList = useCallback(() => {
    let stop = false;
    const handle = () => {
      stop = true;
    };
    listStop.current?.();
    listStop.current = handle;
    api
      .peripherals()
      .then((r) => {
        if (stop) return;
        const rows = r.peripherals as PeripheralRow[];
        setList(rows);
        setListError(null);
        // A device that is gone takes its unsent edits and its message with it, so they do not
        // come back if a device with the same key shows up again.
        const live = new Set(rows.map((row) => row.id));
        setStagedBy((prev) => keepLive(prev, live));
        setSaveErrors((prev) => keepLive(prev, live));
      })
      .catch((e: unknown) => {
        if (!stop) setListError(messageOf(e));
      })
      .finally(() => {
        if (!stop) setListBusy(false);
      });
    return handle;
  }, []);
  useEffect(() => loadList(), [loadList]);

  // The retry: the frame goes back to its placeholder while the list is asked for again, so the
  // button is gone and a second press cannot start a second load.
  const retryList = () => {
    if (listBusy) return;
    setListError(null);
    setListBusy(true);
    loadList();
  };

  useEffect(() => {
    if (!selectedId) return;
    // The read of a device that is picked again. The counter drops an answer that belongs to
    // a device that is no longer the one on screen: a slow reload of one device must not put
    // its error on another.
    const n = ++detailRequest.current;
    const id = selectedId;
    let stop = false;
    api
      .peripheral(id)
      .then((d) => {
        if (stop || n !== detailRequest.current) return;
        // This read is newer than any reload that was out: that reload is dropped when it
        // lands, so its marker is cleared here, or the device would stay "refreshing" for good.
        setReloadingId(null);
        lastRead.set(id, d);
        setLoaded({ id, detail: d });
        if (!keepsStaged(id)) setStagedFor(id, {});
        setDetailError(null);
        setChangeSaved(false);
      })
      .catch((e: unknown) => {
        if (stop || n !== detailRequest.current) return;
        setReloadingId(null);
        setLoaded(null);
        setDetailError(messageOf(e));
        setChangeSaved(false);
      });
    return () => {
      stop = true;
    };
    // editsOf / keepsStaged read refs only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
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

  // The edits that are not on the device yet: they are the ones the strip counts.
  const changedCount = Object.keys(changed).length;

  const onChange = (key: string, value: PropValue) => {
    if (!selectedId) return;
    editCount.current[selectedId] = editsOf(selectedId) + 1;
    setStagedFor(selectedId, (prev) => ({ ...prev, [key]: value }));
    // An edit replaces what the failed write asked for: its message goes.
    setDeviceSaveError(selectedId, null);
  };

  const apply = async () => {
    if (!detail) return;
    setSaving(true);
    const id = detail.info.id;
    const sent = changed;
    setDeviceSaveError(id, null);
    try {
      await api.patchPeripheral(id, sent);
    } catch (e) {
      // The change itself failed: nothing was changed.
      noteFailure(detail.info.id, true);
      setDeviceSaveError(detail.info.id, messageOf(e));
      setSaving(false);
      return;
    }
    // The change is on the device: the values it sent are no longer staged. An edit made while
    // it was out (a new value, or another setting) stays staged.
    setStagedFor(id, (prev) => {
      const out: Props = {};
      for (const [key, value] of Object.entries(prev)) {
        if (!(key in sent) || !sameValue(sent[key], value)) out[key] = value;
      }
      return out;
    });
    // The read that follows only refreshes the page: when it fails, the change is still saved.
    await loadDetail(id, true);
    notify("The settings are on the device");
    setSaving(false);
  };

  const revert = () => {
    if (!selectedId) return;
    setStagedFor(selectedId, {});
    setDeviceSaveError(selectedId, null);
  };

  const setAccess = async (next: { reads?: boolean; writes?: boolean }) => {
    if (!detail) return;
    const reads = next.reads ?? detail.access.reads;
    const writes = next.writes ?? detail.access.writes;
    // A new attempt: the last failure's message goes.
    setDeviceSaveError(detail.info.id, null);
    try {
      await api.putPeripheralAccess(detail.info.id, { reads, writes });
    } catch (e) {
      // The change itself failed: nothing was changed.
      noteFailure(detail.info.id, false);
      setDeviceSaveError(detail.info.id, messageOf(e));
      return;
    }
    // The access is saved. The read that follows only refreshes the page.
    await loadDetail(detail.info.id, true);
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
          summary={
            changeSaved
              ? // The change is on the device; only the page could not be refreshed.
                "The change was saved, but the page could not be refreshed."
              : "canvas-rgb could not read that device. Nothing was changed."
          }
          message={detailError}
          onRetry={() => {
            if (!selectedId) return;
            setDetailError(null);
            setChangeSaved(false);
            void loadDetail(selectedId);
          }}
        />
      }
    >
      {detail && (
        <PeripheralDetail
          detail={detail}
          values={values}
          pickedStage={pickedStage}
          pickedButton={pickedButton}
          onPickStage={setPickedStage}
          onPickButton={setPickedButton}
          onChange={onChange}
          onAccess={(next) => void setAccess(next)}
          reading={reading}
          onReadAgain={() => selectedId && void loadDetail(selectedId, false, true)}
        />
      )}
    </LoadFrame>
  );

  const listState: LoadState =
    listError && !listBusy ? "error" : list === null ? "loading" : rows.length === 0 ? "empty" : "ready";

  return (
    <div className="rd-page rd-peripherals">
      <PageTitle title="Peripherals" context={<>{rows.length} found</>} />
      {/* The pending changes and the way to send them sit in the strip, beside the save (R4). */}
      <PageActions>
        {changedCount > 0 && (
          <>
            <span className="warn-text fig rd-peri-pending">
              {changedCount} change{changedCount === 1 ? "" : "s"} not sent
            </span>
            <Button
              variant="ghost"
              size="sm"
              icon={<ArrowCounterClockwise size={16} />}
              disabled={saving}
              aria-label="Revert"
              onClick={revert}
            >
              <span className="rd-peri-revert-label">Revert</span>
            </Button>
            <Button variant="primary" size="sm" disabled={saving} onClick={() => void apply()}>
              {saving ? "Sending" : "Apply"}
            </Button>
          </>
        )}
      </PageActions>

      {/* The device column becomes a row of keys: one click, as now. */}
      <div className="rd-peri-keys">        <LoadFrame
          className="load-col"
          state={listState}
          shape={<PeripheralListShape />}
          label="Looking for devices"
          error={
            <ErrorPanel
              title="The peripherals could not be listed"
              summary="canvas-rgb could not look for the peripherals. Nothing was changed."
              message={listError}
              onRetry={retryList}
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
                className="peripheral-row rd-devkey"
                data-on={p.id === selectedId ? "true" : "false"}
                aria-current={p.id === selectedId ? "true" : undefined}
                onClick={() => {
                  setDetailError(null);
                  setChangeSaved(false);
                  // A new device: the stage and the button that were picked belong to the
                  // one that was on screen.
                  setPickedStage(null);
                  setPickedButton(null);
                  setSelectedId(p.id);
                }}
              >
                <span className="peripheral-icon" aria-hidden="true">
                  <Glyph size={18} />
                </span>
                <span className="peripheral-text">
                  <span className="peripheral-name">{p.name}</span>
                </span>
                {battery !== null && (
                  <span className="peripheral-battery fig">
                    {p.charging ? <BatteryCharging size={14} /> : <BatteryFull size={14} />}
                    {battery}%
                  </span>
                )}
                <span
                  className="state-dot"
                  style={{ "--tone": p.connected ? "var(--ok)" : "var(--mute)" } as CSSProperties}
                  aria-hidden="true"
                />
              </button>
            );
          })}
        </LoadFrame>
      </div>

      {saveError && (
        <p className="rule rule-warn" role="status">
          <span>{saveError}</span>
        </p>
      )}

      <div className="peripheral-detail">
        {detailPane}
      </div>
    </div>
  );
}

/** The detail of one device: what it is, what may be done with it, and its settings. */
function PeripheralDetail({
  detail,
  values,
  pickedStage,
  pickedButton,
  onPickStage,
  onPickButton,
  onChange,
  onAccess,
  reading,
  onReadAgain,
}: {
  reading: boolean;
  onReadAgain: () => void;
  detail: PeripheralDetail;
  values: Values;
  pickedStage: number | null;
  pickedButton: string | null;
  onPickStage: (n: number | null) => void;
  onPickButton: (key: string | null) => void;
  onChange: (key: string, value: PropValue) => void;
  onAccess: (next: { reads?: boolean; writes?: boolean }) => void;
}) {
  const info = detail.info;
  const Glyph = KIND_GLYPH[info.kind];
  const readOnly = detail.schema.read_only;
  const writes = detail.access.writes;
  const reads = detail.access.reads;
  const isController = detail.schema.kind === "game_controller";
  const isMouse = detail.schema.kind === "mouse";

  // The panels that stand in columns, and the one that spans them all.
  const columnPanels: ReactNode[] = [];
  const widePanels: ReactNode[] = [];

  for (const section of detail.schema.sections) {
    if (isMouse && section.id === "dpi") {
      columnPanels.push(
        <DpiPanel
          key={section.id}
          section={section}
          values={values}
          readOnly={readOnly}
          disabled={!writes}
          picked={pickedStage}
          onPick={onPickStage}
          onChange={onChange}
        />,
      );
      continue;
    }
    if (isController && section.id === "buttons") {
      // The outline of the pad and the mapping of its buttons belong together, and they are
      // wider than a column: they span them all.
      widePanels.push(
        <ControllerPanel
          key={section.id}
          detail={detail}
          values={values}
          readOnly={readOnly}
          disabled={!writes}
          picked={pickedButton}
          onPick={onPickButton}
          onChange={onChange}
        />,
      );
      continue;
    }
    // One profile shows at a time: the active one.
    if (isController && section.id.startsWith("profile-")) {
      const active = String(plain(values["active-profile"]) ?? "1");
      if (section.id !== `profile-${active}`) continue;
    }
    // The paddles are in the mapping list of the plan; the profile panels hold the rest.
    const properties =
      isController && section.id.startsWith("profile-")
        ? section.properties.filter((p) => !p.key.includes("-paddle-"))
        : section.properties;
    columnPanels.push(
      <Panel
        key={section.id}
        title={section.title}
        className={section.id === "buttons" ? "rd-peri-buttons" : undefined}
      >
        <PropertyControls
          schema={schemaOf(`peripheral-${section.id}`, section.title, properties)}
          values={values}
          readOnly={readOnly}
          disabled={!writes}
          optionLabel={choiceWord}
          onChange={onChange}
        />
      </Panel>,
    );
  }

  return (
    <div className="stack">
      {/* The device head, with Access merged into it: what may be done with it is a switch
          beside the device it belongs to (R4). */}
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
          <StateDot tone={info.connected ? "ok" : "mute"}>
            {info.connected ? "Connected" : "Not connected"}
          </StateDot>
          <div className="rd-peri-access">
            <Tooltip label="canvas-rgb opens the device and asks for its settings; it changes nothing">
              <span className="rd-peri-toggle">
                <span className="rd-peri-toggle-label">Read</span>
                <Toggle
                  label="Read settings from this device"
                  checked={reads}
                  onChange={(v) => onAccess({ reads: v, writes: v ? undefined : false })}
                />
              </span>
            </Tooltip>
            <Tooltip label="asks the device for its settings again, for changes made on the mouse itself">
              <span className="rd-peri-toggle">
                <Button
                  size="sm"
                  icon={<ArrowsClockwise size={16} />}
                  disabled={!reads || reading}
                  onClick={onReadAgain}
                >
                  Read again
                </Button>
              </span>
            </Tooltip>
            <Tooltip label="lets canvas-rgb change settings on the device">
              <span className="rd-peri-toggle">
                <span className="rd-peri-toggle-label">Change</span>
                <Toggle
                  label="Change settings on the device"
                  checked={writes}
                  disabled={!reads}
                  onChange={(v) => onAccess({ writes: v })}
                />
              </span>
            </Tooltip>
          </div>
        </div>
        {detail.problem && (
          <p className="rule rule-warn" data-role="problem">
            {detail.problem}
          </p>
        )}
      </Panel>

      {/* The sections in columns: a panel never breaks across two of them. */}
      <div className="rd-peri-sections">{columnPanels}</div>

      {widePanels}

      {isController && (
        <Panel title="RGB rings">
          <p className="mute">The rings on the pad are not read yet: this build cannot name them.</p>
        </Panel>
      )}
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
