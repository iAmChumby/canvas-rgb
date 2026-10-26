"use client";

import { useEffect, useRef, useState } from "react";
import { CopySimple, FloppyDisk, Plus, Trash, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { NumberField, TextField } from "@/components/NumberField";
import { Panel, Field, StateDot } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/Select";
import { Slider } from "@/components/Slider";
import { Sparkline, Figure } from "@/components/Readout";
import { FanCurveEditor } from "@/views/FanCurveEditor";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { refreshFans, useSnap } from "@/lib/store";
import { baseFloor, hotRuleText } from "@/lib/fan-rules";
import { clamp, fmtCelsius, fmtInt } from "@/lib/format";
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

/** The temperature a curve header reads, if its source reports one. */
function tempFor(mode: FanMode, temps: TempReading[]): number | null {
  if (mode.mode !== "curve") return null;
  const key = sourceKey(mode.source);
  const reading = temps.find((t) => sourceKey(t.source) === key);
  return reading?.celsius ?? null;
}

function defaultCurve(floor: number): CurvePoint[] {
  return [
    { temp_c: 30, percent: Math.max(floor, 30) },
    { temp_c: 50, percent: Math.max(floor, 45) },
    { temp_c: 70, percent: Math.max(floor, 70) },
    { temp_c: 85, percent: 100 },
  ];
}

export function FansView() {
  const fans = useSnap((s) => s.fans);
  const fansError = useSnap((s) => s.fansError);
  const rpm = useSnap((s) => s.rpm);
  const [picked, setSelectedId] = useState<string | null>(null);
  // The first header is the default selection until one is clicked.
  const selectedId = picked ?? fans?.headers[0]?.id ?? null;

  if (fansError || !fans) {
    return (
      <div className="stack">
        <div className="page-head">
          <h1>Fans</h1>
        </div>
        <Panel title="The fan service is not answering">
          <div className="stack" style={{ gap: 10 }}>
            <StateDot tone="down">Service down</StateDot>
            <p className="dim" style={{ wordBreak: "break-word" }}>
              {fansError ?? "Reading the fans."}
            </p>
            <p className="mute" style={{ fontSize: 14 }}>
              Lighting is unaffected. This page fills in when the fan service reconnects.
            </p>
          </div>
        </Panel>
      </div>
    );
  }

  const selected = fans.headers.find((h) => h.id === selectedId) ?? fans.headers[0] ?? null;

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
        <Panel title="Headers">
          <div className="table-scroll">
          <table className="fan-table">
            <thead>
              <tr>
                <th scope="col">Header</th>
                <th scope="col" className="num-col">RPM</th>
                <th scope="col" className="num-col">Duty</th>
                <th scope="col">Mode</th>
                <th scope="col">Last 60 s</th>
              </tr>
            </thead>
            <tbody>
              {fans.headers.map((h) => (
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
        </Panel>

        <div className="stack">
          {selected && (
            <HeaderEditor
              key={`${selected.id}|${selected.mode.mode}|${fans.active_profile}`}
              header={selected}
              fans={fans}
            />
          )}
          <Panel title="Temperatures">
            {fans.temps.length === 0 && <p className="mute">No temperature sources reported.</p>}
            {fans.temps.map((t) => (
              <div className="temp-row" key={sourceKey(t.source)}>
                <span>{t.label}</span>
                <span className="fig">{fmtCelsius(t.celsius)}</span>
              </div>
            ))}
          </Panel>
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
        {header.on_bios && <span className="mute" style={{ display: "block", fontSize: 14 }}>on BIOS</span>}
      </td>
      <td className="num-col">
        <span className="rpm-big">{fmtInt(header.rpm)}</span>
      </td>
      <td className="num-col">{header.pwm_percent === null ? "—" : `${header.pwm_percent}%`}</td>
      <td>
        <Chip>{modeLabel(header.mode)}</Chip>
      </td>
      <td>
        <div className="row-wrap" style={{ flexWrap: "nowrap" }}>
          <Sparkline values={history} max={peak} label={`${header.label} RPM over the last 60 seconds`} />
        </div>
      </td>
    </tr>
  );
}

function ProfilesPanel({ fans }: { fans: FansStatus }) {
  const [err, setErr] = useState<string | null>(null);
  // The profile whose delete is waiting for an inline confirm.
  const [confirming, setConfirming] = useState<string | null>(null);

  const current = (): Record<string, FanMode> =>
    Object.fromEntries(fans.headers.map((h) => [h.id, h.mode]));

  const run = async (fn: () => Promise<unknown>) => {
    setErr(null);
    try {
      await fn();
      await refreshFans();
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
    <Panel
      title="Profiles"
      actions={
        <div className="row-wrap">
          <Button size="sm" icon={<CopySimple size={14} />} onClick={() => void run(() => api.putFanProfile(nextName(fans.active_profile), { name: nextName(fans.active_profile), headers: current() }))}>
            Duplicate
          </Button>
          <Button size="sm" icon={<FloppyDisk size={14} />} onClick={() => void run(() => api.putFanProfile(fans.active_profile, { name: fans.active_profile, headers: current() }))}>
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
                onClick={() => void run(() => api.switchFanProfile(p))}
                label={`Switch to profile ${p}`}
              >
                {p}
              </Chip>
              <IconButton
                label={`Delete profile ${p}`}
                size="sm"
                icon={<Trash size={14} />}
                onClick={() => setConfirming(p)}
              />
            </span>
          ))}
        </div>
        {confirming && (
          <div className="confirm" role="alert">
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
        )}
        <Field
          label="Rename the active profile"
          hint="Type a new name and press Enter. The profile is saved under the new name, which becomes active, and the old name is removed."
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
              })
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

  const manual = mode.mode !== "bios";
  const current = mode.mode;

  return (
    <Panel
      title={header.label}
      actions={<span className="mute fig">{fmtInt(header.rpm)} RPM</span>}
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
          <p className="rule">The daemon overrode this: {header.override_reason}</p>
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
            <div className="row-wrap" style={{ flex: "1 1 260px", minWidth: 0 }}>
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

