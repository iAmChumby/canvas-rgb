"use client";

import { useState } from "react";
import { DownloadSimple, FolderOpen, UploadSimple } from "@phosphor-icons/react";
import { Button, FileButton } from "@/components/Button";
import { ColorField } from "@/components/ColorField";
import { NumberField } from "@/components/NumberField";
import { Field, Panel, StateDot } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { Slider } from "@/components/Slider";
import { Toggle } from "@/components/Toggle";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { exportConfig, importConfigFile } from "@/lib/config-transfer";
import { BLACK_HEX } from "@/lib/color";
import { openLogsFolder, tauriHost } from "@/lib/host";
import { patchSnap, refreshAll, refreshRender, refreshSettings, useSnap } from "@/lib/store";
import { fmt1, fmtInt } from "@/lib/format";
import type { Settings } from "@/proto/Settings";

const THEME_KEY = "canvas-rgb.theme";
type Theme = "dark" | "light" | "system";

function readTheme(): Theme {
  if (typeof window === "undefined") return "dark";
  try {
    const v = window.localStorage.getItem(THEME_KEY);
    if (v === "light" || v === "system") return v;
  } catch {
    // Storage can be blocked; the default is dark.
  }
  return "dark";
}

function applyTheme(t: Theme): void {
  const root = document.documentElement;
  if (t === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", t);
  try {
    window.localStorage.setItem(THEME_KEY, t);
  } catch {
    // The choice still applies for this session.
  }
}

export function SettingsView() {
  const settings = useSnap((s) => s.settings);
  const settingsError = useSnap((s) => s.settingsError);
  const status = useSnap((s) => s.status);
  const render = useSnap((s) => s.render);
  const [fps, setFps] = useState<number | null>(null);
  const [brightness, setBrightness] = useState<number | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [theme, setTheme] = useState<Theme>(() => readTheme());
  const [bootDraft, setBootDraft] = useState<string | null>(null);
  const inTray = tauriHost() !== null;

  if (!settings) {
    return (
      <div className="stack">
        <div className="page-head">
          <h1>Settings</h1>
        </div>
        <Panel>
          <p className="mute">{settingsError ? `Settings could not be read: ${settingsError}` : "Reading the settings."}</p>
        </Panel>
      </div>
    );
  }

  const shownFps = fps ?? render?.fps ?? settings.fps;
  const shownBrightness = brightness ?? settings.brightness;

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
    setErr(null);
    try {
      await api.patchRender({ fps: v, scale: null, sampling: null });
      setFps(null);
      await refreshRender();
    } catch (e) {
      setErr(messageOf(e));
      setFps(null);
    }
  };

  const commitBrightness = async (v: number) => {
    setErr(null);
    try {
      await api.patchOutput({ power: null, brightness: v });
      setBrightness(null);
      await refreshSettings();
    } catch (e) {
      setErr(messageOf(e));
      setBrightness(null);
    }
  };

  const doExport = async () => {
    setErr(null);
    setNote(null);
    try {
      const name = await exportConfig();
      setNote(`Downloaded ${name}`);
    } catch (e) {
      setErr(messageOf(e));
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
      setErr(messageOf(e));
    }
  };

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Settings</h1>
        <p>Changes apply as you make them.</p>
      </div>

      <div className="settings-grid">
        <div className="stack">
          <Panel title="Rendering">
            <Field
              label="Frame rate"
              hint={`Measured ${status ? fmt1(status.fps_measured) : "—"} fps over the last second`}
            >
              <span className="row-wrap" style={{ flex: "1 1 240px", minWidth: 0 }}>
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
            <Field label="Brightness" hint="Scales every LED the daemon writes">
              <span className="row-wrap" style={{ flex: "1 1 240px", minWidth: 0 }}>
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
          </Panel>

          <Panel title="Lock and quit">
            <Field label="Go black on lock and monitors off" hint="Off by default">
              <Toggle
                label="Go black on lock and monitors off"
                checked={settings.black_on_lock}
                onChange={(v) => void save({ black_on_lock: v })}
              />
            </Field>
            <Field label="On quit" hint="What the lights do when the tray app exits">
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
            <Field label="Boot colour" hint="Held by the fan service until the daemon connects. Clear it for black.">
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
                  Clear (black)
                </Button>
              </span>
            </Field>
          </Panel>
        </div>

        <div className="stack">
          <Panel title="Config">
            <Field label="Export" hint="Downloads settings, layouts, zone sizes, effects and fan profiles as JSON">
              <Button icon={<DownloadSimple size={16} />} onClick={() => void doExport()}>
                Export config
              </Button>
            </Field>
            <Field label="Import" hint="The daemon refuses a file it cannot read, and says why">
              <FileButton accept="application/json,.json" icon={<UploadSimple size={16} />} onFile={(f) => void doImport(f)}>
                Import config
              </FileButton>
            </Field>
            {note && <StateDot tone="ok">{note}</StateDot>}
            {err && <p className="error-line" style={{ marginTop: 8 }}>{err}</p>}
          </Panel>

          <Panel title="Appearance">
            <Field label="Theme" hint="Dark is the default">
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
          </Panel>

          <Panel title="About">
            <dl className="kv">
              <dt>Version</dt>
              <dd>{status?.version ?? "—"}</dd>
              <dt>Active layout</dt>
              <dd>{status?.active_layout ?? "—"}</dd>
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
              {!inTray && <span className="mute" style={{ fontSize: 14 }}>Available in the tray app.</span>}
            </div>
          </Panel>
        </div>
      </div>
    </div>
  );
}
