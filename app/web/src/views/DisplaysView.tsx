"use client";

import { useMemo, useState } from "react";
import { ArrowClockwise, ThermometerSimple, WarningOctagon } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { EmptyState } from "@/components/EmptyState";
import { NumberField } from "@/components/NumberField";
import { Field, Panel, StateDot } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/Select";
import { Toggle } from "@/components/Toggle";
import { useToast } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { refreshDisplays, useSnap } from "@/lib/store";
import type { DisplayConfig } from "@/proto/DisplayConfig";
import type { DisplaySource } from "@/proto/DisplaySource";
import type { DisplayStatus } from "@/proto/DisplayStatus";
import type { DisplayUnit } from "@/proto/DisplayUnit";
import type { FansStatus } from "@/proto/FansStatus";

/** The source of a display, as one string: the picker works on keys, not on the spine's shape. */
function sourceKey(source: DisplaySource): string {
  switch (source.kind) {
    case "cpu_temp":
      return "cpu_temp";
    case "gpu_temp":
      return "gpu_temp";
    case "board_temp":
      return `board:${source.sensor}`;
    case "fan_rpm":
      return `fan:${source.header}`;
    case "fixed":
      return "fixed";
    case "api":
      return "api";
  }
}

function sourceFromKey(key: string, current: DisplaySource): DisplaySource {
  if (key === "cpu_temp") return { kind: "cpu_temp" };
  if (key === "gpu_temp") return { kind: "gpu_temp" };
  if (key.startsWith("board:")) return { kind: "board_temp", sensor: key.slice("board:".length) };
  if (key.startsWith("fan:")) return { kind: "fan_rpm", header: key.slice("fan:".length) };
  if (key === "fixed") {
    return { kind: "fixed", value: current.kind === "fixed" ? current.value : 0 };
  }
  return { kind: "api" };
}

/** A fan RPM is a whole number: the daemon refuses decimals with one. */
function isRpm(source: DisplaySource): boolean {
  return source.kind === "fan_rpm";
}

/**
 * The digits the pump head shows, the way the adapter writes them: four positions, the first one
 * blank only when it is a leading zero, and the decimal point before the last digit in the
 * temperature mode. A value that does not fit is refused by the adapter, so it is refused here
 * too.
 */
export function displayDigits(
  value: number | null,
  digits: number,
  decimals: number,
  temperature: boolean,
): { cells: (string | null)[]; dot: boolean; fits: boolean } {
  if (value === null) return { cells: [], dot: false, fits: true };
  if (value < 0) return { cells: [], dot: false, fits: false };
  const scaled = temperature
    ? decimals === 0
      ? Math.round(value) * 10
      : Math.round(value * 10)
    : Math.round(value);
  if (scaled > 10 ** digits - 1) return { cells: [], dot: false, fits: false };
  const text = String(scaled).padStart(digits, "0");
  const cells = text.split("").map((ch, i) => (i === 0 && ch === "0" ? null : ch));
  return { cells, dot: temperature, fits: true };
}

/** The big readout: the display's own number, as the pump head shows it. */
function DigitReadout({
  value,
  digits,
  decimals,
  temperature,
  unit,
}: {
  value: number | null;
  digits: number;
  decimals: number;
  temperature: boolean;
  unit: DisplayUnit;
}) {
  const shown = displayDigits(value, digits, decimals, temperature);
  const mark = temperature ? (unit === "celsius" ? "°C" : "°F") : "RPM";
  if (value === null) {
    return (
      <div className="digit-readout" data-blank="true">
        <span className="digit-readout-empty">No reading yet</span>
      </div>
    );
  }
  if (!shown.fits) {
    return (
      <div className="digit-readout" data-blank="true">
        <span className="digit-readout-empty">The number does not fit</span>
      </div>
    );
  }
  return (
    <div className="digit-readout" role="img" aria-label={`Would show ${value} ${mark}`}>
      {shown.cells.map((cell, i) => (
        <span key={i} className="digit" data-blank={cell === null ? "true" : "false"}>
          {cell ?? ""}
          {shown.dot && i === shown.cells.length - 2 ? <span className="digit-dot" aria-hidden="true" /> : null}
        </span>
      ))}
      <span className="digit-unit">{mark}</span>
    </div>
  );
}

/**
 * The number a display would show with this source, read from the same sensors the daemon reads:
 * the fan service's temperatures and the header speeds. It is a preview: the daemon's own value
 * is the one in the readout when the source is saved.
 */
function previewValue(
  source: DisplaySource,
  unit: DisplayUnit,
  fans: FansStatus | null,
  apiValue: number | null,
): number | null {
  let value: number | null = null;
  if (source.kind === "fixed") value = source.value;
  else if (source.kind === "api") value = apiValue;
  else if (source.kind === "cpu_temp") {
    value = fans?.temps.find((t) => t.source.type === "cpu")?.celsius ?? null;
  } else if (source.kind === "gpu_temp") {
    value = fans?.temps.find((t) => t.source.type === "gpu")?.celsius ?? null;
  } else if (source.kind === "board_temp") {
    value =
      fans?.temps.find((t) => t.source.type === "board" && t.source.name === source.sensor)?.celsius ??
      null;
  } else {
    value = fans?.headers.find((h) => h.id === source.header)?.rpm ?? null;
  }
  if (value !== null && unit === "fahrenheit") value = value * 9 / 5 + 32;
  return value;
}

/** One display: what it shows now, and the source it is driven from. */
function DisplayCard({ row, fans }: { row: DisplayStatus; fans: FansStatus | null }) {
  const toast = useToast();
  const [draft, setDraft] = useState<DisplayConfig>(row.config);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const boardSensors = (fans?.temps ?? []).filter((t) => t.source.type === "board");

  const options = useMemo(
    () => [
      { value: "cpu_temp", label: "CPU temperature" },
      { value: "gpu_temp", label: "GPU temperature" },
      ...boardSensors.map((t) => ({
        value: `board:${t.source.type === "board" ? t.source.name : ""}`,
        label: `${t.label} (board sensor)`,
      })),
      ...(fans?.headers ?? []).map((h) => ({ value: `fan:${h.id}`, label: `${h.label} (fan RPM)` })),
      { value: "fixed", label: "A fixed number" },
      { value: "api", label: "Set by API" },
    ],
    [boardSensors, fans],
  );

  const savedKey = JSON.stringify(row.config);
  // A display that has never been written to is not driven yet, so its source is not saved either.
  const driven = row.shown !== null;
  const dirty = JSON.stringify(draft) !== savedKey || !driven;
  const rpm = isRpm(draft.source);
  const temperature = !rpm;
  // What the pump head would show with the source in the editor, in the unit it is written in.
  const preview = previewValue(draft.source, draft.unit, fans, row.shown?.value ?? null);
  const shown = row.shown;

  const set = (patch: Partial<DisplayConfig>) => {
    setDraft((prev) => ({ ...prev, ...patch }));
    setError(null);
  };

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.putDisplay(row.info.id, draft);
      await refreshDisplays();
      toast(`${row.info.name}: source saved`);
    } catch (e) {
      setError(messageOf(e));
      toast(messageOf(e), "down");
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.deleteDisplay(row.info.id);
      await refreshDisplays();
      toast(`${row.info.name}: no longer driven`);
    } catch (e) {
      setError(messageOf(e));
      toast(messageOf(e), "down");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Panel
      title={row.info.name}
      actions={
        <StateDot tone={row.info.connected ? "ok" : "down"}>
          {row.info.connected ? "connected" : "not connected"}
        </StateDot>
      }
    >
      <div className="display-card">
        <div className="display-readout">
          {/* The big number is what the source would read, not what the display shows: it says so,
              and the line under it says what the display shows now. */}
          <p className="readout-caption">
            <span className="readout-caption-title">Would show</span>
            <span className="readout-caption-note">the source set here, once saved</span>
          </p>
          <DigitReadout
            value={preview}
            digits={row.info.digits}
            decimals={draft.decimals}
            temperature={temperature}
            unit={draft.unit}
          />
          <p className="readout-now">
            <span className="readout-now-label">Now</span>
            {shown ? (
              <>
                {" "}
                the pump head shows{" "}
                <span className="fig">
                  {shown.value} {shown.mode === "temperature" ? (shown.unit === "celsius" ? "°C" : "°F") : "RPM"}
                </span>
                , written {shown.blink ? "with the alarm blink" : "without a blink"}.
              </>
            ) : (
              <> nothing is written to it. It keeps whatever it showed before.</>
            )}
          </p>
          <p className="mute display-readout-note fig">
            {row.info.digits} digits · {row.info.max_decimals} decimal
            {row.info.max_decimals === 1 ? "" : "s"} at most
          </p>
        </div>

        <div className="display-controls">
          <Field label="Source" hint="Where the number comes from">
            <Select
              label={`${row.info.name} source`}
              value={sourceKey(draft.source)}
              options={options}
              onChange={(key) => {
                const source = sourceFromKey(key, draft.source);
                set({ source, decimals: isRpm(source) ? 0 : draft.decimals });
              }}
            />
          </Field>

          {draft.source.kind === "fixed" && (
            <Field label="The number" hint="The display shows this, whatever the machine does">
              <NumberField
                label="The fixed number"
                min={0}
                max={9999}
                step={1}
                value={draft.source.value}
                onCommit={(v) => set({ source: { kind: "fixed", value: v } })}
              />
            </Field>
          )}

          <Field label="Unit" hint="The unit the number is written in">
            <Segmented<DisplayUnit>
              label={`${row.info.name} unit`}
              value={draft.unit}
              options={[
                { value: "celsius", label: "°C" },
                { value: "fahrenheit", label: "°F" },
              ]}
              onChange={(unit) => set({ unit })}
            />
          </Field>

          <Field label="Decimals" hint={rpm ? "A fan RPM is a whole number" : "The decimal point before the last digit"}>
            <Segmented<"0" | "1">
              label={`${row.info.name} decimals`}
              value={String(draft.decimals) as "0" | "1"}
              options={[
                { value: "0", label: "0" },
                { value: "1", label: "1" },
              ]}
              onChange={(v) => set({ decimals: Number(v) })}
            />
          </Field>

          <Field label="Blink at or above" hint="The display blinks while the number is at or above this">
            <span className="row-wrap">
              <Toggle
                label={`${row.info.name} blinks`}
                checked={draft.blink_above !== null}
                onChange={(on) => set({ blink_above: on ? 60 : null })}
              />
              {draft.blink_above !== null && (
                <NumberField
                  label="The blink threshold"
                  min={0}
                  max={9999}
                  step={1}
                  value={draft.blink_above}
                  unit="°"
                  onCommit={(v) => set({ blink_above: v })}
                />
              )}
            </span>
          </Field>

          <div className="row-wrap display-actions">
            <Button variant="primary" disabled={!dirty || busy} onClick={() => void save()}>
              Save the source
            </Button>
            <Button variant="ghost" disabled={busy} onClick={() => void stop()}>
              Stop driving it
            </Button>
            {dirty && <span className="mute">Not saved yet. Nothing is sent until it is.</span>}
          </div>
          {error && <p className="error-line">{error}</p>}
        </div>
      </div>
    </Panel>
  );
}

export function DisplaysView() {
  const displays = useSnap((s) => s.displays);
  const displaysError = useSnap((s) => s.displaysError);
  const displaysUnsupported = useSnap((s) => s.displaysUnsupported);
  const fans = useSnap((s) => s.fans);

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Displays</h1>
        <p>A display takes one number, not colours: a pump head that shows a temperature or a fan speed.</p>
      </div>

      <p className="rule rule-warn">
        <WarningOctagon size={16} weight="fill" aria-hidden="true" /> Close the vendor app (ZEUS CAST)
        first. canvas-rgb never opens a display while ZEUS CAST is running, because both would write
        it. Nothing is sent to a display until its source is saved here.
      </p>

      {displaysUnsupported ? (
        // The daemon answered 404: it is older than the display support. That is a version to fix,
        // not a failure, so it does not read like one.
        <Panel>
          <EmptyState
            icon={<ArrowClockwise size={22} />}
            title="This daemon is older than the editor"
            action={
              <Button size="sm" onClick={() => void refreshDisplays()}>
                Check again
              </Button>
            }
          >
            The daemon does not have the display routes yet: it was built before displays existed.
            Update the daemon and the tray app to set a display up. Nothing else on this page is
            affected, and no display is touched until one has a source.
          </EmptyState>
        </Panel>
      ) : displaysError ? (
        <Panel title="The displays could not be read">
          <p className="error-line">{displaysError}</p>
        </Panel>
      ) : displays === null ? (
        <Panel title="Reading the displays">
          <p className="mute">The daemon has not answered yet.</p>
        </Panel>
      ) : displays.length === 0 ? (
        <Panel>
          <EmptyState icon={<ThermometerSimple size={22} />} title="No display has a source yet">
            The daemon drives a display only while it has a saved source, so a display that has
            never been set up is left alone. Nothing is sent to it.
          </EmptyState>
        </Panel>
      ) : (
        displays.map((row) => (
          // The key carries the saved config, so the card's draft starts from the daemon's config
          // and follows it when it is saved, from here or from the CLI.
          <DisplayCard
            key={`${row.info.id}:${JSON.stringify(row.config)}`}
            row={row}
            fans={fans}
          />
        ))
      )}
    </div>
  );
}
