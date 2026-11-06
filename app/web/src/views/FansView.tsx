"use client";

import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import { ArrowClockwise, Check, CopySimple, DotsThreeOutline, Eye, EyeSlash, Fan, FloppyDisk, ListChecks, Minus, Plus, Trash, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Checkbox } from "@/components/Checkbox";
import { Chip } from "@/components/Chip";
import { Popover } from "@/components/Popover";
import { EmptyState } from "@/components/EmptyState";
import { NumberField, TextField } from "@/components/NumberField";
import { Panel, Field } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/Select";
import { Slider } from "@/components/Slider";
import { Sparkline, Figure } from "@/components/Readout";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { usePageNarrow } from "@/components/MediaQuery";
import { PageActions } from "@/shell/PageHeader";
import { Details, LoadFrame } from "@/components/Loading";
import { FansShape } from "@/components/LoadShapes";
import { notify } from "@/components/Notify";
import { FanCurveEditor } from "@/views/FanCurveEditor";
import { FanBulkBar } from "@/views/FanBulk";
import { TACH_ONLY_WHY, UNAVAILABLE_WHY, canDrive, isHandedBack, isUnavailable, plainLabel } from "@/lib/fanBulk";
import { api } from "@/lib/endpoints";
import { writeFanMode } from "@/lib/fanWrites";
import { messageOf } from "@/lib/errors";
import { refreshFans, retryFans as retryFansNow, useSnap } from "@/lib/store";
import { baseFloor, hotRuleText } from "@/lib/fan-rules";
import { clamp, fmtSeconds } from "@/lib/format";
import type { CurvePoint } from "@/proto/CurvePoint";
import type { FanHeaderStatus } from "@/proto/FanHeaderStatus";
import type { FanMode } from "@/proto/FanMode";
import type { FansStatus } from "@/proto/FansStatus";
import type { HotKind } from "@/proto/HotKind";
import type { HotLimits } from "@/proto/HotLimits";
import type { TempSource } from "@/proto/TempSource";
import type { TempReading } from "@/proto/TempReading";
import { PageTitle } from "@/shell/PageHeader";

const MAX_POINTS = 8;
const MIN_POINTS = 2;

function sourceKey(s: TempSource): string {
  return s.type === "board" ? `board:${s.name}` : s.type;
}

function sourceFromKey(k: string): TempSource {
  if (k.startsWith("board:")) return { type: "board", name: k.slice("board:".length) };
  return k === "gpu" ? { type: "gpu" } : { type: "cpu" };
}

function modeLabel(m: FanMode): string {
  if (m.mode === "bios") return "BIOS";
  if (m.mode === "fixed") return `Fixed ${m.percent}%`;
  return "Curve";
}

/** The control mode as a state: a dot and a word, no border, so it never reads as a button. */
function ModeState({ mode }: { mode: FanMode }) {
  return (
    <span className="mode-state" data-mode={mode.mode}>
      <span className="mode-dot" aria-hidden="true" />
      {modeLabel(mode)}
    </span>
  );
}

/** The temperature a curve header reads, if its source reports one. */
function tempFor(mode: FanMode, temps: TempReading[]): number | null {
  if (mode.mode !== "curve") return null;
  const key = sourceKey(mode.source);
  const reading = temps.find((t) => sourceKey(t.source) === key);
  return reading?.celsius ?? null;
}

/** The duty a curve asks for at a temperature: the same evaluation the daemon does. */
function dutyFor(points: CurvePoint[], temp: number | null): number | null {
  if (temp === null || points.length === 0) return null;
  const sorted = [...points].sort((a, b) => a.temp_c - b.temp_c);
  const first = sorted[0];
  const last = sorted[sorted.length - 1];
  if (!first || !last) return null;
  if (temp <= first.temp_c) return first.percent;
  if (temp >= last.temp_c) return last.percent;
  for (let i = 1; i < sorted.length; i++) {
    const a = sorted[i - 1];
    const b = sorted[i];
    if (a && b && temp <= b.temp_c) {
      const f = b.temp_c === a.temp_c ? 0 : (temp - a.temp_c) / (b.temp_c - a.temp_c);
      return a.percent + (b.percent - a.percent) * f;
    }
  }
  return last.percent;
}

/** A temperature bar reads cool, warm or hot, and says so in colour only. */
function tempTone(c: number): string {
  if (c < 50) return "var(--a3)";
  if (c < 72) return "var(--warn)";
  return "var(--down)";
}

function defaultCurve(floor: number): CurvePoint[] {
  return [
    { temp_c: 30, percent: Math.max(floor, 30) },
    { temp_c: 50, percent: Math.max(floor, 45) },
    { temp_c: 70, percent: Math.max(floor, 70) },
    { temp_c: 85, percent: 100 },
  ];
}

/**
 * The headers the editor last read, kept for this viewer, so a fan-service outage can still show the
 * machine. A viewer convenience only: nothing reads it back as state, and nothing writes to a fan from it.
 */
const KNOWN_HEADERS_KEY = "canvas-rgb.fan-headers";
type KnownHeader = { id: string; label: string; rpm: number | null; on_bios: boolean };

function readKnownHeaders(): KnownHeader[] {
  try {
    const v: unknown = JSON.parse(window.localStorage.getItem(KNOWN_HEADERS_KEY) ?? "[]");
    return Array.isArray(v) ? (v as KnownHeader[]) : [];
  } catch {
    return [];
  }
}

function writeKnownHeaders(list: KnownHeader[]): void {
  try {
    window.localStorage.setItem(KNOWN_HEADERS_KEY, JSON.stringify(list));
  } catch {
    // Storage can be blocked; the page still works without the memory.
  }
}

export function FansView() {
  const fans = useSnap((s) => s.fans);
  const fansError = useSnap((s) => s.fansError);
  const serviceDownMs = useSnap((s) => s.status?.service_down_ms ?? null);
  const rpm = useSnap((s) => s.rpm);
  // A load that is in flight after a Try again: the frame shows its placeholder until it ends.
  const retrying = useSnap((s) => s.pending.includes("fans"));
  // The page's box, not the window: the editor sheet in the CSS switches on at 640 px of it.
  const narrow = usePageNarrow(640);
  const [picked, setSelectedId] = useState<string | null>(null);
  // The first header is the default selection until one is clicked. On a phone the list is the
  // page and the editor is a sheet a tap pushes over it, so nothing is selected until then.
  const selectedId = picked ?? (narrow ? null : fans?.headers[0]?.id ?? null);
  // The headers ticked for a change to several at once (#110), and the phone's select mode.
  const [ticked, setTicked] = useState<string[]>([]);
  const [selectMode, setSelectMode] = useState(false);
  const lastTick = useRef<string | null>(null);
  // The bulk bar is applying: the Select/Done key waits, so the run is not cut off from its report.
  const [bulkBusy, setBulkBusy] = useState(false);

  // The headers last read: from this session if there was one, else from the viewer's memory.
  const [saved] = useState<KnownHeader[]>(() => readKnownHeaders());
  const known: KnownHeader[] = fans
    ? fans.headers.map((h) => ({ id: h.id, label: h.label, rpm: h.rpm ?? null, on_bios: h.on_bios }))
    : saved;
  useEffect(() => {
    if (fans) writeKnownHeaders(fans.headers.map((h) => ({ id: h.id, label: h.label, rpm: h.rpm ?? null, on_bios: h.on_bios })));
  }, [fans]);

  // How long the service has been away, in words.
  const downFor = serviceDownMs === null ? "a moment" : fmtSeconds(serviceDownMs);

  // The "Try again" on the service-down state.
  const retryFans = async () => {
    await retryFansNow();
  };
  // The error sits in the frame the page loads into, so the page keeps its shape.
  const errorNode = fansError ? (
      <div className="stack">
        {/* One card says it: the title, how long, what keeps working, and the way to try again.
            The status strip says it in its own words; this page does not repeat the strip. */}
        <Panel>
          <EmptyState
            icon={<Fan size={24} />}
            tone="warn"
            title="The fan service is not answering"
            action={
              <Button size="sm" icon={<ArrowClockwise size={16} />} onClick={() => void retryFans()}>
                Try again
              </Button>
            }
          >
            No answer for {downFor}. The fans stay under BIOS control; the lights are unaffected.
            <Details>{fansError}</Details>
          </EmptyState>
        </Panel>
        {known.length > 0 && (
          <Panel title="Last reading">
            <ul className="known-headers" aria-label="Fan headers, last read">
              {known.map((h) => (
                <li key={h.id}>
                  <span>{h.label}</span>
                  <span className="fig">{h.rpm !== null ? `${h.rpm} RPM` : "no reading"}</span>
                  <span className="mute">{h.on_bios ? "on BIOS" : "not on BIOS"}</span>
                </li>
              ))}
            </ul>
          </Panel>
        )}
      </div>
  ) : null;

  const selected = fans ? (fans.headers.find((h) => h.id === selectedId) ?? (narrow ? null : fans.headers[0] ?? null)) : null;
  const headers = fans?.headers ?? [];
  const checked = headers.filter((h) => canDrive(h) && ticked.includes(h.id));
  const drivable = headers.filter(canDrive);
  /** Ticks or unticks a header; with shift, every header between the last tick and this one. */
  const toggleTick = (id: string, shift = false) => {
    const h = headers.find((x) => x.id === id);
    if (!h || !canDrive(h)) return;
    const last = lastTick.current;
    const on = !ticked.includes(id);
    let ids = [id];
    if (shift && last) {
      const a = drivable.findIndex((x) => x.id === last);
      const b = drivable.findIndex((x) => x.id === id);
      if (a >= 0 && b >= 0) ids = drivable.slice(Math.min(a, b), Math.max(a, b) + 1).map((x) => x.id);
    }
    lastTick.current = id;
    setTicked((cur) => (on ? [...new Set([...cur, ...ids])] : cur.filter((x) => !ids.includes(x))));
  };
  // A header that leaves the list (a profile switch, a chip that stops reporting it) leaves the ticks.
  const idsKey = headers.map((h) => h.id).join("|");
  const [seenIds, setSeenIds] = useState(idsKey);
  if (seenIds !== idsKey) {
    setSeenIds(idsKey);
    const live = new Set(idsKey.split("|"));
    setTicked((cur) => (cur.every((id) => live.has(id)) ? cur : cur.filter((id) => live.has(id))));
  }
  const allTicked = drivable.length > 0 && checked.length === drivable.length;
  const clearTicks = () => {
    setTicked([]);
    lastTick.current = null;
  };
  // A refresh that failed after the readings were read does not take the page away: the numbers
  // stay and the page says they may be old.
  const stale = fans !== null && fansError !== null;

  return (
    <div className="rd-page rd-fans">
      <PageTitle
        title="Fans"
        context={
          fans && !fansError ? (
            <>
              {fans.chip ?? "No chip"} · floor <AnimatedNumber value={fans.floor_percent} />%
            </>
          ) : undefined
        }
      />
      {fans && !fansError && <FanProfileActions fans={fans} />}

      {stale && (
        <div className="rule rule-warn" role="status">
          The fan readings may be out of date.
          <Details>{fansError}</Details>
        </div>
      )}

      <LoadFrame
        className="stack"
        state={retrying ? "loading" : fansError && fans === null ? "error" : fans ? "ready" : "loading"}
        shape={<FansShape />}
        label="Reading the fans"
        error={errorNode}
      >
      {fans && (
      <div className="rd-fans-grid">
        <div className="rd-fans-left">
          <Panel title="Temperatures">
            {fans.temps.length === 0 && <p className="mute">No temperature sources reported.</p>}
            <div className="rd-temp-row">
              {fans.temps.map((t) => {
                const c = t.celsius ?? 0;
                return (
                  <div className="temp-cell" key={sourceKey(t.source)}>
                    <span className="one-line">{t.label}</span>
                    <span className="meter" aria-hidden="true">
                      <span
                        className="meter-fill"
                        style={
                          {
                            "--p": clamp((c - 20) / 80, 0, 1),
                            "--tone": tempTone(c),
                          } as CSSProperties
                        }
                      />
                    </span>
                    <span className="fig">
                      <AnimatedNumber value={c} decimals={1} /> °C
                    </span>
                  </div>
                );
              })}
            </div>
          </Panel>
          <Panel
            title="Headers"
            className="rd-headers"
            actions={
              narrow ? (
                <Button
                  size="sm"
                  variant={selectMode ? "primary" : "default"}
                  icon={<ListChecks size={14} />}
                  aria-pressed={selectMode}
                  disabled={bulkBusy}
                  className="fan-select-key"
                  onClick={() => {
                    if (selectMode) clearTicks();
                    setSelectMode(!selectMode);
                  }}
                >
                  {selectMode ? "Done" : "Select"}
                </Button>
              ) : undefined
            }
          >
          {(!narrow || selectMode) && (
            <FanBulkBar
              fans={fans}
              checked={checked}
              focusedId={selected?.id ?? null}
              onClear={clearTicks}
              onBusy={setBulkBusy}
              write={writeFanMode}
            />
          )}
          {narrow ? (
            <div className="fan-cards">
              {headers.map((h) => (
                <FanCard
                  key={h.id}
                  header={h}
                  selected={selected?.id === h.id}
                  history={rpm[h.id] ?? []}
                  selectMode={selectMode}
                  checked={ticked.includes(h.id)}
                  onSelect={() => (selectMode ? toggleTick(h.id) : setSelectedId(h.id))}
                />
              ))}
            </div>
          ) : (
            <div className="table-scroll">
              <table className="fan-table">
                <thead>
                  <tr>
                    <th scope="col" className="tick-col">
                      <Checkbox
                        label="Select every header that can be set"
                        checked={allTicked}
                        mixed={checked.length > 0 && !allTicked}
                        onToggle={() => setTicked(allTicked ? [] : drivable.map((h) => h.id))}
                      />
                    </th>
                    <th scope="col" className="name-col">Header</th>
                    <th scope="col" className="num-col">RPM</th>
                    <th scope="col">Duty</th>
                    <th scope="col">Mode</th>
                    <th scope="col" className="rd-history hist-head">Last 60 s</th>
                  </tr>
                </thead>
                <tbody>
                  {headers.map((h) => (
                    <FanRow
                      key={h.id}
                      header={h}
                      selected={selected?.id === h.id}
                      history={rpm[h.id] ?? []}
                      checked={ticked.includes(h.id)}
                      onTick={(shift) => toggleTick(h.id, shift)}
                      onSelect={() => setSelectedId(h.id)}
                    />
                  ))}
                </tbody>
              </table>
            </div>
          )}
          </Panel>
        </div>

        {selected && (
          <div className="rd-fans-right">
            {/* One live node for the whole editor, outside the keyed HeaderEditor: it is the same
                element across a mode switch, so a reason that is there stays said once. */}
            <span className="sr-only" aria-live="polite" data-slot="fan-status-announce">
              {reasonKindOf(selected)}
            </span>
            <HeaderEditor
              key={`${selected.id}|${selected.mode.mode}|${fans.active_profile}`}
              header={selected}
              fans={fans}
            />
          </div>
        )}
      </div>
      )}
      </LoadFrame>
    </div>
  );
}

function FanRow({
  header,
  selected,
  history,
  checked,
  onTick,
  onSelect,
}: {
  header: FanHeaderStatus;
  selected: boolean;
  history: number[];
  checked: boolean;
  onTick: (shift: boolean) => void;
  onSelect: () => void;
}) {
  const peak = Math.max(2000, ...history);
  const duty = header.pwm_percent ?? 0;
  const unavailable = isUnavailable(header);
  const tach = !unavailable && !canDrive(header);
  const blocked = !canDrive(header);
  return (
    <tr
      aria-selected={selected}
      className={unavailable ? "fan-unavailable" : undefined}
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        // Enter opens the header in the editor; Space ticks it for a change to several.
        if (e.key === "Enter") {
          e.preventDefault();
          onSelect();
        }
        if (e.key === " ") {
          e.preventDefault();
          onTick(e.shiftKey);
        }
      }}
    >
      <td className="tick-col">
        <Checkbox
          label={unavailable ? `${header.label}: not on this board, cannot be selected` : tach ? `${header.label}: tach only, cannot be selected` : `Select ${header.label}`}
          checked={checked}
          blocked={blocked}
          title={unavailable ? UNAVAILABLE_WHY : tach ? TACH_ONLY_WHY : undefined}
          onToggle={onTick}
        />
      </td>
      <td className="name-col">
        {/* The name and its chips wrap as items, and the reason stands on its own line under
            them, inside the cell: the cell never widens the table past its pane (#114). */}
        <div className="fan-name-cell">
          <span className="fig" style={{ fontWeight: 600 }}>{plainLabel(header.label)}</span>
          {unavailable && <Chip className="rd-hdr-chip">Not on this board</Chip>}
          {header.pwm_percent === null && !unavailable && <Chip className="rd-hdr-chip">tach only</Chip>}
          <Handoff header={header} ticked={checked} />
        </div>
      </td>
      <td className="num-col">
        <span className="rpm-big">
          <AnimatedNumber value={header.rpm ?? 0} />
        </span>
      </td>
      <td>
        <span className="duty-cell">
          <span className="meter" aria-hidden="true">
            <span
              className="meter-fill"
              style={{ "--p": clamp(duty / 100, 0, 1), "--tone": "var(--a3)" } as CSSProperties}
            />
          </span>
          <span className="fig">{header.pwm_percent === null ? "—" : <><AnimatedNumber value={header.pwm_percent} />%</>}</span>
        </span>
      </td>
      <td>
        <ModeState mode={header.mode} />
      </td>
      <td className="rd-history">
        <div className="row-wrap" style={{ flexWrap: "nowrap" }}>
          <Sparkline values={history} max={peak} label={`${header.label} RPM over the last 60 seconds`} />
        </div>
      </td>
    </tr>
  );
}

/**
 * The one place a header's state is said. The mode chip says what was asked for. When the chip
 * is doing something else, the daemon has handed the header back, and that is said here, with
 * the reason.
 */
function Handoff({ header, ticked }: { header: FanHeaderStatus; ticked?: boolean }) {
  if (!isHandedBack(header)) return null;
  return (
    <>
      {/* A handed-back header can be ticked, and a bulk apply leaves it as it is: the row says so. */}
      <span className="fan-handoff">{ticked ? "Handed back to BIOS, skipped when applied" : "Handed back to BIOS"}</span>
      {header.override_reason && <span className="fan-reason">{header.override_reason}</span>}
    </>
  );
}

/** The same header as a card, for phone width: every column readable, no sideways scroll. */
function FanCard({
  header,
  selected,
  history,
  selectMode,
  checked,
  onSelect,
}: {
  header: FanHeaderStatus;
  selected: boolean;
  history: number[];
  selectMode: boolean;
  checked: boolean;
  onSelect: () => void;
}) {
  const peak = Math.max(2000, ...history);
  const duty = header.pwm_percent ?? 0;
  const unavailable = isUnavailable(header);
  const tach = !unavailable && !canDrive(header);
  const blocked = !canDrive(header);
  // In select mode a card is a tick box: a tap ticks it, and a header that cannot be set says why it cannot be.
  const pick = selectMode
    ? ({ role: "checkbox", "aria-checked": checked, "aria-disabled": blocked || undefined } as const)
    : {};
  return (
    <button
      type="button"
      className={unavailable ? "fan-card fan-unavailable" : "fan-card"}
      data-selected={(selectMode ? checked : selected) ? "true" : "false"}
      title={selectMode && unavailable ? UNAVAILABLE_WHY : selectMode && tach ? TACH_ONLY_WHY : undefined}
      {...pick}
      onClick={() => {
        if (!selectMode || !blocked) onSelect();
      }}
    >
      {selectMode && (
        <span className="tick-box" data-checked={checked ? "true" : "false"} aria-hidden="true">
          {checked ? <Check size={12} weight="bold" /> : blocked ? <Minus size={12} weight="bold" /> : null}
        </span>
      )}
      <span className="fan-card-head">
        <span className="fig fan-card-name">{header.label}</span>
        <ModeState mode={header.mode} />
        {selected && !selectMode && <span className="fan-card-editing">Editing</span>}
        {unavailable && (
          <span className="fan-card-editing">{selectMode ? "Not on this board, cannot be set" : "Not on this board"}</span>
        )}
        {selectMode && tach && <span className="fan-card-editing">Tach only, cannot be set</span>}
        <Handoff header={header} ticked={selectMode && checked} />
      </span>
      <span className="fan-card-readouts">
        <span className="fan-card-rpm">
          <AnimatedNumber value={header.rpm ?? 0} />
          <span className="unit">RPM</span>
        </span>
        <span className="meter" aria-hidden="true">
          <span
            className="meter-fill"
            style={{ "--p": clamp(duty / 100, 0, 1), "--tone": "var(--a3)" } as CSSProperties}
          />
        </span>
        <span className="fig">{header.pwm_percent === null ? "—" : <><AnimatedNumber value={header.pwm_percent} />%</>}</span>
      </span>
      <span className="fan-card-spark">
        <Sparkline values={history} max={peak} label={`${header.label} RPM over the last 60 seconds`} />
      </span>
    </button>
  );
}

/**
 * The fan profiles, in the strip: the profile that is on, the key that saves it and the menu
 * that holds the rest (duplicate, rename, delete). They sit beside the page's name, so the
 * profile that is on is never more than a glance away (R4).
 */
function FanProfileActions({ fans }: { fans: FansStatus }) {
  const [err, setErr] = useState<string | null>(null);
  // The profile whose delete is waiting for an inline confirm.
  const [confirming, setConfirming] = useState<string | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLSpanElement>(null);

  const current = (): Record<string, FanMode> =>
    Object.fromEntries(fans.headers.map((h) => [h.id, h.mode]));

  const run = async (fn: () => Promise<unknown>, done?: string) => {
    setErr(null);
    try {
      await fn();
      await refreshFans();
      // Only a message that the page cannot show is said here: the profile the reader picks is
      // on the keys, the copy is in the list and the deleted one is gone from it.
      if (done) notify(done);
    } catch (e) {
      setErr(messageOf(e));
    }
  };

  const nextName = (base: string): string => {
    let name = `${base} copy`;
    let n = 2;
    while (fans.profiles.includes(name)) name = `${base} copy ${n++}`;
    return name;
  };

  return (
    <PageActions>
      <Segmented
        label="The fan profile"
        value={fans.active_profile}
        options={fans.profiles.map((p) => ({ value: p, label: p }))}
        onChange={(p) => void run(() => api.switchFanProfile(p))}
      />
      <Button
        variant="primary"
        size="sm"
        icon={<FloppyDisk size={14} />}
        aria-label={`Save ${fans.active_profile}`}
        onClick={() =>
          void run(
            () => api.putFanProfile(fans.active_profile, { name: fans.active_profile, headers: current() }),
            `${fans.active_profile} saved`,
          )
        }
      >
        <span className="rd-btn-text">Save {fans.active_profile}</span>
      </Button>
      <span ref={moreRef} className="row-wrap">
        <IconButton
          label="The fan profiles"
          size="sm"
          icon={<DotsThreeOutline size={16} />}
          onClick={() => setMoreOpen((o) => !o)}
        />
      </span>
      <Popover
        anchor={moreRef}
        open={moreOpen}
        onClose={() => setMoreOpen(false)}
        label="The fan profiles"
      >
        <div className="stack" style={{ gap: 12 }}>
          <Button
            size="sm"
            icon={<CopySimple size={14} />}
            onClick={() =>
              void run(() =>
                api.putFanProfile(nextName(fans.active_profile), {
                  name: nextName(fans.active_profile),
                  headers: current(),
                }),
              )
            }
          >
            Duplicate
          </Button>
          <Field label="Rename">
            <TextField
              label="New profile name"
              placeholder="New name"
              value=""
              onCommit={(name) =>
                void run(async () => {
                  const old = fans.active_profile;
                  await api.putFanProfile(name, { name, headers: current() });
                  await api.switchFanProfile(name);
                  if (old !== name) await api.deleteFanProfile(old);
                })
              }
            />
          </Field>
          {confirming ? (
            <div className="confirm slide-in" role="alert">
              <span>
                Delete <span className="fig" style={{ color: "var(--ink)" }}>{confirming}</span>? Its saved
                headers are removed.
              </span>
              <div className="row-wrap" style={{ marginLeft: "auto" }}>
                <Button
                  size="sm"
                  variant="danger"
                  onClick={() =>
                    void run(async () => {
                      await api.deleteFanProfile(confirming);
                      setConfirming(null);
                    })
                  }
                >
                  Delete
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setConfirming(null)}>
                  Cancel
                </Button>
              </div>
            </div>
          ) : (
            fans.profiles
              .filter((p) => p !== fans.active_profile)
              .map((p) => (
                <div key={p} className="row-wrap" style={{ justifyContent: "space-between" }}>
                  <span>{p}</span>
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<Trash size={14} />}
                    aria-label={`Delete profile ${p}`}
                    onClick={() => setConfirming(p)}
                  >
                    Delete
                  </Button>
                </div>
              ))
          )}
          {err && <p className="error-line">{err}</p>}
        </div>
      </Popover>
    </PageActions>
  );
}


/** The one sentence a screen reader hears for a header's reason: it names the KIND, never the live value. */
function reasonKindOf(header: FanHeaderStatus): string {
  if (!header.override_reason) return "";
  const handedBack = header.on_bios && header.mode.mode !== "bios";
  return handedBack
    ? `${header.label}: canvas-rgb handed this header back to BIOS`
    : `canvas-rgb is overriding ${header.label}`;
}

/**
 * The reason box: two lines, whole text behind a More key when it is cut. Open, the box lies over
 * what is below it and does not push it (the region keeps its height), so reading the rest moves
 * nothing. With no reason it says what is in charge, in the same box.
 */
function StatusReason({ text, warn, quiet }: { text: string | null; warn: boolean; quiet: string }) {
  const region = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const textRef = useRef<HTMLSpanElement>(null);
  const [clamped, setClamped] = useState(false);
  const [open, setOpen] = useState(false);
  useLayoutEffect(() => {
    const el = textRef.current;
    if (!el) return;
    const measure = () => {
      if (!open) setClamped(el.scrollHeight > el.clientHeight + 1);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [text, open]);
  // Open, the box closes on a press outside it, on Escape anywhere, and when focus leaves it.
  useEffect(() => {
    if (!open) return;
    const press = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", press, true);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("pointerdown", press, true);
      document.removeEventListener("keydown", key);
    };
  }, [open]);
  const shown = open && text !== null;
  return (
    // The region is the stable element: it takes focus when the key that had it goes away.
    <div className="fan-status-reason" data-slot="fan-status-reason" ref={region} tabIndex={-1}>
      {text ? (
        <div
          ref={box}
          className={`rule${warn ? " rule-warn" : ""} fan-status-box${shown ? " is-open" : ""}`}
          onBlur={(e) => {
            if (e.relatedTarget && !e.currentTarget.contains(e.relatedTarget as Node)) setOpen(false);
          }}
        >
          <span className="fan-status-text" ref={textRef}>
            {text}
          </span>
          {(clamped || shown) && (
            <MoreKey open={shown} onToggle={() => setOpen(!shown)} fallback={region} />
          )}
        </div>
      ) : (
        <p className="fan-status-quiet mute">{quiet}</p>
      )}
    </div>
  );
}

/** The More / Less key. If it goes away with focus on it, focus moves to a stable element first. */
function MoreKey({ open, onToggle, fallback }: { open: boolean; onToggle: () => void; fallback: RefObject<HTMLElement | null> }) {
  const ref = useRef<HTMLSpanElement>(null);
  useLayoutEffect(
    () => () => {
      if (ref.current?.contains(document.activeElement)) fallback.current?.focus();
    },
    [fallback],
  );
  return (
    <span className="fan-status-more" ref={ref}>
      <Button size="sm" variant="ghost" aria-expanded={open} onClick={onToggle}>
        {open ? "Less" : "More"}
      </Button>
    </span>
  );
}

/**
 * Issue #146: the owner's word on whether a header is on this board. A header the board does
 * not route can be shown anyway; a header on the board can be taken off it. While the word is
 * there, the way back to the board list stands beside it.
 *
 * The key is a ghost button, so it reads as an aside of the header and not as a second mode
 * control. A refusal is said next to the key, in the panel's own error colour.
 */
function HeaderAvailability({ header }: { header: FanHeaderStatus }) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const unavailable = isUnavailable(header);
  // The owner's word, when there is one: `false` "not on my board", `true` "show anyway".
  const override = header.override_available ?? null;

  const send = async (available: boolean | null) => {
    setErr(null);
    setBusy(true);
    try {
      await api.putFanHeaderAvailability(header.id, { available });
      await refreshFans();
    } catch (e) {
      setErr(messageOf(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <span className="fan-hdr-availability" data-slot="fan-header-availability">
      {unavailable ? (
        <Button
          size="sm"
          variant="ghost"
          icon={<Eye size={14} />}
          disabled={busy}
          onClick={() => void send(true)}
        >
          Show anyway
        </Button>
      ) : (
        <Button
          size="sm"
          variant="ghost"
          icon={<EyeSlash size={14} />}
          disabled={busy}
          onClick={() => void send(false)}
        >
          Not on my board
        </Button>
      )}
      {override !== null && (
        <Button
          size="sm"
          variant="ghost"
          icon={<ListChecks size={14} />}
          disabled={busy}
          onClick={() => void send(null)}
        >
          Use the board list
        </Button>
      )}
      {err && (
        <span className="error-line fan-hdr-availability-error" role="alert" title={err}>
          {err}
        </span>
      )}
    </span>
  );
}

function HeaderEditor({ header, fans }: { header: FanHeaderStatus; fans: FansStatus }) {
  const base = baseFloor(header.id, fans.floor_percent);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const mode = header.mode;
  const [percent, setPercent] = useState(mode.mode === "fixed" ? mode.percent : Math.max(base, 50));
  const [points, setPoints] = useState<CurvePoint[]>(mode.mode === "curve" ? mode.points : defaultCurve(base));
  const [source, setSource] = useState<TempSource>(mode.mode === "curve" ? mode.source : { type: "cpu" });
  const pointsTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (pointsTimer.current) clearTimeout(pointsTimer.current);
    },
    [],
  );

  /** A curve edit that has not been sent yet is replaced by whatever the user chooses next. */
  const cancelCurve = () => {
    if (pointsTimer.current) clearTimeout(pointsTimer.current);
    pointsTimer.current = null;
  };

  const apply = async (next: FanMode) => {
    setErr(null);
    setBusy(true);
    try {
      await writeFanMode(header.id, next, (e) => {
        setErr(messageOf(e));
      });
      await refreshFans();
    } finally {
      setBusy(false);
    }
  };

  const setMode = (m: "bios" | "fixed" | "curve") => {
    cancelCurve();
    if (m === "bios") void apply({ mode: "bios" });
    if (m === "fixed") {
      setPercent(clamp(percent, base, 100));
      void apply({ mode: "fixed", percent: clamp(percent, base, 100) });
    }
    if (m === "curve") {
      const pts = points.length >= MIN_POINTS ? points : defaultCurve(base);
      setPoints(pts);
      void apply({ mode: "curve", source, points: pts });
    }
  };

  const commitPercent = (v: number) => {
    cancelCurve();
    const p = clamp(Math.round(v), base, 100);
    setPercent(p);
    void apply({ mode: "fixed", percent: p });
  };

  const commitCurve = (pts: CurvePoint[]) => {
    setPoints(pts);
    cancelCurve();
    pointsTimer.current = setTimeout(() => {
      pointsTimer.current = null;
      void apply({ mode: "curve", source, points: pts });
    }, 250);
  };

  const sourceOptions = fans.temps.map((t) => ({ value: sourceKey(t.source), label: t.label }));
  const currentTemp = tempFor({ mode: "curve", source, points }, fans.temps);
  const currentDuty = dutyFor(points, currentTemp);

  // Issue #130: the hot line for this curve's source kind. A Fixed header has no curve
  // source: its reference temperature is the CPU reading, as the service treats it.
  const [hotErr, setHotErr] = useState<string | null>(null);
  const [hotBusy, setHotBusy] = useState(false);
  const hotKind: HotKind = mode.mode === "curve" ? source.type : "cpu";
  const hotRow = (fans.hot ?? []).find((h) => h.kind === hotKind) ?? null;

  /** Sends the whole set of hot limits with one kind changed (`null` = automatic).
   * Resolves true when the service accepted it, false when the PUT failed (422). */
  const commitHot = async (value: number | null): Promise<boolean> => {
    setHotBusy(true);
    setHotErr(null);
    const limits: HotLimits = { cpu_c: null, gpu_c: null, board_c: null };
    for (const row of fans.hot ?? []) {
      const v = row.user_set ? row.limit_c : null;
      if (row.kind === "cpu") limits.cpu_c = v;
      if (row.kind === "gpu") limits.gpu_c = v;
      if (row.kind === "board") limits.board_c = v;
    }
    if (hotKind === "cpu") limits.cpu_c = value;
    if (hotKind === "gpu") limits.gpu_c = value;
    if (hotKind === "board") limits.board_c = value;
    try {
      await api.putFanHot(limits);
      await refreshFans();
      return true;
    } catch (e) {
      setHotErr(messageOf(e));
      return false;
    } finally {
      setHotBusy(false);
    }
  };

  const manual = mode.mode !== "bios";
  const current = mode.mode;
  // The chip is doing something else than what was asked for: the daemon handed it back.
  const handedBack = header.on_bios && mode.mode !== "bios";
  // What the daemon says about this header, in words that name it. The words are on the page for
  // the eye; the reader of a screen hears one stable sentence per KIND of reason, so a live
  // temperature inside the reason ("while CPU is 78 °C") is not read again at every change.
  const reasonText = header.override_reason
    ? handedBack
      ? `canvas-rgb handed ${header.label} back to BIOS: ${header.override_reason}`
      : `canvas-rgb overrode ${header.label}: ${header.override_reason}`
    : null;
  // The status stays in the page whether it has anything to say or not: the reason, an error and
  // "Applying" fill room that is already reserved, so a commit or a hot floor never grows the
  // panel, never flashes its scrollbar and never moves the points (#109). It stands under the
  // control it explains: the chart, the duty or the mode note.
  const quiet = {
    bios: "The BIOS is in charge of this fan.",
    fixed: "Fixed duty is in charge; nothing is overriding it.",
    curve: "The curve is in charge; nothing is overriding it.",
  }[mode.mode];
  const status = (
    <div className="fan-status" data-slot="fan-status">
      <StatusReason text={reasonText} warn={handedBack} quiet={quiet} />
      <div className="fan-status-line" data-slot="fan-status-line">
        <span className="fan-status-error error-line" role="alert" title={err ?? undefined}>
          {err ?? ""}
        </span>
        <span className="fan-status-busy mute">{busy ? "Applying" : ""}</span>
      </div>
    </div>
  );

  // The compact "don't exceed" control, shared by Fixed (the CPU kind, as the
  // service treats it) and Curve. It sits in the floor note, under the sentence it
  // sets, never in a Field of its own, so it adds the least height to the panel.
  // The words, the figure and its state stay together on one line (they wrap as one group);
  // Reset shows only once the user has set a limit, as a settings reset usually does.
  const hotControls = hotRow && (
    <span className="hot-controls">
      <span className="mute">Don&apos;t exceed</span>
      <NumberField
        label="Don't exceed temperature"
        min={40}
        max={hotRow.max_settable_c}
        value={hotRow.limit_c}
        unit="°C"
        disabled={hotBusy}
        onCommit={(v) => void commitHot(v)}
      />
      {hotRow.user_set ? (
        <Button
          size="sm"
          variant="ghost"
          icon={<ArrowClockwise size={14} />}
          disabled={hotBusy}
          onClick={() => void commitHot(null)}
        >
          Reset to automatic
        </Button>
      ) : (
        <span className="mute">automatic</span>
      )}
    </span>
  );
  const hotError = hotErr && (
    <span className="error-line" role="alert">
      {hotErr}
    </span>
  );

  return (
    <Panel
      title={header.label}
      className={isUnavailable(header) ? "fan-unavailable" : undefined}
      actions={
        <span className="fan-hdr-actions">
          {/* Issue #146: the key that takes a header off this board. A header the board does not
              route says its own thing in the panel's body instead, so it is not here twice. */}
          {!isUnavailable(header) && <HeaderAvailability header={header} />}
          <span className="rpm-readout">
            <AnimatedNumber value={header.rpm ?? 0} />
            <span className="unit">RPM</span>
          </span>
        </span>
      }
    >
      {isUnavailable(header) ? (
        <div className="stack" style={{ gap: 8 }}>
          <p className="mute">
            {header.override_available === false
              ? "You marked this header as not on your board, so there is no mode to set."
              : "Not on this board: this header is not fitted on your board, so there is no mode to set."}
          </p>
          <HeaderAvailability header={header} />
        </div>
      ) : (
      <div className="stack" style={{ gap: 8 }}>
        <Segmented<"bios" | "fixed" | "curve">
          label={`${header.label} mode`}
          value={current}
          options={[
            { value: "bios", label: "BIOS" },
            { value: "fixed", label: "Fixed %" },
            { value: "curve", label: "Curve" },
          ]}
          onChange={setMode}
        />

        {mode.mode === "bios" && (
          <p className="mute">Chip auto control. Floors off.</p>
        )}
        {mode.mode === "bios" && status}

        {manual && (
          <div className="rule">
            <span>
              Floor <span className="fig" style={{ color: "var(--ink)" }}><AnimatedNumber value={base} />%</span>. {hotRuleText(hotRow?.limit_c)}.
            </span>
            {hotControls}
            {hotError}
          </div>
        )}

        {mode.mode === "fixed" && (
          <Field label="Duty">
            <div className="row-wrap field-slider">
              <Slider
                label={`${header.label} duty`}
                min={base}
                max={100}
                value={percent}
                floor={base}
                disabled={busy}
                onChange={setPercent}
                onCommit={commitPercent}
              />
              <NumberField
                label={`${header.label} duty value`}
                min={base}
                max={100}
                value={percent}
                unit="%"
                onCommit={commitPercent}
              />
            </div>
          </Field>
        )}
        {mode.mode === "fixed" && status}

        {mode.mode === "curve" && (
          <>
            <Field label="Temperature source">
              <div className="row-wrap">
                <Select
                  label={`${header.label} temperature source`}
                  value={sourceKey(source)}
                  options={sourceOptions}
                  onChange={(k) => {
                    const s = sourceFromKey(k);
                    cancelCurve();
                    setSource(s);
                    void apply({ mode: "curve", source: s, points });
                  }}
                />
              </div>
            </Field>
            <FanCurveEditor
              header={header.id}
              points={points}
              floor={base}
              currentTemp={currentTemp}
              currentDuty={currentDuty}
              hot={hotRow}
              onChange={setPoints}
              onCommit={commitCurve}
              onHotCommit={(v) => commitHot(v)}
            />
            {status}
            <div className="curve-rows">
              {points.map((p, i) => (
                <div className="curve-row" key={i}>
                  <span className="curve-row-index">{i + 1}</span>
                  <NumberField
                    label={`Point ${i + 1} temperature`}
                    min={20}
                    max={100}
                    value={p.temp_c}
                    unit="°C"
                    onCommit={(v) => {
                      const next = points.map((q, j) => (j === i ? { ...q, temp_c: v } : q));
                      commitCurve([...next].sort((a, b) => a.temp_c - b.temp_c));
                    }}
                  />
                  <NumberField
                    label={`Point ${i + 1} percent`}
                    min={base}
                    max={100}
                    value={p.percent}
                    unit="%"
                    onCommit={(v) => {
                      const next = points.map((q, j) => (j === i ? { ...q, percent: v } : q));
                      commitCurve(next);
                    }}
                  />
                  <IconButton
                    label={`Remove point ${i + 1}`}
                    size="sm"
                    disabled={points.length <= MIN_POINTS}
                    icon={<X size={14} />}
                    onClick={() => commitCurve(points.filter((_, j) => j !== i))}
                  />
                </div>
              ))}
              <div className="row-wrap">
                <Button
                  size="sm"
                  icon={<Plus size={14} />}
                  disabled={points.length >= MAX_POINTS}
                  onClick={() => {
                    const last = points[points.length - 1];
                    const t = last ? Math.min(100, last.temp_c + 10) : 50;
                    commitCurve([...points, { temp_c: t, percent: Math.max(base, 100) }].sort((a, b) => a.temp_c - b.temp_c));
                  }}
                >
                  Add point
                </Button>
                <span className="mute fig" style={{ fontSize: 14 }}>
                  <AnimatedNumber value={points.length} /> of {MAX_POINTS} points
                </span>
              </div>
            </div>
          </>
        )}

        {mode.mode === "fixed" && (
          <p className="mute" style={{ fontSize: 14 }}>
            Duty <Figure unit="%">{header.pwm_percent === null ? "—" : <AnimatedNumber value={header.pwm_percent} />}</Figure>
          </p>
        )}
      </div>
      )}
    </Panel>
  );
}
