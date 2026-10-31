// The demo backend: answers the same routes as the daemon, from in-memory state, when the page
// runs outside the Tauri host (a plain browser or the screenshot server). It mirrors the
// daemon's validation messages (crates/canvas-proto/src/fans.rs) so the inline 422 and 503 paths
// can be seen. `?state=conflict|service-down|first-run|html` changes what the status reports.
//
// The scenarios in `@/lib/demoScenarios` are wired in here too, so the editor's own states can
// be seen in a plain browser: `?state=daemon-down|openrgb-down|no-layout|device-empty|slow|
// save-error|handback:<header id>`.

import { ApiFailure } from "@/lib/errors";
import { demoThumbnail } from "@/lib/thumb-demo";
import {
  answerDelayMs,
  applyDeviceScenarios,
  applyFanScenarios,
  applyLayoutScenarios,
  applyStatusScenarios,
  conflictClearsMs,
  daemonDown,
  foundOnRescan,
  onboardingDone,
  openRgbConnecting,
  saveError,
  wakingMs,
} from "@/lib/demoScenarios";
import {
  DEVICES,
  EFFECTS,
  FAN_HEADERS,
  FAN_PROFILES,
  HTML_FIXTURE,
  INITIAL_SETTINGS,
  ONBOARD_EFFECTS,
  isHiddenEffect,
  LAYOUT_DESK,
  CHIONE_DISPLAY,
  DISPLAY_DEFAULT,
  type DemoEffect,
  type Props,
} from "@/lib/fixtures";
import { PERIPHERALS, type DemoPeripheral } from "@/lib/peripheralFixtures";
import type { Status } from "@/proto/Status";
import type { Layout } from "@/proto/Layout";
import type { FanMode } from "@/proto/FanMode";
import type { FanProfile } from "@/proto/FanProfile";
import type { FanHeaderStatus } from "@/proto/FanHeaderStatus";
import type { FansStatus } from "@/proto/FansStatus";
import type { TempReading } from "@/proto/TempReading";
import type { Settings } from "@/proto/Settings";
import type { ExportBundle } from "@/proto/ExportBundle";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { PropValue } from "@/proto/PropValue";
import type { PropKind } from "@/proto/PropKind";
import type { Preset } from "@/proto/Preset";
import type { PresetCreate } from "@/proto/PresetCreate";
import type { PresetReplace } from "@/proto/PresetReplace";
import type { DisplayConfig } from "@/proto/DisplayConfig";
import type { DisplayInfo } from "@/proto/DisplayInfo";
import type { DisplayStatus } from "@/proto/DisplayStatus";
import type { DisplayUnit } from "@/proto/DisplayUnit";
import type { DisplayValue } from "@/proto/DisplayValue";
import type { PeripheralAccess } from "@/proto/PeripheralAccess";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { ImportEffect } from "@/proto/ImportEffect";
import type { ImportEffectResponse } from "@/proto/ImportEffectResponse";
import type { FailedEffect } from "@/proto/FailedEffect";

const BASE = "/api/v1";
const DEMO_LAYOUTS_KEY = "canvas-rgb.demo-layouts";
const DEMO_ONBOARDED_KEY = "canvas-rgb.demo-onboarded";
const APP_VERSION = "0.1.3";
const SCHEMA_VERSION = 1;

function scenario(): Set<string> {
  if (typeof window === "undefined") return new Set();
  const raw = new URLSearchParams(window.location.search).get("state") ?? "";
  return new Set(raw.split(",").map((s) => s.trim()).filter(Boolean));
}

interface DemoState {
  settings: Settings;
  devices: DeviceInfo[];
  effects: Record<string, DemoEffect>;
  favorites: Set<string>;
  saved: Record<string, Props>;
  /** The owner's presets per effect, in the order they were saved. */
  userPresets: Record<string, Preset[]>;
  activeEffect: string | null;
  layouts: Layout[];
  activeLayout: string;
  fanModes: Record<string, FanMode>;
  fanFloor: number;
  fanProfiles: FanProfile[];
  activeProfile: string;
  zonePanel: boolean;
  /** The owner reached the end of the onboarding. */
  onboardingComplete: boolean;
  /** The devices a rescan finds (`?state=found-on-rescan`), held back until it is asked for. */
  held: DeviceInfo[];
  serviceUp: boolean;
  conflicts: Status["conflicts"];
  startedAt: number;
  /** The numeric display's saved config, and the value last PUT to its value route. */
  displayConfig: DisplayConfig | null;
  displayApiValue: number | null;
  /** The peripherals the adapters see, with what they hold. */
  peripherals: DemoPeripheral[];
  /** What the daemon may do with each peripheral. Nothing is on until the owner says so. */
  peripheralAccess: Record<string, PeripheralAccess>;
  /** The user effect files that did not parse. */
  failedEffects: FailedEffect[];
}

/**
 * `?state=presets`: Aurora has two presets of the owner's, and the one on the lights is
 * "Slow dusk" - so the strip opens with a key lit, as it does for a PC that has used presets.
 */
function seededPresets(): Record<string, Preset[]> {
  const aurora = EFFECTS.find((e) => e.schema.id === "aurora");
  if (!aurora) return {};
  const c = (value: string): PropValue => ({ type: "color", value });
  const n = (value: number): PropValue => ({ type: "number", value });
  return {
    aurora: [
      { name: "Slow dusk", props: { ...aurora.defaults, color1: c("#8fb4e8"), color2: c("#b49cf0"), color3: c("#5fd0c0"), speed: n(8), scale: n(3) } },
      { name: "Late shift", props: { ...aurora.defaults, color1: c("#ffb703"), color2: c("#e5484d"), color3: c("#3a0ca3"), speed: n(35) } },
    ],
  };
}

function initialState(): DemoState {
  const s = scenario();
  const effects: Record<string, DemoEffect> = {};
  for (const e of EFFECTS) effects[e.schema.id] = e;
  for (const e of ONBOARD_EFFECTS) effects[e.schema.id] = e;
  if (s.has("html")) effects[HTML_FIXTURE.schema.id] = HTML_FIXTURE;
  const profile = FAN_PROFILES[0];
  const fanModes: Record<string, FanMode> = {};
  for (const h of FAN_HEADERS) fanModes[h.id] = profile?.headers[h.id] ?? { mode: "bios" };
  const favorites = new Set(EFFECTS.filter((e) => e.summary.favorite).map((e) => e.schema.id));
  const layouts = applyLayoutScenarios([{ ...LAYOUT_DESK, zones: LAYOUT_DESK.zones.map((z) => ({ ...z })) }], LAYOUT_DESK.id);
  // `?state=persist`: saved layouts are kept in localStorage, so a reload sees what was saved,
  // as it does with the daemon. Used by the onboarding's reload checks.
  if (s.has("persist")) {
    try {
      const kept = window.localStorage.getItem(DEMO_LAYOUTS_KEY);
      if (kept) layouts.layouts = JSON.parse(kept) as Layout[];
    } catch {
      // Starts from the fixture.
    }
  }
  if (s.has("presets")) {
    const dusk = seededPresets().aurora?.[0];
    const layout = layouts.layouts.find((l) => l.id === layouts.active);
    if (dusk && layout) layout.effect = { id: "aurora", props: { ...dusk.props } };
  }
  let onboarded = onboardingDone();
  if (s.has("persist")) {
    try {
      if (window.localStorage.getItem(DEMO_ONBOARDED_KEY) === "1") onboarded = true;
    } catch {
      // Not remembered.
    }
  }
  const everyDevice = DEVICES.map((d) => ({ ...d, zones: d.zones.map((z) => ({ ...z })) }));
  return {
    settings: { ...INITIAL_SETTINGS },
    onboardingComplete: onboarded,
    held: foundOnRescan() ? everyDevice : [],
    devices: applyDeviceScenarios(DEVICES.map((d) => ({ ...d, zones: d.zones.map((z) => ({ ...z })) }))),
    effects,
    favorites,
    saved: {},
    userPresets: s.has("presets") ? seededPresets() : {},
    activeEffect: "aurora",
    layouts: layouts.layouts,
    activeLayout: layouts.active,
    fanModes,
    fanFloor: 20,
    fanProfiles: FAN_PROFILES.map((p) => ({ ...p, headers: { ...p.headers } })),
    activeProfile: profile?.name ?? "Default",
    zonePanel: s.has("first-run"),
    serviceUp: !s.has("service-down"),
    conflicts: s.has("conflict") || s.has("conflict-clears") ? [{ kind: "signal_rgb", process: true, service: false }] : [],
    startedAt: Date.now(),
    // No display has a saved source yet: the daemon leaves the pump head alone.
    // `?state=display-saved` (or `vendor-blocked`) starts with a source saved, as a configured PC has.
    displayConfig: s.has("display-saved") || s.has("vendor-blocked") ? { ...DISPLAY_DEFAULT } : null,
    displayApiValue: null,
    // The daemon never opens a device it was not told to: both start with reads off.
    peripherals: PERIPHERALS.map((p) => ({ ...p, values: { ...p.values } })),
    peripheralAccess: {},
    // A user effect file that did not parse: the author sees why.
    failedEffects: [
      {
        file: "neon-grid.html",
        error: "the effect does not parse: effect html:neon-grid has no script",
      },
    ],
  };
}

let state: DemoState | null = null;
function st(): DemoState {
  if (!state) state = initialState();
  return state;
}

function fail(status: number, code: string, message: string): never {
  throw new ApiFailure(status, code, message);
}

function activeLayout(): Layout {
  const found = st().layouts.find((l) => l.id === st().activeLayout);
  if (!found) fail(404, "not_found", "no active layout");
  return found;
}

// ---- simulated sensors and fans ----

function temps(): { cpu: number; gpu: number; board: number; t: number } {
  const t = (Date.now() - st().startedAt) / 1000;
  return {
    cpu: 47 + 7 * (0.5 + 0.5 * Math.sin(t / 9)),
    gpu: 40 + 3 * Math.sin(t / 13),
    board: 34 + 1.4 * Math.sin(t / 17),
    t,
  };
}

function tempFor(source: { type: string; name?: string }): number {
  const r = temps();
  if (source.type === "cpu") return r.cpu;
  if (source.type === "gpu") return r.gpu;
  return r.board;
}

function interpolate(points: { temp_c: number; percent: number }[], temp: number): number {
  const first = points[0];
  const last = points[points.length - 1];
  if (!first || !last) return 0;
  if (temp <= first.temp_c) return first.percent;
  if (temp >= last.temp_c) return last.percent;
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1];
    const b = points[i];
    if (a && b && temp <= b.temp_c) {
      const f = (temp - a.temp_c) / (b.temp_c - a.temp_c);
      return a.percent + f * (b.percent - a.percent);
    }
  }
  return last.percent;
}

function floorFor(header: string, configured: number): number {
  const floor = Math.max(configured, 20);
  return header === "cpu_fan" ? Math.max(floor, 30) : floor;
}

function validateMode(header: string, mode: FanMode, configured: number): void {
  const floor = floorFor(header, configured);
  if (mode.mode === "fixed") {
    if (mode.percent > 100) fail(422, "validation", `${header}: ${mode.percent}% is above 100%`);
    if (mode.percent < floor) {
      fail(422, "validation", `${header}: ${mode.percent}% is below the floor of ${floor}%`);
    }
  }
  if (mode.mode === "curve") {
    const n = mode.points.length;
    if (n < 2 || n > 8) fail(422, "validation", `${header}: a curve needs 2 to 8 points, got ${n}`);
    for (let i = 1; i < n; i++) {
      const a = mode.points[i - 1];
      const b = mode.points[i];
      if (a && b && b.temp_c <= a.temp_c) {
        fail(422, "validation", `${header}: curve points must be sorted by temperature with no repeats`);
      }
    }
    for (const p of mode.points) {
      if (p.percent < floor) {
        fail(422, "validation", `${header}: ${p.percent}% is below the floor of ${floor}%`);
      }
      if (p.percent > 100) fail(422, "validation", `${header}: ${p.percent}% is above 100%`);
    }
  }
}

function fansStatus(): FansStatus {
  const s = st();
  const r = temps();
  const jitter = (i: number) => Math.sin(r.t * 1.7 + i * 2.1) * 9;
  const headers: FanHeaderStatus[] = FAN_HEADERS.map((h, i) => {
    const mode = s.fanModes[h.id] ?? { mode: "bios" as const };
    let percent: number;
    if (mode.mode === "fixed") percent = mode.percent;
    else if (mode.mode === "curve") percent = interpolate(mode.points, tempFor(mode.source));
    else percent = 34 + (i % 3) * 4;
    const rpm = mode.mode === "bios" ? 980 + i * 60 + Math.round(jitter(i) * 4) : Math.round(percent * 17.5 + jitter(i));
    return {
      id: h.id,
      label: h.label,
      rpm,
      pwm_percent: Math.round(percent),
      mode,
      on_bios: mode.mode === "bios",
      override_reason: null,
    };
  });
  const temperatures: TempReading[] = [
    { source: { type: "cpu" }, label: "CPU Tctl", celsius: Math.round(r.cpu * 10) / 10 },
    { source: { type: "gpu" }, label: "GPU", celsius: Math.round(r.gpu * 10) / 10 },
    { source: { type: "board", name: "SYSTIN" }, label: "Board SYSTIN", celsius: Math.round(r.board * 10) / 10 },
  ];
  return {
    chip: "NCT6687D-R @ 0xA20",
    hardware_watchdog: null,
    headers,
    temps: temperatures,
    active_profile: s.activeProfile,
    profiles: s.fanProfiles.map((p) => p.name),
    floor_percent: s.fanFloor,
  };
}

// ---- status ----

function statusNow(): Status {
  const s = st();
  const elapsedMs = Date.now() - s.startedAt;
  const waking = openRgbConnecting() || elapsedMs < wakingMs();
  const connected = waking ? 0 : s.devices.filter((d) => d.connected).length;
  const down = !s.serviceUp;
  const fps = s.settings.fps;
  const jitter = Math.sin(Date.now() / 1700) * 0.12;
  const elapsed = (Date.now() - s.startedAt) / 1000;
  return applyStatusScenarios({
    version: APP_VERSION,
    power: s.settings.power,
    brightness: s.settings.brightness,
    fps_target: fps,
    fps_measured: Math.round((fps - Math.abs(jitter)) * 10) / 10,
    frame_time_ms: Math.round((1000 / fps) * 100) / 100,
    frames_rendered: Math.round((elapsed / 1000) * fps),
    parked: false,
    devices_connected: connected,
    preview_subscribers: 1,
    active_layout: s.activeLayout,
    active_effect: s.activeEffect,
    openrgb: waking ? "connecting" : "connected",
    service: down ? "down" : "connected",
    service_down_ms: down ? 14000 + elapsed : null,
    conflicts: conflictClearsMs() > 0 && elapsed * 1000 > conflictClearsMs() ? [] : s.conflicts,
    show_zone_panel: s.zonePanel,
    onboarding_complete: s.onboardingComplete,
  });
}

// ---- effects ----

function effectOrNotFound(id: string): DemoEffect {
  const e = st().effects[id];
  if (!e) fail(404, "not_found", `no effect with id ${id}`);
  return e;
}

function currentProps(e: DemoEffect): Props {
  const s = st();
  if (s.activeEffect === e.schema.id) {
    const layout = activeLayout();
    if (layout.effect) return { ...layout.effect.props };
  }
  return { ...e.defaults, ...(s.saved[e.schema.id] ?? {}) };
}

function mergeProps(base: Props, over: Props): Props {
  const merged: Props = { ...base };
  for (const [k, v] of Object.entries(over)) {
    if (k in base) merged[k] = v as PropValue;
  }
  return merged;
}

function setActiveProps(over: Props): void {
  const s = st();
  if (!s.activeEffect) fail(422, "validation", "no effect is active");
  const e = effectOrNotFound(s.activeEffect);
  const layout = activeLayout();
  const next = mergeProps(layout.effect?.props ?? currentProps(e), over);
  layout.effect = { id: e.schema.id, props: next };
  s.saved[e.schema.id] = next;
}

// ---- numeric displays ----

/** The display the CHIONE adapter finds: the pump head, four digits. */
function displayInfo(): DisplayInfo {
  // `?state=vendor-blocked`: ZEUS CAST holds the display, as the daemon's poll loop reports it.
  if (scenario().has("vendor-blocked")) return { ...CHIONE_DISPLAY, connected: false, blocked_by: "ZEUS CAST" };
  return { ...CHIONE_DISPLAY };
}

/** The value a display's source reads now, in the display's own unit. */
function displayReads(s: DemoState, config: DisplayConfig): { value: number | null; rpm: boolean } {
  const source = config.source;
  if (source.kind === "fixed") return { value: source.value, rpm: false };
  if (source.kind === "api") return { value: s.displayApiValue, rpm: false };
  if (source.kind === "cpu_temp") return { value: temps().cpu, rpm: false };
  if (source.kind === "gpu_temp") return { value: temps().gpu, rpm: false };
  if (source.kind === "board_temp") {
    const reading = fansStatus().temps.find(
      (t) => t.source.type === "board" && t.source.name === source.sensor,
    );
    return { value: reading?.celsius ?? null, rpm: false };
  }
  const header = fansStatus().headers.find((h) => h.id === source.header);
  return { value: header?.rpm ?? 0, rpm: true };
}

/** What the display shows now: the value in its unit, rounded the way the adapter rounds it. */
function displayShown(s: DemoState, config: DisplayConfig): DisplayValue {
  const { value, rpm } = displayReads(s, config);
  const unit: DisplayUnit = config.unit;
  let out = value;
  if (out !== null && unit === "fahrenheit") out = out * 9 / 5 + 32;
  if (out !== null) {
    const scale = rpm ? 1 : 10 ** config.decimals;
    out = Math.round(out * scale) / scale;
  }
  return {
    value: out,
    decimals: rpm ? 0 : config.decimals,
    unit,
    mode: rpm ? "fan_rpm" : "temperature",
    blink: config.blink_above !== null && out !== null && out >= config.blink_above,
  };
}

/** The displays the daemon would report. */
function displayList(): DisplayStatus[] {
  const s = st();
  const config = s.displayConfig ?? { ...DISPLAY_DEFAULT };
  return [
    {
      info: displayInfo(),
      config,
      // Nothing is written to a display before a source is saved for it.
      shown: s.displayConfig ? displayShown(s, config) : null,
    },
  ];
}

/** The daemon's own validation (crates/canvas-daemon/src/displays.rs). */
function validateDisplay(pathId: string, config: DisplayConfig): void {
  if (config.device !== pathId) {
    fail(422, "validation", `the device in the body (${config.device}) does not match the path (${pathId})`);
  }
  if (config.decimals > 1) {
    fail(422, "validation", `decimals must be 0 or 1, got ${config.decimals}`);
  }
  if (config.source.kind === "fan_rpm") {
    if (config.decimals !== 0) fail(422, "validation", "a fan RPM is shown with 0 decimals");
    if (!config.source.header.trim()) fail(422, "validation", "the fan header name is empty");
  }
  if (config.source.kind === "board_temp" && !config.source.sensor.trim()) {
    fail(422, "validation", "the board sensor name is empty");
  }
  if (config.source.kind === "fixed" && !Number.isFinite(config.source.value)) {
    fail(422, "validation", "the fixed value must be a finite number");
  }
  if (config.blink_above !== null && !Number.isFinite(config.blink_above)) {
    fail(422, "validation", "blink_above must be a finite number");
  }
}

// ---- peripherals ----

/** The daemon's own words when a peripheral's access is asked for. */
const READS_OFF = "reads are off for this device";

/**
 * Checks one value against the property's kind, the way the adapter does: the key must be
 * one the device has, it must be writable, and the value must be inside its range.
 */
function checkPeripheralValue(device: DemoPeripheral, key: string, value: PropValue): void {
  const def = device.schema.sections.flatMap((s) => s.properties).find((p) => p.key === key);
  if (!def) fail(422, "validation", `the device has no setting called ${key}`);
  if (device.schema.read_only.includes(key)) {
    fail(422, "validation", `${key} is read-only: this adapter cannot write it yet`);
  }
  const kind = def.kind;
  if (kind.type === "number" && value.type === "number") {
    if (value.value < kind.min || value.value > kind.max) {
      fail(422, "validation", `${key} is ${value.value}; it takes ${kind.min} to ${kind.max}`);
    }
    if (kind.step > 0 && ((value.value - kind.min) / kind.step) % 1 !== 0) {
      fail(422, "validation", `${key} is ${value.value}; it takes steps of ${kind.step}`);
    }
    return;
  }
  if (kind.type === "choice" && value.type === "choice") {
    if (!kind.options.includes(value.value)) {
      fail(422, "validation", `${key} has no option ${value.value}`);
    }
    return;
  }
  if (kind.type === "color" && value.type === "color") return;
  if (kind.type === "bool" && value.type === "bool") return;
  if (kind.type === "hue" && value.type === "hue") return;
  fail(422, "validation", `${key} does not take that kind of value`);
}

// ---- effect import ----

/** The daemon's own limit on an imported effect. */
const IMPORT_MAX_BYTES = 1024 * 1024;

/** The names Windows reserves, as the daemon's file name check lists them. */
const RESERVED_NAMES = [
  "con",
  "prn",
  "aux",
  "nul",
  "com1",
  "com2",
  "com3",
  "com4",
  "com5",
  "com6",
  "com7",
  "com8",
  "com9",
  "lpt1",
  "lpt2",
  "lpt3",
  "lpt4",
  "lpt5",
  "lpt6",
  "lpt7",
  "lpt8",
  "lpt9",
];

/**
 * The file name check the daemon runs before it writes anything: letters, digits, spaces,
 * `-`, `_` and `.`, ending in `.html`, and no folder and no reserved name.
 */
function checkFileName(fileName: string): string {
  const trimmed = fileName.trim();
  if (trimmed.length === 0) fail(422, "validation", "the file name is empty");
  if (trimmed !== fileName) fail(422, "validation", `"${fileName}" has a space at either end`);
  for (const ch of trimmed) {
    const ok = /[a-zA-Z0-9]/.test(ch) || ch === " " || ch === "-" || ch === "_" || ch === ".";
    if (!ok) {
      fail(422, "validation", `"${fileName}" may only hold letters, digits, spaces, '-', '_' and '.'`);
    }
  }
  if (trimmed.includes("..")) fail(422, "validation", `"${fileName}" must not hold '..'`);
  if (!trimmed.toLowerCase().endsWith(".html")) {
    fail(422, "validation", `"${fileName}" must end in .html`);
  }
  const stem = trimmed.slice(0, trimmed.length - ".html".length);
  if (stem.length === 0) fail(422, "validation", `"${fileName}" needs a name before .html`);
  if (stem.endsWith(" ") || stem.endsWith(".")) {
    fail(422, "validation", `"${fileName}" must not end in a space or a dot`);
  }
  const base = stem.split(".")[0] ?? stem;
  if (RESERVED_NAMES.includes(base.toLowerCase())) {
    fail(422, "validation", `"${fileName}" is a name Windows keeps for itself`);
  }
  return trimmed;
}

/**
 * The demo's small reader of a SignalRGB-format HTML effect: the title, the property metas
 * and the script. A file with no script does not parse, exactly as the daemon says.
 */
function parseUserEffect(id: string, html: string): DemoEffect {
  const script = /<script\b[^>]*>[\s\S]*?<\/script>/i.exec(html);
  if (!script) {
    fail(422, "validation", `the effect does not parse: effect ${id} has no script`);
  }
  const title = /<title[^>]*>([\s\S]*?)<\/title>/i.exec(html)?.[1]?.trim() ?? "";
  const name = title.length > 0 ? title : id.slice("html:".length);
  const properties: { key: string; label: string; kind: PropKind }[] = [];
  for (const match of html.matchAll(/<meta\b([^>]*)>/gi)) {
    const attrs = match[1] ?? "";
    const property = /\bproperty\s*=\s*"([^"]*)"/i.exec(attrs)?.[1];
    if (!property) continue;
    const label = /\blabel\s*=\s*"([^"]*)"/i.exec(attrs)?.[1] ?? property;
    const type = /\btype\s*=\s*"([^"]*)"/i.exec(attrs)?.[1] ?? "number";
    properties.push({ key: property, label, kind: kindFromType(type) });
  }
  const defaults: Props = {};
  for (const p of properties) defaults[p.key] = defaultValueOf(p.kind);
  const schema = {
    id,
    name,
    description: "A user effect.",
    needs: { screen: false, audio: false },
    properties: properties.map((p) => ({ key: p.key, label: p.label, kind: p.kind })),
    presets: [],
  };
  return {
    schema,
    defaults,
    summary: { id, name, description: schema.description, needs: schema.needs, favorite: false },
  };
}

/** The kind a property meta names. */
function kindFromType(type: string): PropKind {
  if (type === "color") return { type: "color", default: "#6000fc" };
  if (type === "bool") return { type: "bool", default: false };
  if (type === "hue") return { type: "hue", default: 0 };
  if (type === "choice") return { type: "choice", options: ["a", "b"], default: "a" };
  return { type: "number", min: 0, max: 100, step: 1, default: 0 };
}

/** The value a kind starts at. */
function defaultValueOf(kind: PropKind): PropValue {
  switch (kind.type) {
    case "color":
      return { type: "color", value: kind.default };
    case "number":
      return { type: "number", value: kind.default };
    case "bool":
      return { type: "bool", value: kind.default };
    case "choice":
      return { type: "choice", value: kind.default };
    case "hue":
      return { type: "hue", value: kind.default };
  }
}

// ---- router ----

export async function demoRequest(method: string, path: string, body: unknown): Promise<unknown> {
  // Yield once, as a network call would, so callers see the same async behaviour. The `slow`
  // scenario makes every answer take long enough for the editor's skeletons to show.
  await new Promise((resolve) => setTimeout(resolve, answerDelayMs()));
  // The daemon does not answer at all.
  if (daemonDown()) fail(503, "service_unavailable", "the daemon is not answering");
  // Every write is refused.
  if (method !== "GET" && saveError()) fail(500, "internal", "the daemon refused the change");

  const url = new URL(path, "http://demo.invalid");
  const pathname = decodeURIComponent(url.pathname);
  if (!pathname.startsWith(BASE)) fail(404, "not_found", `no route ${method} ${pathname}`);
  const seg = pathname.slice(BASE.length).split("/").filter(Boolean);
  const s = st();
  const bad = (): never => fail(404, "not_found", `no route ${method} ${pathname}`);

  const head = seg[0];
  if (head === "status") return method === "GET" ? statusNow() : bad();

  if (head === "output") {
    if (method === "GET") return { power: s.settings.power, brightness: s.settings.brightness };
    if (method === "PATCH") {
      const b = body as { power?: "running" | "paused" | null; brightness?: number | null };
      if (b.power) s.settings.power = b.power;
      if (typeof b.brightness === "number") s.settings.brightness = Math.max(0, Math.min(100, b.brightness));
      return { power: s.settings.power, brightness: s.settings.brightness };
    }
    return bad();
  }

  if (head === "render") {
    if (method === "GET") {
      const layout = activeLayout();
      return { fps: s.settings.fps, scale: layout.render_scale, sampling: layout.sampling };
    }
    if (method === "PATCH") {
      const b = body as { fps?: number | null; scale?: number | null; sampling?: string | null };
      if (typeof b.fps === "number") {
        if (b.fps < 10 || b.fps > 60) fail(422, "validation", `fps must be 10 to 60, got ${b.fps}`);
        s.settings.fps = b.fps;
      }
      const layout = activeLayout();
      if (b.scale === 25 || b.scale === 50 || b.scale === 100) layout.render_scale = b.scale;
      if (b.sampling === "nearest" || b.sampling === "bilinear" || b.sampling === "area_average") {
        layout.sampling = b.sampling;
      }
      return { fps: s.settings.fps, scale: layout.render_scale, sampling: layout.sampling };
    }
    return bad();
  }

  if (head === "devices") {
    if (seg.length === 1 && method === "GET") {
      // A rescan (the onboarding's "Rescan" asks with ?rescan=1) finds what was held back.
      if (url.searchParams.get("rescan") === "1" && s.held.length > 0) {
        s.devices = s.held;
        s.held = [];
      }
      const waking = openRgbConnecting() || Date.now() - s.startedAt < wakingMs();
      return { devices: waking ? [] : s.devices };
    }
    if (seg.length === 5 && seg[2] === "zones" && method === "PATCH") {
      const id = seg[1] ?? "";
      const zoneIndex = Number(seg[3]);
      const device = s.devices.find((d) => d.id === id);
      const zone = device?.zones.find((z) => z.index === zoneIndex);
      if (!device || !zone) fail(404, "not_found", `no zone ${zoneIndex} on ${id}`);
      const count = (body as { led_count: number }).led_count;
      if (!zone.resizable) fail(422, "validation", `${zone.name} is not resizable`);
      if (count < zone.min_leds || count > zone.max_leds) {
        fail(422, "validation", `${zone.name}: ${count} LEDs is outside ${zone.min_leds} to ${zone.max_leds}`);
      }
      zone.led_count = count;
      return { ok: true };
    }
    if (seg.length === 3 && seg[2] === "identify" && method === "POST") {
      const id = seg[1] ?? "";
      const device = s.devices.find((d) => d.id === id);
      if (!device) fail(404, "not_found", `no device ${id}`);
      if (!device.connected) fail(503, "service_unavailable", `${device.name} is not connected`);
      // A test page can collect the requests: `window.__identify = []` before the walk.
      (globalThis as { __identify?: unknown[] }).__identify?.push({ id, at: Date.now(), ...(body as object) });
      return { ok: true };
    }
    return bad();
  }

  if (head === "effects") {
    if (seg.length === 1 && method === "GET") {
      const list = Object.values(s.effects)
        .filter((e) => !isHiddenEffect(e.schema.id))
        .map((e) => ({ ...e.summary, favorite: s.favorites.has(e.schema.id) }));
      return { effects: list, active: s.activeEffect, failed: s.failedEffects };
    }
    if (seg[1] === "import" && method === "POST") {
      const req = body as ImportEffect;
      const fileName = checkFileName(req.file_name);
      if (req.html.length > IMPORT_MAX_BYTES) {
        fail(422, "validation", `the effect is ${req.html.length} bytes; the limit is ${IMPORT_MAX_BYTES}`);
      }
      const id = `html:${fileName.slice(0, fileName.length - ".html".length)}`;
      const existing = s.effects[id];
      if (existing && !req.replace) {
        fail(409, "conflict", `${req.file_name} is already installed; ask for a replace to overwrite it`);
      }
      const effect = parseUserEffect(id, req.html);
      s.effects[id] = effect;
      s.saved[id] = { ...effect.defaults };
      const answer: ImportEffectResponse = { effect: { ...effect.summary }, replaced: !!existing };
      return answer;
    }
    if (seg[1] === "active" && seg[2] === "props" && method === "PATCH") {
      setActiveProps(body as Props);
      return { ok: true };
    }
    const id = seg[1] ?? "";
    if (seg.length === 2 && method === "GET") {
      const e = effectOrNotFound(id);
      return {
        schema: e.schema,
        favorite: s.favorites.has(id),
        user_presets: (s.userPresets[id] ?? []).map((p) => ({ ...p, props: { ...p.props } })),
        props: currentProps(e),
      };
    }
    if (seg.length >= 3 && seg[2] === "presets") {
      const e = effectOrNotFound(id);
      const list = (s.userPresets[id] ??= []);
      const taken = (name: string, except?: string) =>
        [...e.schema.presets, ...list].some(
          (p) => p.name.toLowerCase() === name.toLowerCase() && p.name.toLowerCase() !== except?.toLowerCase(),
        );
      const clean = (raw: string): string => {
        const name = raw.trim();
        if (name.length === 0) fail(422, "validation", "a preset needs a name");
        if ([...name].length > 40) fail(422, "validation", "a preset name is at most 40 characters");
        return name;
      };
      const answer = () => ({ user_presets: list.map((p) => ({ ...p, props: { ...p.props } })) });
      if (seg.length === 3 && method === "POST") {
        const req = body as PresetCreate;
        const name = clean(req.name);
        if (taken(name)) fail(409, "conflict", `${name} is already a preset of this effect`);
        list.push({ name, props: { ...req.props } });
        return answer();
      }
      const name = seg[3] ?? "";
      const at = list.findIndex((p) => p.name === name);
      if (seg.length === 4 && method === "PUT") {
        const req = body as PresetReplace;
        const rename = req.rename_to == null ? null : clean(req.rename_to);
        const found = list[at];
        if (!found) fail(404, "not_found", `no preset named ${name}`);
        if (JSON.stringify(found.props) !== JSON.stringify(req.expected)) {
          fail(409, "conflict", `${name} was changed in another window`);
        }
        if (rename && taken(rename, name)) fail(409, "conflict", `${rename} is already a preset of this effect`);
        found.props = { ...req.props };
        if (rename) found.name = rename;
        return answer();
      }
      if (seg.length === 4 && method === "DELETE") {
        if (at < 0) fail(404, "not_found", `no preset named ${name}`);
        list.splice(at, 1);
        return answer();
      }
      return bad();
    }
    if (seg.length === 3 && seg[2] === "thumbnail" && method === "GET") {
      effectOrNotFound(id);
      return demoThumbnail(id);
    }
    if (seg.length === 3 && seg[2] === "activate" && method === "POST") {
      const e = effectOrNotFound(id);
      const req = body as { props?: Props; preset?: string | null };
      let base: Props = { ...e.defaults, ...(s.saved[id] ?? {}) };
      if (req.preset) {
        const want = req.preset.toLowerCase();
        const preset = [...e.schema.presets, ...(s.userPresets[id] ?? [])].find((p) => p.name.toLowerCase() === want);
        if (!preset) fail(404, "not_found", `no preset named ${req.preset}`);
        // A preset sits on the defaults, never on the saved values.
        base = { ...e.defaults, ...preset.props };
      }
      const next = mergeProps(base, req.props ?? {});
      s.activeEffect = id;
      const layout = activeLayout();
      layout.effect = { id, props: next };
      s.saved[id] = next;
      return { ok: true };
    }
    if (seg.length === 3 && seg[2] === "favorite" && method === "PUT") {
      effectOrNotFound(id);
      const fav = (body as { favorite: boolean }).favorite;
      if (fav) s.favorites.add(id);
      else s.favorites.delete(id);
      return { ok: true };
    }
    if (seg.length === 2 && method === "DELETE") {
      if (!id.startsWith("html:")) {
        fail(422, "validation", `${id} is not a user effect; only html: effects can be deleted`);
      }
      const e = s.effects[id];
      if (!e) fail(404, "not_found", `no user effect with the id ${id}`);
      delete s.effects[id];
      delete s.saved[id];
      delete s.userPresets[id];
      s.favorites.delete(id);
      if (s.activeEffect === id) s.activeEffect = null;
      return { ok: true };
    }
    return bad();
  }

  if (head === "peripherals") {
    if (seg.length === 1 && method === "GET") {
      return { peripherals: s.peripherals.map((p) => ({ ...p.info })) };
    }
    const id = seg[1] ?? "";
    const device = s.peripherals.find((p) => p.info.id === id);
    if (!device) fail(404, "not_found", `no peripheral with the id ${id}`);
    const access = s.peripheralAccess[id] ?? { reads: false, writes: false };
    if (seg.length === 2 && method === "GET") {
      const detail: PeripheralDetail = {
        info: { ...device.info },
        schema: device.schema,
        values: {},
        access,
        problem: READS_OFF,
      };
      if (access.reads) {
        // A mouse that is asleep answers nothing: the adapter says so.
        if (scenario().has("asleep") && device.info.kind === "mouse") {
          detail.problem = "the mouse is asleep: move it and try again";
        } else {
          detail.values = { ...device.values };
          detail.problem = null;
        }
      }
      return detail;
    }
    if (seg.length === 2 && method === "PATCH") {
      if (!access.writes) {
        fail(422, "validation", `config writes are off for ${id}: turn them on before writing`);
      }
      if (!access.reads) {
        fail(422, "validation", `config writes need reads for ${id}: turn reads on before writing`);
      }
      const props = body as Props;
      for (const [key, value] of Object.entries(props)) {
        checkPeripheralValue(device, key, value);
      }
      Object.assign(device.values, props);
      return props;
    }
    if (seg.length === 3 && seg[2] === "access" && method === "PUT") {
      const next = body as PeripheralAccess;
      if (next.writes && !next.reads) {
        fail(422, "validation", "config writes need reads: turn reads on too");
      }
      s.peripheralAccess[id] = { reads: !!next.reads, writes: !!next.writes };
      return s.peripheralAccess[id];
    }
    return bad();
  }

  if (head === "layouts") {
    if (seg.length === 1 && method === "GET") return { active: s.activeLayout, layouts: s.layouts };
    if (seg[1] === "active" && method === "PUT") {
      s.activeLayout = (body as { id: string }).id;
      return { active: s.activeLayout, layouts: s.layouts };
    }
    const id = seg[1] ?? "";
    const layout = s.layouts.find((l) => l.id === id);
    if (seg.length === 2 && method === "GET") {
      if (!layout) fail(404, "not_found", `no layout ${id}`);
      return layout;
    }
    if (seg.length === 2 && method === "PUT") {
      const next = body as Layout;
      if (next.id !== id) fail(422, "validation", "the layout id in the body does not match the path");
      if (![25, 50, 100].includes(next.render_scale)) {
        fail(422, "validation", `render scale must be 25, 50 or 100, got ${next.render_scale}`);
      }
      const idx = s.layouts.findIndex((l) => l.id === id);
      if (idx < 0) s.layouts.push(next);
      else s.layouts[idx] = next;
      if (scenario().has("persist")) {
        try {
          window.localStorage.setItem(DEMO_LAYOUTS_KEY, JSON.stringify(s.layouts));
        } catch {
          // Kept in memory only.
        }
      }
      return next;
    }
    return bad();
  }

  if (head === "fans") {
    if (seg.length === 1 && method === "GET") {
      if (!s.serviceUp) fail(503, "service_unavailable", "the fan service is not reachable");
      return applyFanScenarios(fansStatus());
    }
    if (seg[1] === "headers" && method === "PATCH") {
      if (!s.serviceUp) fail(503, "service_unavailable", "the fan service is not reachable");
      const header = seg[2] ?? "";
      if (!FAN_HEADERS.some((h) => h.id === header)) fail(404, "not_found", `no header ${header}`);
      const mode = body as FanMode;
      validateMode(header, mode, s.fanFloor);
      s.fanModes[header] = mode;
      return { ok: true, header, mode };
    }
    if (seg[1] === "profile" && method === "GET") {
      return { name: s.activeProfile };
    }
    if (seg[1] === "profile" && method === "PATCH") {
      if (!s.serviceUp) fail(503, "service_unavailable", "the fan service is not reachable");
      const name = (body as { name: string }).name;
      const profile = s.fanProfiles.find((p) => p.name === name);
      if (!profile) fail(404, "not_found", `no fan profile named ${name}`);
      s.activeProfile = name;
      for (const h of FAN_HEADERS) s.fanModes[h.id] = profile.headers[h.id] ?? { mode: "bios" };
      return { name };
    }
    if (seg[1] === "profiles" && method === "DELETE") {
      if (!s.serviceUp) fail(503, "service_unavailable", "the fan service is not reachable");
      const name = seg[2] ?? "";
      if (!s.fanProfiles.some((p) => p.name === name)) fail(404, "not_found", `no fan profile named ${name}`);
      if (name === s.activeProfile) {
        fail(409, "conflict", `${name} is the active profile; switch to another profile before deleting it`);
      }
      if (s.fanProfiles.length <= 1) fail(409, "conflict", "the last profile cannot be deleted");
      s.fanProfiles = s.fanProfiles.filter((p) => p.name !== name);
      return { ok: true };
    }
    if (seg[1] === "profiles" && method === "PUT") {
      if (!s.serviceUp) fail(503, "service_unavailable", "the fan service is not reachable");
      const name = seg[2] ?? "";
      const profile = body as FanProfile;
      if (profile.name !== name) {
        fail(422, "validation", `the profile name in the body (${profile.name}) does not match the path (${name})`);
      }
      for (const [header, mode] of Object.entries(profile.headers)) validateMode(header, mode, s.fanFloor);
      const idx = s.fanProfiles.findIndex((p) => p.name === name);
      if (idx < 0) s.fanProfiles.push(profile);
      else s.fanProfiles[idx] = profile;
      return profile;
    }
    return bad();
  }

  if (head === "settings") {
    if (method === "GET") return { ...s.settings };
    if (method === "PUT") {
      const next = body as Settings;
      if (next.fps < 10 || next.fps > 60) fail(422, "validation", `fps must be 10 to 60, got ${next.fps}`);
      s.settings = { ...next };
      return { ...s.settings };
    }
    return bad();
  }

  if (head === "onboarding" && seg[1] === "session" && method === "POST") {
    // A test page can collect the heartbeats: `window.__session = []` before the walk.
    (globalThis as { __session?: unknown[] }).__session?.push({ at: Date.now(), ...(body as object) });
    return { ok: true };
  }

  if (head === "onboarding" && seg[1] === "complete" && method === "POST") {
    s.onboardingComplete = true;
    try {
      window.localStorage.setItem(DEMO_ONBOARDED_KEY, "1");
    } catch {
      // Not remembered across a reload.
    }
    return { ok: true };
  }

  if (head === "first-run" && seg[1] === "dismiss-zone-panel" && method === "POST") {
    s.zonePanel = false;
    return { ok: true };
  }

  if (head === "displays") {
    if (seg.length === 1 && method === "GET") return displayList();
    const id = seg[1] ?? "";
    if (seg.length === 2 && method === "PUT") {
      const config = body as DisplayConfig;
      validateDisplay(id, config);
      s.displayConfig = { ...config, source: { ...config.source } };
      return s.displayConfig;
    }
    if (seg.length === 2 && method === "DELETE") {
      // The daemon answers 404 when there is no saved config; the demo lets the editor stop a
      // display that was never saved, which is the same end state.
      s.displayConfig = null;
      s.displayApiValue = null;
      return { ok: true, device_id: id };
    }
    if (seg.length === 3 && seg[2] === "value" && method === "PUT") {
      const b = body as { value: number | null };
      if (b.value !== null && !Number.isFinite(b.value)) {
        fail(422, "validation", "the value must be a finite number");
      }
      if (s.displayConfig && s.displayConfig.source.kind !== "api") {
        fail(422, "validation", `display ${id} is not set to the api source, so a value does not show`);
      }
      s.displayApiValue = b.value;
      return b;
    }
    return bad();
  }

  if (head === "config") {
    if (seg[1] === "export" && method === "GET") {
      const bundle: ExportBundle = {
        schema_version: SCHEMA_VERSION,
        exported_at: new Date().toISOString(),
        app_version: APP_VERSION,
        settings: { ...s.settings },
        layouts: { schema_version: SCHEMA_VERSION, active: s.activeLayout, layouts: s.layouts },
        zones: {
          schema_version: SCHEMA_VERSION,
          sizes: Object.fromEntries(
            s.devices.map((d) => [d.id, Object.fromEntries(d.zones.filter((z) => z.resizable).map((z) => [z.index, z.led_count]))]),
          ),
        },
        effects: {
          schema_version: SCHEMA_VERSION,
          saved: s.saved,
          presets: s.userPresets,
          favorites: [...s.favorites],
        },
        fans: {
          schema_version: SCHEMA_VERSION,
          floor_percent: s.fanFloor,
          active_profile: s.activeProfile,
          profiles: s.fanProfiles,
          boot_color: s.settings.boot_color,
        },
        displays: s.displayConfig ? [{ ...s.displayConfig }] : [],
      };
      return bundle;
    }
    if (seg[1] === "import" && method === "POST") {
      const bundle = body as ExportBundle;
      if (bundle.schema_version !== SCHEMA_VERSION) {
        fail(422, "validation", `unsupported config version ${bundle.schema_version}; this build reads version ${SCHEMA_VERSION}`);
      }
      s.settings = { ...bundle.settings };
      return { ok: true };
    }
    return bad();
  }

  return bad();
}
