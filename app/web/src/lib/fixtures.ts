// Demo-mode fixtures, used only when there is no Tauri host (a plain browser, or the
// screenshot server). They copy the shapes of the Rust side: the 12 effect schemas from
// crates/canvas-effects/src/*.rs, the devices from the spec's hardware list, and the fan
// headers from the spine. The production path never reads this file.

import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { Layout } from "@/proto/Layout";
import type { FanProfile } from "@/proto/FanProfile";
import type { Settings } from "@/proto/Settings";
import type { DisplayInfo } from "@/proto/DisplayInfo";
import type { DisplayConfig } from "@/proto/DisplayConfig";
import type { PropValue } from "@/proto/PropValue";
import type { PropKind } from "@/proto/PropKind";
import type { PropertySchema } from "@/proto/PropertySchema";

import type { Props } from "@/proto/props";

export type { Props };

export interface DemoEffect {
  schema: PropertySchema;
  defaults: Props;
  summary: EffectSummary;
}

const colorKind = (def: string): PropKind => ({ type: "color", default: def });
const numberKind = (min: number, max: number, step: number, def: number): PropKind => ({
  type: "number",
  min,
  max,
  step,
  default: def,
});
const choiceKind = (options: string[], def: string): PropKind => ({ type: "choice", options, default: def });
const hueKind = (def: number): PropKind => ({ type: "hue", default: def });

const c = (hex: string): PropValue => ({ type: "color", value: hex });
const n = (value: number): PropValue => ({ type: "number", value });
const h = (value: number): PropValue => ({ type: "hue", value });
const ch = (value: string): PropValue => ({ type: "choice", value });

type Overrides = { [key: string]: PropValue };

function build(
  id: string,
  name: string,
  description: string,
  needs: { screen: boolean; audio: boolean },
  props: { key: string; label: string; kind: PropKind }[],
  presets: { name: string; over: Overrides }[],
  favorite: boolean,
): DemoEffect {
  const defaults: Props = {};
  for (const p of props) {
    defaults[p.key] = defaultValue(p.kind);
  }
  const fullPreset = (over: Overrides): Props => {
    const merged: Props = { ...defaults };
    for (const [k, v] of Object.entries(over)) merged[k] = v;
    return merged;
  };
  const schema: PropertySchema = {
    id,
    name,
    description,
    needs,
    properties: props.map((p) => ({ key: p.key, label: p.label, kind: p.kind })),
    presets: presets.map((p) => ({ name: p.name, props: fullPreset(p.over) })),
  };
  return {
    schema,
    defaults,
    summary: { id, name, description, needs, favorite },
  };
}

function defaultValue(kind: PropKind): PropValue {
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

const NO_INPUT = { screen: false, audio: false };

/** The 12 built-in effects, in EFFECT_IDS order. */
export const EFFECTS: DemoEffect[] = [
  build("solid", "Solid", "One flat color across the whole canvas.", NO_INPUT,
    [{ key: "color", label: "Color", kind: colorKind("#6000fc") }],
    [
      { name: "Purple", over: { color: c("#6000fc") } },
      { name: "Amber", over: { color: c("#ff8c00") } },
      { name: "Teal", over: { color: c("#00d8c0") } },
    ], false),
  build("gradient", "Gradient", "A linear gradient between two colors, optionally scrolling.", NO_INPUT,
    [
      { key: "color1", label: "Color 1", kind: colorKind("#6000fc") },
      { key: "color2", label: "Color 2", kind: colorKind("#00d8ff") },
      { key: "angle", label: "Angle", kind: hueKind(90) },
      { key: "scroll_speed", label: "Scroll Speed", kind: numberKind(0, 100, 1, 0) },
    ],
    [
      { name: "Sunset", over: { color1: c("#ff2d55"), color2: c("#ffb703"), angle: h(120) } },
      { name: "Ocean", over: { color1: c("#0a0a4a"), color2: c("#1e6fd9"), angle: h(90) } },
      { name: "Scrolling", over: { scroll_speed: n(20) } },
    ], false),
  build("rainbow_wave", "Rainbow Wave", "A rainbow band that travels across the canvas.", NO_INPUT,
    [
      { key: "speed", label: "Speed", kind: numberKind(0, 100, 1, 30) },
      { key: "width", label: "Width", kind: numberKind(5, 200, 1, 60) },
      { key: "direction", label: "Direction", kind: numberKind(0, 360, 1, 0) },
      { key: "cycles", label: "Cycles", kind: numberKind(1, 8, 1, 1) },
    ],
    [
      { name: "Fast sweep", over: { speed: n(60), width: n(40) } },
      { name: "Diagonal", over: { direction: n(45), speed: n(40) } },
      { name: "Wide arc", over: { width: n(150), cycles: n(2) } },
    ], true),
  build("spiral", "Spiral", "Rotating arms of color winding around the canvas center.", NO_INPUT,
    [
      { key: "speed", label: "Speed", kind: numberKind(0, 100, 1, 25) },
      { key: "arms", label: "Arms", kind: numberKind(1, 8, 1, 3) },
      { key: "saturation", label: "Saturation", kind: numberKind(0, 100, 1, 100) },
      { key: "hue_shift", label: "Hue Shift", kind: numberKind(0, 360, 1, 0) },
    ],
    [
      { name: "Two arms", over: { arms: n(2), speed: n(35) } },
      { name: "Hot", over: { hue_shift: n(180), saturation: n(100) } },
      { name: "Slow six", over: { arms: n(6), speed: n(10) } },
    ], false),
  build("breathing", "Breathing", "A color that pulses in and out.", NO_INPUT,
    [
      { key: "color", label: "Color", kind: colorKind("#6000fc") },
      { key: "period", label: "Period", kind: numberKind(0.5, 20, 0.1, 4) },
      { key: "min_brightness", label: "Min Brightness", kind: numberKind(0, 100, 1, 5) },
    ],
    [
      { name: "Slow violet", over: { period: n(6) } },
      { name: "Fast heartbeat", over: { period: n(1), color: c("#ff2d55") } },
      { name: "Deep glow", over: { period: n(8), min_brightness: n(0) } },
    ], false),
  build("color_cycle", "Color Cycle", "A hue that sweeps around the whole spectrum.", NO_INPUT,
    [
      { key: "speed", label: "Speed", kind: numberKind(0, 100, 1, 20) },
      { key: "saturation", label: "Saturation", kind: numberKind(0, 100, 1, 100) },
      { key: "value", label: "Brightness", kind: numberKind(0, 100, 1, 100) },
    ],
    [
      { name: "Fast rainbow", over: { speed: n(60) } },
      { name: "Deep blues", over: { speed: n(10), saturation: n(90), value: n(70) } },
      { name: "Slow purple", over: { speed: n(8) } },
    ], false),
  build("aurora", "Aurora", "Layered moving noise bands in a three-color palette.", NO_INPUT,
    [
      { key: "color1", label: "Color 1", kind: colorKind("#00d8a0") },
      { key: "color2", label: "Color 2", kind: colorKind("#0066ff") },
      { key: "color3", label: "Color 3", kind: colorKind("#a300ff") },
      { key: "speed", label: "Speed", kind: numberKind(0, 100, 1, 20) },
      { key: "scale", label: "Scale", kind: numberKind(1, 20, 0.5, 6) },
      { key: "bands", label: "Bands", kind: numberKind(1, 6, 1, 3) },
    ],
    [
      { name: "Northern lights", over: { color1: c("#00ff9c"), color2: c("#0077ff"), color3: c("#c400ff") } },
      { name: "Ember", over: { color1: c("#ff5e00"), color2: c("#ffb703"), color3: c("#8a0303") } },
      { name: "Slow and wide", over: { speed: n(6), scale: n(3) } },
    ], true),
  build("rain", "Rain", "Drops falling from the top, each leaving a trail.", NO_INPUT,
    [
      { key: "drop_color", label: "Drop Color", kind: colorKind("#a8d8ff") },
      { key: "background", label: "Background", kind: colorKind("#050814") },
      { key: "density", label: "Density", kind: numberKind(1, 400, 1, 120) },
      { key: "speed", label: "Speed", kind: numberKind(10, 400, 1, 120) },
      { key: "trail_length", label: "Trail Length", kind: numberKind(2, 60, 1, 14) },
    ],
    [
      { name: "Neon rain", over: { drop_color: c("#00ffc8"), background: c("#000000") } },
      { name: "Heavy storm", over: { density: n(300), speed: n(220) } },
      { name: "Drizzle", over: { density: n(40), speed: n(60) } },
    ], false),
  build("fire", "Fire", "Classic heat propagation upward.", NO_INPUT,
    [
      { key: "intensity", label: "Intensity", kind: numberKind(0, 100, 1, 55) },
      { key: "palette", label: "Palette", kind: choiceKind(["Classic", "Blue", "Green", "Mono"], "Classic") },
      { key: "cooling", label: "Cooling", kind: numberKind(0, 100, 1, 30) },
    ],
    [
      { name: "Blue flame", over: { palette: ch("Blue") } },
      { name: "Slow burn", over: { intensity: n(30), cooling: n(50) } },
      { name: "White hot", over: { palette: ch("Mono"), intensity: n(90) } },
    ], false),
  build("screen_ambience", "Screen Ambience", "The primary monitor's picture, scaled to fill the canvas.",
    { screen: true, audio: false },
    [
      { key: "saturation", label: "Saturation Boost", kind: numberKind(0, 300, 1, 120) },
      { key: "smoothing", label: "Smoothing", kind: numberKind(0, 100, 1, 30) },
      { key: "brightness", label: "Brightness", kind: numberKind(0, 200, 1, 100) },
    ],
    [
      { name: "Punchy", over: { saturation: n(180) } },
      { name: "Soft", over: { saturation: n(90), smoothing: n(60) } },
      { name: "Bright", over: { brightness: n(140) } },
    ], false),
  build("audio_spectrum", "Audio Spectrum", "One bar per audio band, colored by a gradient.",
    { screen: false, audio: true },
    [
      { key: "color_low", label: "Low Color", kind: colorKind("#6000fc") },
      { key: "color_high", label: "High Color", kind: colorKind("#00ff9c") },
      { key: "sensitivity", label: "Sensitivity", kind: numberKind(0, 500, 1, 150) },
      { key: "decay", label: "Decay", kind: numberKind(0, 100, 1, 40) },
      { key: "peak_hold", label: "Peak Hold", kind: numberKind(0, 100, 1, 30) },
    ],
    [
      { name: "Neon", over: { color_low: c("#ff00a0"), color_high: c("#00fff0") } },
      { name: "Warm", over: { color_low: c("#ff5e00"), color_high: c("#ffe600") } },
      { name: "Punchy", over: { sensitivity: n(250), decay: n(15) } },
    ], false),
];

/** A user HTML effect, shown only with ?state=html so the HTML badge can be checked. */
export const HTML_FIXTURE: DemoEffect = build(
  "html:fixture-sample", "Sample HTML", "Demo fixture: a user HTML effect.", NO_INPUT,
  [{ key: "speed", label: "Speed", kind: numberKind(0, 100, 1, 40) }],
  [], false,
);

const zone = (index: number, name: string, kind: DeviceInfo["zones"][number]["kind"], led: number, min: number, max: number, resizable: boolean) => ({
  index,
  name,
  kind,
  led_count: led,
  min_leds: min,
  max_leds: max,
  resizable,
});

/**
 * The machine as the daemon really sees it: the two ENE DRAM sticks, the HyperX Quadcast S,
 * the MSI Mystic Light with its four headers and the ROYUAN keyboard on the native adapter.
 * The ids, names, addresses and LED counts are the ones `GET /api/v1/devices` answers on
 * this PC, so the demo and the real thing look the same.
 */
export const DEVICES: DeviceInfo[] = [
  {
    id: "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x71",
    name: "ENE DRAM",
    vendor: "ENE",
    kind: "dram",
    adapter: "openrgb",
    adapter_index: 0,
    serial: "",
    location: "I2C: PawnIO SMBus PIIX4 0, address 0x71",
    connected: true,
    zones: [zone(0, "DRAM", { type: "linear" }, 8, 8, 8, false)],
  },
  {
    id: "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x73",
    name: "ENE DRAM",
    vendor: "ENE",
    kind: "dram",
    adapter: "openrgb",
    adapter_index: 1,
    serial: "",
    location: "I2C: PawnIO SMBus PIIX4 0, address 0x73",
    connected: true,
    zones: [zone(0, "DRAM", { type: "linear" }, 8, 8, 8, false)],
  },
  {
    id: "openrgb:HID: \\\\?\\HID#VID_03F0&PID_028C&MI_00#a&eb32b1c&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
    name: "HyperX Quadcast S",
    vendor: "HyperX",
    kind: "microphone",
    adapter: "openrgb",
    adapter_index: 2,
    serial: "",
    location: "HID: \\\\?\\HID#VID_03F0&PID_028C&MI_00#a&eb32b1c&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
    connected: true,
    zones: [zone(0, "Microphone", { type: "single" }, 2, 2, 2, false)],
  },
  {
    id: "openrgb:A1B2C3D4E5F6",
    name: "MSI MYSTIC LIGHT",
    vendor: "MSI",
    kind: "motherboard",
    adapter: "openrgb",
    adapter_index: 3,
    serial: "A1B2C3D4E5F6",
    location: "HID: \\\\?\\HID#VID_0000&PID_0000#0&00000000&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
    connected: true,
    zones: [
      zone(0, "JAF", { type: "linear" }, 0, 0, 240, true),
      zone(1, "JARGB 1", { type: "linear" }, 120, 0, 240, true),
      zone(2, "JARGB 2", { type: "linear" }, 120, 0, 240, true),
      zone(3, "JARGB 3", { type: "linear" }, 120, 0, 240, true),
    ],
  },
  {
    id: "royuan:3151:4015",
    name: "ROYUAN MonsGeek Keyboard",
    vendor: "ROYUAN",
    kind: "keyboard",
    adapter: "royuan",
    adapter_index: 0,
    serial: "",
    location: "hid:3151:4015:if2",
    connected: true,
    zones: [zone(0, "Keyboard", { type: "single" }, 1, 1, 1, false)],
  },
  {
    id: "openrgb:fan-hub-1",
    name: "Lian Li UNI FAN",
    vendor: "Lian Li",
    kind: "other",
    adapter: "openrgb",
    adapter_index: 4,
    serial: "fan-hub-1",
    location: "HID: \\\\?\\HID#VID_0CF2&PID_A100#fan-hub-1",
    connected: true,
    zones: [zone(0, "Front fan", { type: "linear" }, 16, 16, 16, false)],
  },
];

/** The active layout: the three JARGB headers, the keyboard, the microphone and the two
 *  DRAM sticks are placed; the MSI's fixed `JAF` header is not. */
export const LAYOUT_DESK: Layout = {
  id: "desk",
  name: "Desk",
  render_scale: 50,
  sampling: "area_average",
  zones: [
    // A diagram of the PC: the three ARGB headers as strips down the left, the front fan as a ring,
    // the RAM as two upright sticks on the right, and the keyboard and the microphone as points
    // along the bottom.
    { device_id: "openrgb:A1B2C3D4E5F6", zone_index: 1, x: 10, y: 10, width: 120, height: 10, rotation_deg: 0, reversed: false, visible: true, shape: { type: "strip" }, label: "Top strip" },
    { device_id: "openrgb:A1B2C3D4E5F6", zone_index: 2, x: 10, y: 30, width: 120, height: 10, rotation_deg: 0, reversed: false, visible: true, shape: { type: "strip" }, label: "Shelf light" },
    { device_id: "openrgb:A1B2C3D4E5F6", zone_index: 3, x: 10, y: 50, width: 120, height: 10, rotation_deg: 0, reversed: false, visible: true, shape: { type: "strip" } },
    { device_id: "openrgb:fan-hub-1", zone_index: 0, x: 160, y: 20, width: 56, height: 56, rotation_deg: 0, reversed: false, visible: true, shape: { type: "ring" } },
    { device_id: "royuan:3151:4015", zone_index: 0, x: 20, y: 140, width: 120, height: 36, rotation_deg: 0, reversed: false, visible: true, shape: { type: "point" } },
    // The zone OpenRGB used for the keyboard, before the native adapter took it over. The
    // daemon does not drive that device any more, so the zone draws nothing: the editor shows
    // it as stale.
    { device_id: "openrgb:HID: \\\\?\\HID#VID_3151&PID_4015&MI_02#9&298b2e71&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}", zone_index: 0, x: 20, y: 140, width: 120, height: 36, rotation_deg: 0, reversed: false, visible: true, shape: { type: "auto" } },
    { device_id: "openrgb:HID: \\\\?\\HID#VID_03F0&PID_028C&MI_00#a&eb32b1c&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}", zone_index: 0, x: 170, y: 120, width: 20, height: 40, rotation_deg: 0, reversed: false, visible: true, shape: { type: "point" } },
    { device_id: "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x71", zone_index: 0, x: 294, y: 36, width: 16, height: 120, rotation_deg: 0, reversed: false, visible: true, shape: { type: "strip" } },
    { device_id: "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x73", zone_index: 0, x: 270, y: 36, width: 16, height: 120, rotation_deg: 0, reversed: false, visible: true, shape: { type: "strip" } },
  ],
  // Aurora in the editor's own accents (--a1 blue, --a2 violet, --a3 teal): one calm picture
  // flowing across every device. The demo thumbnail is rendered with the same three colours.
  effect: {
    id: "aurora",
    props: { ...EFFECTS.find((e) => e.schema.id === "aurora")!.defaults, color1: c("#8fb4e8"), color2: c("#b49cf0"), color3: c("#5fd0c0") },
  },
};

export interface DemoHeader {
  id: string;
  label: string;
}

/** The headers the fan service really has on this PC. */
export const FAN_HEADERS: DemoHeader[] = [
  { id: "cpu_fan", label: "CPU fan" },
  { id: "pump_fan", label: "Pump fan" },
  { id: "sys_fan1", label: "System fan 1" },
  { id: "sys_fan2", label: "System fan 2" },
  { id: "sys_fan3", label: "System fan 3" },
  { id: "sys_fan4", label: "System fan 4" },
  { id: "sys_fan5", label: "System fan 5" },
  { id: "sys_fan6", label: "System fan 6" },
  { id: "pump_fan2", label: "Pump fan 2 (tach only)" },
  { id: "gpu_fan1", label: "GPU fan 1" },
  { id: "gpu_fan2", label: "GPU fan 2" },
];

/** Two profiles: Default and Quiet. Headers not listed stay on BIOS. */
export const FAN_PROFILES: FanProfile[] = [
  {
    name: "Default",
    headers: {
      cpu_fan: { mode: "curve", source: { type: "cpu" }, points: [
        { temp_c: 30, percent: 30 }, { temp_c: 50, percent: 45 }, { temp_c: 70, percent: 70 }, { temp_c: 85, percent: 100 },
      ] },
      pump_fan: { mode: "fixed", percent: 60 },
      sys_fan1: { mode: "curve", source: { type: "board", name: "SYSTIN" }, points: [
        { temp_c: 30, percent: 20 }, { temp_c: 45, percent: 35 }, { temp_c: 60, percent: 60 }, { temp_c: 75, percent: 100 },
      ] },
      sys_fan3: { mode: "fixed", percent: 40 },
    },
  },
  {
    name: "Quiet",
    headers: {
      cpu_fan: { mode: "curve", source: { type: "cpu" }, points: [
        { temp_c: 35, percent: 30 }, { temp_c: 55, percent: 40 }, { temp_c: 75, percent: 65 }, { temp_c: 90, percent: 100 },
      ] },
      pump_fan: { mode: "fixed", percent: 40 },
      sys_fan1: { mode: "fixed", percent: 25 },
    },
  },
];

export const INITIAL_SETTINGS: Settings = {
  fps: 30,
  black_on_lock: false,
  on_quit: "black",
  boot_color: "#6000fc",
  brightness: 100,
  power: "running",
};

/**
 * The numeric display the CHIONE adapter finds: the GAMDIAS pump head, four digits and a degree
 * mark. The id is the adapter's own (`chione:1b80:b53c`).
 */
export const CHIONE_DISPLAY: DisplayInfo = {
  id: "chione:1b80:b53c",
  name: "CHIONE Digital Display",
  digits: 4,
  max_decimals: 1,
  modes: ["temperature", "fan_rpm"],
  connected: true,
};

/** What a display shows before anything has been saved for it: nothing at all. */
export const DISPLAY_DEFAULT: DisplayConfig = {
  device: CHIONE_DISPLAY.id,
  source: { kind: "cpu_temp" },
  unit: "celsius",
  decimals: 1,
  blink_above: null,
};
