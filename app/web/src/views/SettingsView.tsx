"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { CompassTool, DownloadSimple, FolderOpen, UploadSimple } from "@phosphor-icons/react";
import { Button, FileButton } from "@/components/Button";
import { ColorField } from "@/components/ColorField";
import { Field, Panel, StateDot } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { Toggle } from "@/components/Toggle";
import { NumberField } from "@/components/NumberField";
import { Slider } from "@/components/Slider";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { exportConfig, importConfigFile } from "@/lib/config-transfer";
import { BLACK_HEX } from "@/lib/color";
import { openLogsFolder, tauriHost } from "@/lib/host";
import { getSnap, patchSnap, refreshAll, refreshRender, refreshSettings, retrySettings, useSnap } from "@/lib/store";
import { fmt1, fmtInt } from "@/lib/format";
import { useToast } from "@/components/Toast";
import { ErrorPanel, Details, LoadFrame } from "@/components/Loading";
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

export function SettingsView() {
  const settings = useSnap((s) => s.settings);
  const settingsError = useSnap((s) => s.settingsError);
  // A load that is in flight after a Try again: the frame shows its placeholder until it ends.
  const retrying = useSnap((s) => s.pending.includes("settings"));
  const status = useSnap((s) => s.status);
  const layouts = useSnap((s) => s.layouts);
  const render = useSnap((s) => s.render);
  const [fps, setFps] = useState<number | null>(null);
  const [brightness, setBrightness] = useState<number | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [renderErr, setRenderErr] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [theme, setTheme] = useState<Theme>(() => readTheme());
  const [motion, setMotion] = useState<MotionPref>(() => readMotion());
  const [bootDraft, setBootDraft] = useState<string | null>(null);
  const inTray = tauriHost() !== null;
  const toast = useToast();
  const router = useRouter();

  if (!settings) {
    return (
      <div className="stack">
        <div className="page-head">
          <h1>Settings</h1>
        </div>
        <LoadFrame
          state={retrying ? "loading" : settingsError ? "error" : "loading"}
          shape={<SettingsShape />}
          label="Reading the settings"
          error={
            <ErrorPanel
              title="The settings could not be read"
              summary="canvas-rgb could not read the settings. Nothing was changed."
              message={settingsError}
              onRetry={() => void retrySettings()}
            />
          }
        />
      </div>
    );
  }

  const shownFps = fps ?? render?.fps ?? settings.fps;
  const shownBrightness = brightness ?? settings.brightness;

  // The layout is named the same everywhere: the Layout page titles the layout by its name, so
  // this says the name too, not the id the daemon stores it under.
  const activeLayout = layouts?.find((l) => l.id === status?.active_layout) ?? null;
  const layoutName = activeLayout?.name ?? status?.active_layout ?? "—";

  const save = async (patch: Partial<Settings>) => {
    setErr(null);
    setNote(null);
    const next: Settings = { ...settings, ...patch };
    patchSnap({ settings: next });
    try {
      await api.putSettings(next);
      await refreshSettings();
    } catch (e) {
      setErr(messageOf(e));
      await refreshSettings();
    }
  };

  const commitFps = async (v: number) => {
    setRenderErr(null);
    try {
      // The daemon's answer goes into the snapshot first: the draft goes away onto the new value,
      // not onto the old one for as long as the refresh takes.
      patchSnap({ render: await api.patchRender({ fps: v, scale: null, sampling: null }) });
      setFps(null);
      await refreshRender();
    } catch (e) {
      // The control goes back to the value the daemon has: nothing fails silently.
      setFps(null);
      setRenderErr(messageOf(e));
      toast(messageOf(e), "down");
    }
  };

  const commitBrightness = async (v: number) => {
    setRenderErr(null);
    try {
      const out = await api.patchOutput({ power: null, brightness: v });
      const cur = getSnap().settings;
      if (cur) patchSnap({ settings: { ...cur, brightness: out.brightness } });
      setBrightness(null);
      await refreshSettings();
    } catch (e) {
      setBrightness(null);
      setRenderErr(messageOf(e));
      toast(messageOf(e), "down");
    }
  };

  const doExport = async () => {
    setErr(null);
    setNote(null);
    try {
      const name = await exportConfig();
      setNote(`Downloaded ${name}`);
      toast(`Downloaded ${name}`);
    } catch (e) {
      setErr(messageOf(e));
      toast(messageOf(e), "down");
    }
  };

  const doImport = async (file: File) => {
    setErr(null);
    setNote(null);
    try {
      await importConfigFile(file);
      setNote(`Imported ${file.name}`);
      toast(`Imported ${file.name}`);
      await refreshAll();
    } catch (e) {
      setErr(messageOf(e));
      toast(messageOf(e), "down");
    }
  };

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Settings</h1>
      </div>

      {settingsError && (
        // A refresh that failed after the settings were read does not take the page away: the
        // settings stay and the page says they may be old. The raw words are behind the
        // disclosure.
        <div className="rule rule-warn" role="status">
          The settings may be out of date.
          <Details>{settingsError}</Details>
        </div>
      )}

      <div className="settings-grid">
        <div className="stack">
          <Panel title="Rendering">
            <Field
              label="Frame rate"
              hint={<span className="fig">{status ? fmt1(status.fps_measured) : "—"} fps measured</span>}
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
            {renderErr && <p className="error-line" style={{ marginTop: 8 }}>{renderErr}</p>}
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
        </div>

        <div className="stack">
          <Panel title="Lights">
            <Field label="Map my PC" hint="Walk through every light again">
              <Button
                icon={<CompassTool size={16} />}
                onClick={() => {
                  try {
                    window.sessionStorage.setItem("canvas-rgb.onboarding-remap", "1");
                  } catch {
                    // The address carries it instead.
                  }
                  router.push("/?map=1");
                }}
              >
                Map my PC again
              </Button>
            </Field>
          </Panel>

          <Panel title="Config">
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
            {err && <p className="error-line" style={{ marginTop: 8 }}>{err}</p>}
          </Panel>

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
                  toast(`Theme: ${t}`);
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
                  toast(m === "reduced" ? "Motion reduced" : "Motion on");
                }}
              />
            </Field>
          </Panel>

          <Panel title="About">
            <dl className="kv">
              <dt>Version</dt>
              <dd>{status?.version ?? "—"}</dd>
              <dt>Active layout</dt>
              <dd>{layoutName}</dd>
              <dt>Devices connected</dt>
              <dd>{fmtInt(status?.devices_connected ?? null)}</dd>
            </dl>
            <div className="row-wrap" style={{ marginTop: 12 }}>
              <Button
                icon={<FolderOpen size={16} />}
                disabled={!inTray}
                onClick={() => {
                  setErr(null);
                  openLogsFolder().catch((e: unknown) => setErr(messageOf(e)));
                }}
              >
                Open logs folder
              </Button>
              {!inTray && <span className="mute" style={{ fontSize: 14 }}>Tray app only</span>}
            </div>
          </Panel>

          <UpdatesPanel />
        </div>
      </div>
    </div>
  );
}
