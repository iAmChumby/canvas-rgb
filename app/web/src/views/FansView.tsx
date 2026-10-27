"use client";

import { useEffect, useRef, useState, type CSSProperties } from "react";
import { ArrowClockwise, CopySimple, Fan, FloppyDisk, Plus, Trash, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { NumberField, TextField } from "@/components/NumberField";
import { Panel, Field } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/Select";
import { Slider } from "@/components/Slider";
import { Sparkline, Figure } from "@/components/Readout";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { useMediaQuery } from "@/components/MediaQuery";
import { SkeletonLines } from "@/components/Skeleton";
import { useToast } from "@/components/Toast";
import { FanCurveEditor } from "@/views/FanCurveEditor";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { refreshFans, useSnap } from "@/lib/store";
import { baseFloor, hotRuleText } from "@/lib/fan-rules";
import { clamp, fmtSeconds } from "@/lib/format";
import type { CurvePoint } from "@/proto/CurvePoint";
import type { FanHeaderStatus } from "@/proto/FanHeaderStatus";
import type { FanMode } from "@/proto/FanMode";
import type { FansStatus } from "@/proto/FansStatus";
import type { TempSource } from "@/proto/TempSource";
import type { TempReading } from "@/proto/TempReading";

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
  const narrow = useMediaQuery("(max-width: 720px)");
  const [picked, setSelectedId] = useState<string | null>(null);
  // The first header is the default selection until one is clicked.
  const selectedId = picked ?? fans?.headers[0]?.id ?? null;

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
    await refreshFans();
  };

  if (fansError) {
    return (
      <div className="stack">
        <div className="page-head">
          <h1>Fans</h1>
        </div>
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
            The daemon has not heard from it for {downFor}. Fan control is unavailable until the
            service is back. The lights are unaffected, and the fans stay under BIOS control in the
            meantime.
            <span className="state-detail">the daemon said: {fansError}</span>
          </EmptyState>
        </Panel>
        {known.length > 0 && (
          <Panel title="Headers, as the editor last read them">
            <p className="mute known-note">Read only while the fan service is away. Nothing here can be changed.</p>
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
    );
  }

  if (!fans) {
    return (
      <div className="stack">
        <div className="page-head">
          <h1>Fans</h1>
        </div>
        <Panel title="Reading the fans">
          <SkeletonLines rows={5} />
        </Panel>
      </div>
    );
  }

  const selected = fans.headers.find((h) => h.id === selectedId) ?? fans.headers[0] ?? null;
  const headers = fans.headers;

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Fans</h1>
        <p>
          {fans.chip ?? "Chip not detected"} · floor {fans.floor_percent}% · profile{" "}
          <span style={{ color: "var(--ink)" }}>{fans.active_profile}</span>
        </p>
      </div>

      <ProfilesPanel fans={fans} />

      <div className="fans-grid">
        <div className="stack">
        <Panel title="Headers">
          {narrow ? (
            <div className="fan-cards">
              {headers.map((h) => (
                <FanCard
                  key={h.id}
                  header={h}
                  selected={selected?.id === h.id}
                  history={rpm[h.id] ?? []}
                  onSelect={() => setSelectedId(h.id)}
                />
              ))}
            </div>
          ) : (
            <div className="table-scroll">
              <table className="fan-table">
                <thead>
                  <tr>
                    <th scope="col">Header</th>
                    <th scope="col" className="num-col">RPM</th>
                    <th scope="col">Duty</th>
                    <th scope="col">Mode</th>
                    <th scope="col">Last 60 s</th>
                  </tr>
                </thead>
                <tbody>
                  {headers.map((h) => (
                    <FanRow
                      key={h.id}
                      header={h}
                      selected={selected?.id === h.id}
                      history={rpm[h.id] ?? []}
                      onSelect={() => setSelectedId(h.id)}
                    />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Panel>
          <Panel title="Temperatures">
            {fans.temps.length === 0 && <p className="mute">No temperature sources reported.</p>}
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
          </Panel>
        </div>

        <div className="stack fans-editor-col">
          {selected && (
            <HeaderEditor
              key={`${selected.id}|${selected.mode.mode}|${fans.active_profile}`}
              header={selected}
              fans={fans}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function FanRow({
  header,
  selected,
  history,
  onSelect,
}: {
  header: FanHeaderStatus;
  selected: boolean;
  history: number[];
  onSelect: () => void;
}) {
  const peak = Math.max(2000, ...history);
  const duty = header.pwm_percent ?? 0;
  return (
    <tr
      aria-selected={selected}
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <td>
        <span className="fig" style={{ fontWeight: 600 }}>{header.label}</span>
        <Handoff header={header} />
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
          <span className="fig">{header.pwm_percent === null ? "—" : `${header.pwm_percent}%`}</span>
        </span>
      </td>
      <td>
        <ModeState mode={header.mode} />
      </td>
      <td>
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
function Handoff({ header }: { header: FanHeaderStatus }) {
  const handedBack = header.on_bios && header.mode.mode !== "bios";
  if (!handedBack) return null;
  return (
    <>
      <span className="fan-handoff">Handed back to BIOS</span>
      {header.override_reason && <span className="fan-reason">{header.override_reason}</span>}
    </>
  );
}

/** The same header as a card, for phone width: every column readable, no sideways scroll. */
function FanCard({
  header,
  selected,
  history,
  onSelect,
}: {
  header: FanHeaderStatus;
  selected: boolean;
  history: number[];
  onSelect: () => void;
}) {
  const peak = Math.max(2000, ...history);
  const duty = header.pwm_percent ?? 0;
  return (
    <button type="button" className="fan-card" data-selected={selected ? "true" : "false"} onClick={onSelect}>
      <span className="fan-card-head">
        <span className="fig fan-card-name">{header.label}</span>
        <ModeState mode={header.mode} />
        {selected && <span className="fan-card-editing">Editing</span>}
        <Handoff header={header} />
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
        <span className="fig">{header.pwm_percent === null ? "—" : `${header.pwm_percent}%`}</span>
      </span>
      <span className="fan-card-spark">
        <Sparkline values={history} max={peak} label={`${header.label} RPM over the last 60 seconds`} />
      </span>
    </button>
  );
}

function ProfilesPanel({ fans }: { fans: FansStatus }) {
  const [err, setErr] = useState<string | null>(null);
  // The profile whose delete is waiting for an inline confirm.
  const [confirming, setConfirming] = useState<string | null>(null);
  const toast = useToast();

  const current = (): Record<string, FanMode> =>
    Object.fromEntries(fans.headers.map((h) => [h.id, h.mode]));

  const run = async (fn: () => Promise<unknown>, done?: string) => {
    setErr(null);
    try {
      await fn();
      await refreshFans();
      if (done) toast(done);
    } catch (e) {
      setErr(messageOf(e));
      toast(messageOf(e), "down");
    }
  };

  const nextName = (base: string): string => {
    let name = `${base} copy`;
    let n = 2;
    while (fans.profiles.includes(name)) name = `${base} copy ${n++}`;
    return name;
  };

  return (
    <Panel
      title="Profiles"
      actions={
        <div className="row-wrap">
          <Button size="sm" icon={<CopySimple size={14} />} onClick={() => void run(() => api.putFanProfile(nextName(fans.active_profile), { name: nextName(fans.active_profile), headers: current() }), "Profile duplicated")}>
            Duplicate
          </Button>
          <Button variant="primary" size="sm" icon={<FloppyDisk size={14} />} onClick={() => void run(() => api.putFanProfile(fans.active_profile, { name: fans.active_profile, headers: current() }), `${fans.active_profile} saved`)}>
            Save {fans.active_profile}
          </Button>
        </div>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <div className="profile-row">
          {fans.profiles.map((p) => (
            <span key={p} className="row-wrap" style={{ gap: 2 }}>
              <Chip
                pressed={p === fans.active_profile}
                onClick={() => void run(() => api.switchFanProfile(p), `${p} is the active profile`)}
                label={`Switch to profile ${p}`}
              >
                {p}
              </Chip>
              {/* The active profile is not deletable: you switch to another one first. One way to
                  delete, not two. */}
              {p !== fans.active_profile && (
                <IconButton
                  label={`Delete profile ${p}`}
                  size="sm"
                  icon={<Trash size={14} />}
                  onClick={() => setConfirming(p)}
                />
              )}
            </span>
          ))}
        </div>
        {confirming && (
          <div className="confirm slide-in" role="alert">
            <span>
              Delete <span className="fig" style={{ color: "var(--ink)" }}>{confirming}</span>? Its saved headers are removed.
            </span>
            <div className="row-wrap" style={{ marginLeft: "auto" }}>
              <Button
                size="sm"
                variant="danger"
                onClick={() =>
                  void run(async () => {
                    await api.deleteFanProfile(confirming);
                    setConfirming(null);
                  }, `${confirming} deleted`)
                }
              >
                Delete
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setConfirming(null)}>
                Cancel
              </Button>
            </div>
          </div>
        )}
        <Field
          label="Rename the active profile"
          hint="Type a new name and press Enter. The profile is saved under it and becomes active."
        >
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
              }, `Profile renamed to ${name}`)
            }
          />
        </Field>
        {err && <p className="error-line">{err}</p>}
      </div>
    </Panel>
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
  const toast = useToast();

  useEffect(
    () => () => {
      if (pointsTimer.current) clearTimeout(pointsTimer.current);
    },
    [],
  );

  const apply = async (next: FanMode) => {
    setErr(null);
    setBusy(true);
    try {
      await api.patchFanHeader(header.id, next);
      await refreshFans();
    } catch (e) {
      setErr(messageOf(e));
      toast(messageOf(e), "down");
    } finally {
      setBusy(false);
    }
  };

  const setMode = (m: "bios" | "fixed" | "curve") => {
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
    const p = clamp(Math.round(v), base, 100);
    setPercent(p);
    void apply({ mode: "fixed", percent: p });
  };

  const commitCurve = (pts: CurvePoint[]) => {
    setPoints(pts);
    if (pointsTimer.current) clearTimeout(pointsTimer.current);
    pointsTimer.current = setTimeout(() => {
      void apply({ mode: "curve", source, points: pts });
    }, 250);
  };

  const sourceOptions = fans.temps.map((t) => ({ value: sourceKey(t.source), label: t.label }));
  const currentTemp = tempFor({ mode: "curve", source, points }, fans.temps);
  const currentDuty = dutyFor(points, currentTemp);

  const manual = mode.mode !== "bios";
  const current = mode.mode;
  // The chip is doing something else than what was asked for: the daemon handed it back.
  const handedBack = header.on_bios && mode.mode !== "bios";

  return (
    <Panel
      title={header.label}
      actions={
        <span className="rpm-readout">
          <AnimatedNumber value={header.rpm ?? 0} />
          <span className="unit">RPM</span>
        </span>
      }
    >
      <div className="stack" style={{ gap: 14 }}>
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

        {header.override_reason && (
          <p className={handedBack ? "rule rule-warn" : "rule"}>
            {handedBack
              ? "The daemon handed this header back to BIOS"
              : "The daemon overrode this"}
            : {header.override_reason}
          </p>
        )}

        {mode.mode === "bios" && (
          <p className="dim">The chip&apos;s own automatic control, as the BIOS configured it. Floors do not apply.</p>
        )}

        {manual && (
          <div className="rule">
            Floor <span className="fig" style={{ color: "var(--ink)" }}>{base}%</span>. {hotRuleText()}.
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

        {mode.mode === "curve" && (
          <>
            <Field label="Temperature source">
              <Select
                label={`${header.label} temperature source`}
                value={sourceKey(source)}
                options={sourceOptions}
                onChange={(k) => {
                  const s = sourceFromKey(k);
                  setSource(s);
                  void apply({ mode: "curve", source: s, points });
                }}
              />
            </Field>
            <FanCurveEditor
              header={header.id}
              points={points}
              floor={base}
              currentTemp={currentTemp}
              currentDuty={currentDuty}
              onChange={setPoints}
              onCommit={commitCurve}
            />
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
                  {points.length} of {MAX_POINTS} points
                </span>
              </div>
            </div>
          </>
        )}

        {mode.mode === "fixed" && (
          <p className="mute" style={{ fontSize: 14 }}>
            Current duty: <Figure unit="%">{header.pwm_percent ?? "—"}</Figure>
          </p>
        )}
        {err && <p className="error-line">{err}</p>}
        {busy && <p className="mute" style={{ fontSize: 14 }}>Applying</p>}
      </div>
    </Panel>
  );
}
