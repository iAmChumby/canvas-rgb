"use client";
import { ErrorState } from "@/components/ErrorState";
import { StaleNotice } from "@/components/StaleNotice";

import { useState } from "react";
import { DownloadSimple, FolderOpen, Info, Lifebuoy, UploadSimple } from "@phosphor-icons/react";
import { Button, ButtonLink, FileButton } from "@/components/Button";
import { ColorField } from "@/components/ColorField";
import { Field, Panel, StateDot } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { InlineSkeleton } from "@/components/Skeleton";
import { Toggle } from "@/components/Toggle";
import { NumberField } from "@/components/NumberField";
import { Slider } from "@/components/Slider";
import { api } from "@/lib/endpoints";
import { caught, type Failure } from "@/lib/errors";
import { exportConfig, importConfigFile } from "@/lib/config-transfer";
import { BLACK_HEX } from "@/lib/color";
import { openLogsFolder, tauriHost } from "@/lib/host";
import { SLOW_POLL_MS, getSnap, patchSnap, refreshAll, refreshSettings, retrySettings, useSnap } from "@/lib/store";
import { fmt1 } from "@/lib/format";
import { ErrorPanel, LoadFrame } from "@/components/Loading";
import { SettingsShape } from "@/components/LoadShapes";
import {
  applyMotion,
  applyTheme,
  readMotion,
  readTheme,
  type MotionPref,
  type Theme,
} from "@/shell/prefs";
import type { Settings } from "@/proto/Settings";
import { UpdatesPanel } from "./UpdatesPanel";
import { AudioSourcePanel } from "./AudioSourcePanel";
import { PageTitle } from "@/shell/PageHeader";

export function SettingsView() {
  const settings = useSnap((s) => s.settings);
  const settingsError = useSnap((s) => s.settingsError);
  // A load that is in flight after a Try again: the frame shows its placeholder until it ends.
  const retrying = useSnap((s) => s.pending.includes("settings"));
  const status = useSnap((s) => s.status);
  const [fps, setFps] = useState<number | null>(null);
  const [brightness, setBrightness] = useState<number | null>(null);
  const [warmth, setWarmth] = useState<number | null>(null);
  const [err, setErr] = useState<Failure | null>(null);
  const [renderErr, setRenderErr] = useState<Failure | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [theme, setTheme] = useState<Theme>(() => readTheme());
  const [motion, setMotion] = useState<MotionPref>(() => readMotion());
  const [bootDraft, setBootDraft] = useState<string | null>(null);
  const inTray = tauriHost() !== null;

  if (!settings) {
    return (
      <div className="rd-page rd-settings">
        <PageTitle title="Settings" />
        <LoadFrame
          state={retrying ? "loading" : settingsError ? "error" : "loading"}
          shape={<SettingsShape />}
          label="Reading the settings"
          error={
            <ErrorPanel
              title="The settings could not be read"
              summary="canvas-rgb could not read the settings. Nothing was changed."
              error={settingsError}
              onRetry={() => void retrySettings()}
            />
          }
        />
      </div>
    );
  }

  const shownFps = fps ?? settings.fps;
  const shownBrightness = brightness ?? settings.brightness;
  const shownWarmth = warmth ?? settings.warmth;

  const save = async (patch: Partial<Settings>) => {
    setErr(null);
    setNote(null);
    const next: Settings = { ...settings, ...patch };
    patchSnap({ settings: next });
    try {
      await api.putSettings(next);
      await refreshSettings();
    } catch (e) {
      setErr(caught(e));
      await refreshSettings();
    }
  };

  const commitFps = async (v: number) => {
    // The settings route is where the frame rate lives now (#310: the layout's render route is
    // gone, and the scene's scale and sampling are fixed). `save` shows a refusal and puts the
    // control back on the daemon's value.
    setFps(null);
    await save({ fps: v });
  };

  const commitBrightness = async (v: number) => {
    setRenderErr(null);
    try {
      const out = await api.patchOutput({ power: null, brightness: v, warmth: null });
      const cur = getSnap().settings;
      if (cur) patchSnap({ settings: { ...cur, brightness: out.brightness } });
      setBrightness(null);
      await refreshSettings();
    } catch (e) {
      setBrightness(null);
      setRenderErr(caught(e));
    }
  };

  const commitWarmth = async (v: number) => {
    setRenderErr(null);
    try {
      const out = await api.patchOutput({ power: null, brightness: null, warmth: v });
      const cur = getSnap().settings;
      if (cur) patchSnap({ settings: { ...cur, warmth: out.warmth } });
      setWarmth(null);
      await refreshSettings();
    } catch (e) {
      setWarmth(null);
      setRenderErr(caught(e));
    }
  };

  const doExport = async () => {
    setErr(null);
    setNote(null);
    try {
      const name = await exportConfig();
      setNote(`Downloaded ${name}`);
    } catch (e) {
      setErr(caught(e));
    }
  };

  const doImport = async (file: File) => {
    setErr(null);
    setNote(null);
    try {
      await importConfigFile(file);
      setNote(`Imported ${file.name}`);
      await refreshAll();
    } catch (e) {
      setErr(caught(e));
    }
  };

  return (
    <div className="rd-page rd-settings">
      <PageTitle title="Settings" />

      {/* A refresh that failed after the settings were read does not take the page away: the
          settings stay and the page says they may be old. */}
      <StaleNotice
        message="The settings may be out of date."
        error={settingsError}
        everyMs={SLOW_POLL_MS}
        onRetry={() => void retrySettings()}
      />

      {/* The pane the panels scroll in when the window is too short for them (R1). */}
      <div className="rd-settings-scroll">
      <div className="rd-settings-grid">
          <Panel title="Rendering">
            <Field
              label="Frame rate"
              hint={<span className="fig" aria-busy={!status || undefined}>{status ? fmt1(status.fps_measured) : <InlineSkeleton width="4ch" />} fps measured</span>}
            >
              <span className="row-wrap field-slider">
                <Slider
                  label="Frame rate"
                  min={10}
                  max={60}
                  value={shownFps}
                  onChange={setFps}
                  onCommit={commitFps}
                />
                <NumberField label="Frame rate value" min={10} max={60} value={shownFps} unit="fps" onCommit={commitFps} />
              </span>
            </Field>
            <Field label="Brightness">
              <span className="row-wrap field-slider">
                <Slider
                  label="Brightness"
                  min={0}
                  max={100}
                  value={shownBrightness}
                  onChange={setBrightness}
                  onCommit={commitBrightness}
                />
                <NumberField label="Brightness value" min={0} max={100} value={shownBrightness} unit="%" onCommit={commitBrightness} />
              </span>
            </Field>
            <Field label="Warmth">
              <span className="row-wrap field-slider">
                <Slider
                  label="Warmth"
                  min={1800}
                  max={6500}
                  step={100}
                  value={shownWarmth}
                  onChange={setWarmth}
                  onCommit={commitWarmth}
                />
                <NumberField label="Warmth value" min={1800} max={6500} step={100} value={shownWarmth} unit="K" onCommit={commitWarmth} />
                {shownWarmth === 6500 && <span className="unit">Off</span>}
              </span>
            </Field>
            {renderErr && <ErrorState compact error={renderErr} style={{ marginTop: 8 }} />}
          </Panel>

          {/* Audio source sits under Rendering: both are what the effects draw from, and the
              columns balance by it. The left column then reads Rendering, Audio source,
              Appearance; the right one the system's: Lock and quit, Setup, Version (#294). */}
          <AudioSourcePanel />

          <Panel title="Appearance">
            <Field label="Theme">
              <Segmented<Theme>
                label="Theme"
                value={theme}
                options={[
                  { value: "dark", label: "Dark" },
                  { value: "light", label: "Light" },
                  { value: "system", label: "System" },
                ]}
                onChange={(t) => {
                  setTheme(t);
                  applyTheme(t);
                }}
              />
            </Field>
            <Field label="Motion">
              <Segmented<MotionPref>
                label="Interface motion"
                value={motion}
                options={[
                  { value: "full", label: "Full" },
                  { value: "reduced", label: "Reduced" },
                ]}
                onChange={(m) => {
                  setMotion(m);
                  applyMotion(m);
                }}
              />
            </Field>
          </Panel>

          <Panel title="Lock and quit">
            <Field label="Black on lock">
              <Toggle
                label="Black on lock and monitors off"
                checked={settings.black_on_lock}
                onChange={(v) => void save({ black_on_lock: v })}
              />
            </Field>
            <Field label="On quit">
              <Segmented<"black" | "leave_last_frame">
                label="On quit"
                value={settings.on_quit}
                options={[
                  { value: "black", label: "Black" },
                  { value: "leave_last_frame", label: "Leave last frame" },
                ]}
                onChange={(v) => void save({ on_quit: v })}
              />
            </Field>
            <Field label="Boot colour">
              <span className="row-wrap">
                <ColorField
                  label="Boot colour"
                  value={bootDraft ?? settings.boot_color ?? BLACK_HEX}
                  onChange={setBootDraft}
                  onCommit={(hex) => {
                    setBootDraft(null);
                    if (hex !== settings.boot_color) void save({ boot_color: hex });
                  }}
                />
                <Button size="sm" variant="ghost" disabled={settings.boot_color === null} onClick={() => void save({ boot_color: null })}>
                  Clear
                </Button>
              </span>
            </Field>
          </Panel>

          <Panel title="Setup">
            <Field label="Export">
              <Button icon={<DownloadSimple size={16} />} onClick={() => void doExport()}>
                Export
              </Button>
            </Field>
            <Field label="Import">
              <FileButton accept="application/json,.json" icon={<UploadSimple size={16} />} onFile={(f) => void doImport(f)}>
                Import
              </FileButton>
            </Field>
            {note && <StateDot tone="ok">{note}</StateDot>}
            {err && <ErrorState compact error={err} style={{ marginTop: 8 }} />}
          </Panel>

          <Panel title="Version" actions={<span className="fig rd-version" aria-busy={!status || undefined}>{status ? status.version : <InlineSkeleton width="8ch" />}</span>}>
            <UpdatesPanel />
            <div className="row-wrap" style={{ marginTop: 12 }}>
              <ButtonLink href="/credits/" icon={<Info size={16} />}>
                Credits
              </ButtonLink>
              <ButtonLink href="/help/" icon={<Lifebuoy size={16} />}>
                {"Help & About"}
              </ButtonLink>
              <Button
                icon={<FolderOpen size={16} />}
                disabled={!inTray}
                onClick={() => {
                  setErr(null);
                  openLogsFolder().catch((e: unknown) => setErr(caught(e)));
                }}
              >
                Open logs folder
              </Button>
              {!inTray && <span className="mute" style={{ fontSize: 14 }}>Tray app only</span>}
            </div>
          </Panel>
      </div>
      </div>
    </div>
  );
}
