// The demo backend: answers the same routes as the daemon, from in-memory state, when the page
// runs outside the Tauri host (a plain browser or the screenshot server). It mirrors the
// daemon's validation messages (crates/canvas-proto/src/fans.rs) so the inline 422 and 503 paths
// can be seen. `?state=conflict|service-down|first-run|html` changes what the status reports.
//
// The scenarios in `@/lib/demoScenarios` are wired in here too, so the editor's own states can
// be seen in a plain browser: `?state=daemon-down|openrgb-down|no-layout|device-empty|slow|
// save-error|handback:<header id>`.

import { ApiFailure } from "@/lib/errors";
import { thumbnailBytes } from "@/lib/thumb-demo";
import {
  answerDelayMs,
  applyDeviceScenarios,
  applyFanScenarios,
  applyLayoutScenarios,
  applyStatusScenarios,
  daemonDown,
  saveError,
} from "@/lib/demoScenarios";
import {
  DEVICES,
  EFFECTS,
  FAN_HEADERS,
  FAN_PROFILES,
  HTML_FIXTURE,
  INITIAL_SETTINGS,
  LAYOUT_DESK,
  type DemoEffect,
  type Props,
} from "@/lib/fixtures";
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

const BASE = "/api/v1";
const APP_VERSION = "0.1.0";
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
  activeEffect: string | null;
  layouts: Layout[];
  activeLayout: string;
  fanModes: Record<string, FanMode>;
  fanFloor: number;
  fanProfiles: FanProfile[];
  activeProfile: string;
  zonePanel: boolean;
  serviceUp: boolean;
  conflicts: Status["conflicts"];
  startedAt: number;
}

function initialState(): DemoState {
  const s = scenario();
  const effects: Record<string, DemoEffect> = {};
  for (const e of EFFECTS) effects[e.schema.id] = e;
  if (s.has("html")) effects[HTML_FIXTURE.schema.id] = HTML_FIXTURE;
  const profile = FAN_PROFILES[0];
  const fanModes: Record<string, FanMode> = {};
  for (const h of FAN_HEADERS) fanModes[h.id] = profile?.headers[h.id] ?? { mode: "bios" };
  const favorites = new Set(EFFECTS.filter((e) => e.summary.favorite).map((e) => e.schema.id));
  const layouts = applyLayoutScenarios([{ ...LAYOUT_DESK, zones: LAYOUT_DESK.zones.map((z) => ({ ...z })) }], LAYOUT_DESK.id);
  return {
    settings: { ...INITIAL_SETTINGS },
    devices: applyDeviceScenarios(DEVICES.map((d) => ({ ...d, zones: d.zones.map((z) => ({ ...z })) }))),
    effects,
    favorites,
    saved: {},
    activeEffect: "poison",
    layouts: layouts.layouts,
    activeLayout: layouts.active,
    fanModes,
    fanFloor: 20,
    fanProfiles: FAN_PROFILES.map((p) => ({ ...p, headers: { ...p.headers } })),
    activeProfile: profile?.name ?? "Default",
    zonePanel: s.has("first-run"),
    serviceUp: !s.has("service-down"),
    conflicts: s.has("conflict") ? [{ kind: "signal_rgb", process: true, service: false }] : [],
    startedAt: Date.now(),
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
  const connected = s.devices.filter((d) => d.connected).length;
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
    openrgb: "connected",
    service: down ? "down" : "connected",
    service_down_ms: down ? 14000 + elapsed : null,
    conflicts: s.conflicts,
    show_zone_panel: s.zonePanel,
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
    if (seg.length === 1 && method === "GET") return { devices: s.devices };
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
      return { ok: true };
    }
    return bad();
  }

  if (head === "effects") {
    if (seg.length === 1 && method === "GET") {
      const list = Object.values(s.effects).map((e) => ({ ...e.summary, favorite: s.favorites.has(e.schema.id) }));
      return { effects: list, active: s.activeEffect };
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
        user_presets: [],
        props: currentProps(e),
      };
    }
    if (seg.length === 3 && seg[2] === "thumbnail" && method === "GET") {
      effectOrNotFound(id);
      return thumbnailBytes(id);
    }
    if (seg.length === 3 && seg[2] === "activate" && method === "POST") {
      const e = effectOrNotFound(id);
      const req = body as { props?: Props; preset?: string | null };
      let base: Props = { ...e.defaults, ...(s.saved[id] ?? {}) };
      if (req.preset) {
        const preset = e.schema.presets.find((p) => p.name === req.preset);
        if (!preset) fail(422, "validation", `no preset named ${req.preset}`);
        base = { ...preset.props };
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

  if (head === "first-run" && seg[1] === "dismiss-zone-panel" && method === "POST") {
    s.zonePanel = false;
    return { ok: true };
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
          presets: {},
          favorites: [...s.favorites],
        },
        fans: {
          schema_version: SCHEMA_VERSION,
          floor_percent: s.fanFloor,
          active_profile: s.activeProfile,
          profiles: s.fanProfiles,
          boot_color: s.settings.boot_color,
        },
        displays: [],
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
