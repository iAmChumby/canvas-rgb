// The demo backend: answers the same routes as the daemon, from in-memory state, when the page
// runs outside the Tauri host (a plain browser or the screenshot server). It mirrors the
// daemon's validation messages (crates/canvas-proto/src/fans.rs) so the inline 422 and 503 paths
// can be seen. `?state=conflict|service-down|first-run|html` changes what the status reports.
//
// The scenarios in `@/lib/demoScenarios` are wired in here too, so the editor's own states can
// be seen in a plain browser: `?state=daemon-down|openrgb-down|no-layout|device-empty|slow|
// save-error|handback:<header id>`.

import { ApiFailure } from "@/lib/errors";
import { demoThumbnail, thumbnailBytes } from "@/lib/thumb-demo";
import {
  answerDelayMs,
  applyDeviceScenarios,
  applyFanScenarios,
  applyLayoutScenarios,
  applyStatusScenarios,
  conflictClearsMs,
  daemonDown,
  emptyArea,
  failsArea,
  holdMs,
  slowRead,
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
  INITIAL_SETTINGS,
  ONBOARD_EFFECTS,
  isHiddenEffect,
  LAYOUT_DESK,
  CHIONE_DISPLAY,
  DISPLAY_DEFAULT,
  type DemoEffect,
  type Props,
} from "@/lib/fixtures";
import { demoHtmlEffects, demoSource, parseUserEffect } from "@/lib/demoEffects";
import { scriptBody } from "@/lib/effectSource";
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
import type { ReadProgress } from "@/proto/ReadProgress";
import type { ImportEffect } from "@/proto/ImportEffect";
import type { ImportEffectResponse } from "@/proto/ImportEffectResponse";
import type { FailedEffect } from "@/proto/FailedEffect";
import type { EffectError } from "@/proto/EffectError";
import type { EffectPreviewRequest } from "@/proto/EffectPreviewRequest";
import type { EffectPreviewResponse } from "@/proto/EffectPreviewResponse";
import type { DraftRequest } from "@/proto/DraftRequest";
import type { DraftStatus } from "@/proto/DraftStatus";

const BASE = "/api/v1";
const DEMO_LAYOUTS_KEY = "canvas-rgb.demo-layouts";
const DEMO_ONBOARDED_KEY = "canvas-rgb.demo-onboarded";
const APP_VERSION = "0.5.0";
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
  /** The draft on the lights, in memory only, as the daemon holds one. */
  draft: {
    html: string;
    props: Props;
    hash: string;
    leaseUntil: number;
  } | null;
  /** The preview requests answered, so a test can count them. */
  previews: number;
  /** The draft requests made, so a test can count them and read what they carried. */
  draftCalls: { method: string; at: number; props?: Props; htmlBytes?: number }[];
  /** The sentence the next draft ending carries (a prior that could not be restored). */
  draftNote: string | null;
  /** `?state=draft-running` has put its draft on the lights once. */
  draftSeeded: boolean;
  /** `?state=draft-stop-fail` has refused one Stop already. */
  draftStopFailed: boolean;
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
    // The user effect of `?state=html`, so its delete asks what to do with the presets.
    "html:fixture-sample": [{ name: "Calm", props: { speed: n(12) } }],
  };
}

function initialState(): DemoState {
  const s = scenario();
  const effects: Record<string, DemoEffect> = {};
  for (const e of EFFECTS) effects[e.schema.id] = e;
  for (const e of ONBOARD_EFFECTS) effects[e.schema.id] = e;
  if (s.has("html")) for (const e of demoHtmlEffects()) effects[e.schema.id] = e;
  // `?state=stock`: the two stock HTML effects a fresh install ships (the copy flow's second
  // source). The fixture sample stays out unless `html` is asked for too.
  if (s.has("stock")) {
    for (const e of demoHtmlEffects()) {
      if (e.schema.id !== "html:fixture-sample") effects[e.schema.id] = e;
    }
  }
  // `?state=shadowed`: a built-in preset takes a name the owner's own preset already has, as a
  // new build's preset table can. The user key is shown as shadowed (D2.2, A8).
  if (s.has("shadowed")) {
    const fixture = effects["html:fixture-sample"];
    if (fixture) fixture.schema.presets = [{ name: "Calm", props: { ...fixture.defaults } }];
  }
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
    // `?state=reads-on` starts with reads allowed for every device, as a configured PC has.
    peripheralAccess: s.has("reads-on")
      ? Object.fromEntries(PERIPHERALS.map((p) => [p.info.id, { reads: true, writes: false }]))
      : {},
    // A user effect file that did not load: the author sees why.
    // `?state=no-failed` has none, as most PCs do.
    failedEffects: s.has("no-failed") ? [] : [
      {
        file: "neon-grid.html",
        error: "the effect does not parse: effect html:neon-grid has no script",
      },
    ],
    // `?state=draft-running`: a draft is already on the lights when the page opens, as a reload
    // during a draft finds it.
    draft: s.has("draft-running") ? seededDraft() : null,
    draftSeeded: s.has("draft-running"),
    // `?state=draft-note`: the prior effect cannot be put back, so an ending says so.
    draftNote: s.has("draft-note")
      ? "the effect the draft took over from could not be restored: its file is gone, so the lights are black"
      : null,
    previews: 0,
    draftCalls: [],
    draftStopFailed: false,
  };
}

let state: DemoState | null = null;
function st(): DemoState {
  if (!state) state = initialState();
  return state;
}

function fail(status: number, code: string, message: string, retryAfterMs?: number): never {
  throw new ApiFailure(status, code, message, retryAfterMs);
}

/** The daemon's `source_hash`: FNV-1a over the bytes, 16 hex characters. */
export function demoHash(source: string): string {
  let hash = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  for (let i = 0; i < source.length; i++) {
    hash ^= BigInt(source.charCodeAt(i));
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}

/** A binary loop as the base64 the preview route answers with. */
function base64Of(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let text = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(text);
}

/**
 * `?state=source-error`: the source the editor opens throws on its first drawn frame, so the
 * preview's error, its list and its gutter can be seen (and tested) without typing.
 */
function withThrow(source: string): string {
  return source.replace(/(function draw\(\) \{\n)/, '$1    throw new Error("the ink ran out");\n');
}

/**
 * What the demo answers a preview with. The schema comes from the draft itself; an error is
 * reported where the source writes `throw` (the demo has no JavaScript engine, so a real
 * runtime's line is simulated there); the check's "never moves" warning is read from the
 * source; and the loop is the demo's generic one, as the daemon renders a draft.
 */
function previewAnswer(req: EffectPreviewRequest, s: DemoState): EffectPreviewResponse {
  s.previews += 1;
  (globalThis as { __preview?: unknown[] }).__preview?.push({
    at: Date.now(),
    html: req.html,
    props: req.props,
  });
  if (scenario().has("preview-busy")) {
    fail(503, "service_unavailable", "the daemon is already rendering 2 previews; try again shortly", 400);
  }
  const hash = demoHash(req.html);
  const id = `draft:${hash}`;
  const effect = parseUserEffect(id, req.html);
  const errors: EffectError[] = [];
  // `?state=preview-late-error`: the first draft draws, every later one fails the way a frame
  // callback that runs past its deadline does. The last good loop is what the editor keeps.
  if (scenario().has("preview-late-error") && s.previews > 1) {
    errors.push({
      message: "Error: the frame callback ran past its deadline and was stopped",
      line: 4,
      column: 1,
    });
  }
  const script = scriptBody(req.html);
  if (script) {
    script.body.split(/\r?\n/).forEach((text, i) => {
      if (!/^\s*throw\b/.test(text)) return;
      errors.push({
        message: "Error: the effect threw where the runtime stopped",
        line: i + 1,
        column: text.indexOf("throw") + 1,
      });
    });
  }
  const warnings: string[] = [];
  if (script && !/requestAnimationFrame/.test(script.body)) {
    warnings.push("the picture never moves: the effect asks for no more frames");
  }
  return {
    schema: { ...effect.schema, id },
    source_hash: hash,
    errors,
    warnings,
    thumbnail: base64Of(thumbnailBytes(id)),
  };
}

/** The draft `?state=draft-running` finds on the lights: the sample's source, no props. */
function seededDraft(): NonNullable<DemoState["draft"]> {
  const html = demoSource("html:fixture-sample")?.source ?? "";
  return { html, props: {}, hash: demoHash(html), leaseUntil: Date.now() + 15_000 };
}

/** The draft on the lights, as the daemon answers `GET`/`PUT`/`DELETE /effects/draft`. */
function draftStatus(s: DemoState): DraftStatus {
  if (s.draft && s.draft.leaseUntil <= Date.now()) s.draft = null;
  const draft = s.draft;
  // `?state=fast-draft`: the same beats, a few times quicker, so the states can be watched (and
  // tested) without waiting out a 5 s heartbeat and a 15 s lease. `?state=slow-draft` is a
  // second interval, so a test can tell that the beat follows the answer and not a constant.
  const fast = scenario().has("fast-draft");
  const slow = scenario().has("slow-draft");
  const heartbeat = fast ? 300 : slow ? 600 : 5000;
  return {
    active: draft !== null,
    ...(draft ? { id: `draft:${draft.hash}`, source_hash: draft.hash } : {}),
    props: draft ? { ...draft.props } : {},
    lease_remaining_ms: draft ? Math.max(0, draft.leaseUntil - Date.now()) : 0,
    heartbeat_ms: heartbeat,
    lease_ms: fast ? 900 : 15000,
  };
}

/** Ends the draft and answers with the note the ending carries, when it has one. */
function endDraft(s: DemoState): DraftStatus {
  const note = s.draftNote ?? undefined;
  s.draftNote = null;
  s.draft = null;
  return {
    active: false,
    props: {},
    lease_remaining_ms: 0,
    heartbeat_ms: scenario().has("fast-draft") ? 300 : 5000,
    lease_ms: scenario().has("fast-draft") ? 900 : 15000,
    ...(note ? { note } : {}),
  };
}

function activeLayout(): Layout {
  const found = st().layouts.find((l) => l.id === st().activeLayout);
  if (!found) fail(404, "not_found", "no active layout");
  return found;
}

// ---- a slow device read (`?state=slow-read`) ----

/** When each open read started. */
const readStartedAt = new Map<string, number>();
/** Opening, waking, then five reports a little under a second apart. */
const OPEN_MS = 700;
const WAKE_MS = 1500;
const REPORT_MS = 900;
const REPORTS = 5;
const SLOW_READ_TOTAL_MS = OPEN_MS + WAKE_MS + REPORT_MS * REPORTS;

/** What the daemon's read route would say for this device now. */
function readProgressNow(id: string): ReadProgress {
  const started = readStartedAt.get(id);
  if (started === undefined) return { stage: "idle", done: 0, total: 0 };
  const t = Date.now() - started;
  if (t < OPEN_MS) return { stage: "opening", done: 0, total: 0 };
  if (t < OPEN_MS + WAKE_MS) return { stage: "waking", done: 0, total: 0 };
  const done = Math.min(REPORTS, Math.floor((t - OPEN_MS - WAKE_MS) / REPORT_MS));
  return { stage: "reading", done, total: REPORTS };
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
  // Every device the daemon knows about, connected or not: the editor draws one card per device
  // while the list is on its way.
  const total = waking ? 0 : s.devices.length;
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
    devices_total: total,
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
  // A test can see that the editor never patches the active effect while a draft runs.
  (globalThis as { __patches?: unknown[] }).__patches?.push({ at: Date.now(), props: over });
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

// ---- router ----

export async function demoRequest(method: string, path: string, body: unknown): Promise<unknown> {
  // Yield once, as a network call would, so callers see the same async behaviour. The `slow`
  // scenario makes every answer take long enough for the editor's skeletons to show.
  await new Promise((resolve) => setTimeout(resolve, answerDelayMs()));
  // The daemon does not answer at all.
  if (daemonDown()) fail(503, "service_unavailable", "canvas-rgb is not answering");
  // Every write is refused.
  if (method !== "GET" && saveError()) fail(500, "internal", "could not save the effects: access is denied (os error 5)");

  const url = new URL(path, "http://demo.invalid");
  const pathname = decodeURIComponent(url.pathname);
  if (!pathname.startsWith(BASE)) fail(404, "not_found", `no route ${method} ${pathname}`);
  const seg = pathname.slice(BASE.length).split("/").filter(Boolean);
  const s = st();
  const bad = (): never => fail(404, "not_found", `no route ${method} ${pathname}`);

  const head = seg[0];
  // The loading states: an area's GETs can be held, made to fail or made to come back empty,
  // so the editor's placeholder, error and empty states can be seen (see demoScenarios).
  if (method === "GET" && head && head !== "status") {
    const area =
      head === "effects" && seg.length === 2 && seg[1] !== "active"
        ? "effect"
        : head === "peripherals" && seg.length === 2
          ? "peripheral"
          : head;
    const wait = holdMs(area);
    if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
    // The daemon's own words: the route that failed and why, the way the service says it.
    if (failsArea(area)) fail(500, "internal", `could not read the ${area}: the request came back with an error`);
    if (emptyArea(area)) {
      if (area === "peripherals") return { peripherals: [] };
      if (area === "displays") return [];
      if (area === "effects") return { effects: [], active: null, failed: [] };
    }
  }
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
      (globalThis as { __imports?: unknown[] }).__imports?.push({
        file_name: fileName,
        replace: req.replace,
        at: Date.now(),
      });
      if (req.html.length > IMPORT_MAX_BYTES) {
        fail(422, "validation", `the effect is ${req.html.length} bytes; the limit is ${IMPORT_MAX_BYTES}`);
      }
      const id = `html:${fileName.slice(0, fileName.length - ".html".length)}`;
      const existing = s.effects[id];
      // `?state=import-clash`: another window saved the file between this window's read of the
      // list and its Save, so the route refuses a write that did not ask to replace.
      if (!req.replace && (existing || scenario().has("import-clash"))) {
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
    if (seg[1] === "preview" && seg.length === 2 && method === "POST") {
      return previewAnswer(body as EffectPreviewRequest, s);
    }
    if (seg[1] === "draft" && seg.length === 2) {
      if (method === "GET") {
        // A reload during a draft finds the slot still there. The scenario seeds it once, so a
        // Stop is not undone by the next read.
        if (scenario().has("draft-running") && !s.draftSeeded) {
          s.draftSeeded = true;
          s.draft = seededDraft();
        }
        return draftStatus(s);
      }
      if (method === "PUT") {
        const req = body as DraftRequest;
        const call = { method, at: Date.now(), props: { ...req.props }, htmlBytes: req.html.length };
        s.draftCalls.push(call);
        (globalThis as { __draft?: unknown[] }).__draft?.push(call);
        if (req.html.length > IMPORT_MAX_BYTES) {
          fail(422, "validation", `the draft is ${req.html.length} bytes; the limit is ${IMPORT_MAX_BYTES}`);
        }
        // `?state=draft-beat-fail`: the first beat starts the draft, the rest fail, so the
        // expiring banner and the daemon's own restore can be seen.
        if (scenario().has("draft-beat-fail") && s.draft !== null) {
          fail(503, "service_unavailable", "the draft could not be renewed");
        }
        const hash = demoHash(req.html);
        // A draft that does not parse leaves a running draft alone, as the daemon's does.
        parseUserEffect(`draft:${hash}`, req.html);
        const lease = scenario().has("fast-draft") ? 900 : 15_000;
        s.draft = { html: req.html, props: { ...req.props }, hash, leaseUntil: Date.now() + lease };
        return draftStatus(s);
      }
      if (method === "DELETE") {
        const call = { method, at: Date.now() };
        s.draftCalls.push(call);
        (globalThis as { __draft?: unknown[] }).__draft?.push(call);
        // `?state=draft-stop-fail`: the first Stop cannot restore the prior effect, so the
        // restore-error state and its Retry can be seen. The next Stop goes through.
        if (scenario().has("draft-stop-fail") && !s.draftStopFailed) {
          s.draftStopFailed = true;
          fail(503, "service_unavailable", "the previous effect could not be restored; try again");
        }
        return endDraft(s);
      }
      return bad();
    }
    const id = seg[1] ?? "";
    if (seg.length === 3 && seg[2] === "source" && method === "GET") {
      // `?state=source-fail`: the file cannot be read, as a locked or moved file answers.
      if (scenario().has("source-fail")) fail(500, "internal", "could not read the effect file: access is denied (os error 5)");
      const found = demoSource(id);
      if (!found) {
        fail(404, "not_found", `${id} is a built-in effect written in Rust; it has no HTML source to edit`);
      }
      const source = scenario().has("source-error") ? withThrow(found.source) : found.source;
      return { source, file_name: found.file_name };
    }
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
      // `?state=activate-fail`: the file saved but the effect would not start, the one failure
      // that must not leave a draft on the lights while the editor says it is saved.
      if (scenario().has("activate-fail")) {
        fail(503, "service_unavailable", "the effect could not start: the engine is busy");
      }
      const req = body as { props?: Props; preset?: string | null };
      let base: Props = { ...e.defaults, ...(s.saved[id] ?? {}) };
      if (req.preset) {
        const want = req.preset.toLowerCase();
        const preset = [...e.schema.presets, ...(s.userPresets[id] ?? [])].find((p) => p.name.toLowerCase() === want);
        if (!preset) fail(404, "not_found", `no preset named ${req.preset}`);
        // A preset sits on the defaults, never on the saved values.
        base = { ...e.defaults, ...preset.props };
      }
      // An activate ends a running draft without putting the old effect back.
      s.draft = null;
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
      // The presets go with the effect unless the query keeps them.
      if (url.searchParams.get("keep_presets") !== "true") delete s.userPresets[id];
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
    // The stage of the read that is open: answers at once, like the daemon's route.
    if (seg.length === 3 && seg[2] === "read" && method === "GET") return readProgressNow(id);
    if (seg.length === 2 && method === "GET") {
      const detail: PeripheralDetail = {
        info: { ...device.info },
        schema: device.schema,
        values: {},
        access,
        problem: READS_OFF,
      };
      // `?state=slow-read`: the read goes through its stages, as a mouse that has to wake does.
      if (access.reads && slowRead()) {
        readStartedAt.set(id, Date.now());
        await new Promise((resolve) => setTimeout(resolve, SLOW_READ_TOTAL_MS));
        readStartedAt.delete(id);
      }
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
