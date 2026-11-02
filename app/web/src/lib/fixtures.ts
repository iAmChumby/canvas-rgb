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

/**
 * The onboarding's two built-in effects. The daemon hides them from the effect list and still
 * activates them by id; the demo does the same (see `isHiddenEffect`).
 */
export const ONBOARD_EFFECTS: DemoEffect[] = [
  build("onboard-hello", "Hello", "One slow, soft wave of colour, then a calm glow.", NO_INPUT, [], [], false),
  build("onboard-sweep", "Sweep", "A bright band crosses the canvas once.", NO_INPUT, [], [], false),
];

/** An effect the effect list never shows: the onboarding's own. */
export function isHiddenEffect(id: string): boolean {
  return id.startsWith("onboard-");
}

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
 * the MSI Mystic Light with its four headers, the ROYUAN keyboard on the native adapter and the
 * fan hubs that carry the case's seven fans.
 *
 * The fan hubs are the ones a header with several fans behind it looks like: one zone with
 * forty LEDs, of which the first three fans take twelve each and the last four are unclaimed.
 * The ids, names, addresses and LED counts of the real devices are the ones
 * `GET /api/v1/devices` answers on this PC, so the demo and the real thing look the same.
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
    id: "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x75",
    name: "ENE DRAM",
    vendor: "ENE",
    kind: "dram",
    adapter: "openrgb",
    adapter_index: 9,
    serial: "",
    location: "I2C: PawnIO SMBus PIIX4 0, address 0x75",
    // A stick that was taken out of the machine: its part stays on the desk, dimmed.
    connected: false,
    zones: [zone(0, "DRAM", { type: "linear" }, 8, 8, 8, false)],
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
    zones: [zone(0, "Front fan", { type: "linear" }, 40, 40, 40, false)],
  },
  {
    id: "openrgb:fan-hub-2",
    name: "Lian Li UNI FAN",
    vendor: "Lian Li",
    kind: "other",
    adapter: "openrgb",
    adapter_index: 5,
    serial: "fan-hub-2",
    location: "HID: \\\\?\\HID#VID_0CF2&PID_A100#fan-hub-2",
    connected: true,
    zones: [zone(0, "Floor fan", { type: "linear" }, 40, 40, 40, false)],
  },
  {
    id: "openrgb:fan-hub-3",
    name: "Lian Li UNI FAN",
    vendor: "Lian Li",
    kind: "other",
    adapter: "openrgb",
    adapter_index: 6,
    serial: "fan-hub-3",
    location: "HID: \\\\?\\HID#VID_0CF2&PID_A100#fan-hub-3",
    connected: true,
    zones: [zone(0, "Rear fan", { type: "linear" }, 16, 16, 16, false)],
  },
  {
    id: "openrgb:gpu-1",
    name: "ASUS TUF Gaming",
    vendor: "ASUS",
    kind: "gpu",
    adapter: "openrgb",
    adapter_index: 7,
    serial: "gpu-1",
    location: "HID: \\\\?\\HID#VID_1B4C&PID_1133#gpu-1",
    connected: true,
    zones: [zone(0, "Graphics card", { type: "linear" }, 16, 16, 16, false)],
  },
  {
    id: "openrgb:mouse-1",
    name: "Logitech G Pro X Superlight",
    vendor: "Logitech",
    kind: "mouse",
    adapter: "openrgb",
    adapter_index: 8,
    serial: "mouse-1",
    location: "HID: \\\\?\\HID#VID_046D&PID_C08B#mouse-1",
    connected: true,
    zones: [zone(0, "Mouse", { type: "single" }, 1, 1, 1, false)],
  },
  {
    id: "openrgb:strip-1",
    name: "Arctic Liquid Freezer III",
    vendor: "Arctic",
    kind: "led_strip",
    adapter: "openrgb",
    adapter_index: 9,
    serial: "strip-1",
    location: "HID: \\\\?\\HID#VID_0CF2&PID_A101#strip-1",
    connected: true,
    zones: [zone(0, "Strip", { type: "linear" }, 60, 60, 60, false)],
  },
  {
    // A strip on a header with one zone that can be resized: the Devices list shows its Size key
    // in the device's own row, since it has no row of zones under it.
    id: "openrgb:strip-2",
    name: "NZXT Hue 2 Strip",
    vendor: "NZXT",
    kind: "led_strip",
    adapter: "openrgb",
    adapter_index: 10,
    serial: "strip-2",
    location: "HID: \\\\?\\HID#VID_1E71&PID_2002#strip-2",
    connected: true,
    zones: [zone(0, "Channel 1", { type: "linear" }, 30, 0, 120, true)],
  },
];

/**
 * The demo desk: Luke's own PC, drawn the way the new editor draws it.
 *
 * The case is a box with a handle on every side (a landmark, not a form-factor picker). Inside
 * it: three fans at the front pointing at the reader, three fans on the floor pointing up, a
 * fan at the back, two sticks of RAM, a graphics card and a strip along the roof. On the desk:
 * the keyboard, the mouse and the microphone.
 *
 * The three front fans share one header: the zone has forty LEDs, the fans take twelve each,
 * and the last four belong to nobody and stay black. The scale is 4 mm to the canvas unit, so
 * a 120 mm fan is thirty units and the case is 48 x 49 cm.
 */
export const LAYOUT_DESK: Layout = {
  id: "desk",
  name: "Desk",
  render_scale: 50,
  sampling: "area_average",
  case_form: "none",
  landmarks: [
    // The case: 120 x 122 units, 48 x 49 cm, standing on the desk line.
    { kind: "case", x: 100, y: 27, width: 120, height: 122, rotation_deg: 0, facing: "viewer", visible: true },
  ],
  zones: [
    // The strip along the roof of the case.
    {
      device_id: "openrgb:strip-1",
      zone_index: 0,
      x: 106,
      y: 30,
      width: 100,
      height: 5,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "strip" },
      part: { kind: "strip", facing: "viewer", mapped: false },
      label: "Roof strip",
    },
    // The three fans at the front, pointing at the reader: one header, three runs of LEDs.
    ...([0, 1, 2] as const).map((i) => ({
      device_id: "openrgb:fan-hub-1",
      zone_index: 0,
      x: 103,
      y: 38 + i * 32,
      width: 30,
      height: 30,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "ring" } as const,
      leds: { start: i * 12, count: 12 },
      part: { kind: "fan" as const, facing: "viewer" as const, mapped: false },
      label: `Front fan ${i + 1}`,
    })),
    // The three fans on the floor, pointing up: seen from the side, so they are bars.
    ...([0, 1, 2] as const).map((i) => ({
      device_id: "openrgb:fan-hub-2",
      zone_index: 0,
      x: 104 + i * 32,
      y: 136,
      width: 30,
      height: 7,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "strip" } as const,
      leds: { start: i * 12, count: 12 },
      part: { kind: "fan" as const, facing: "up" as const, mapped: false },
      label: `Floor fan ${i + 1}`,
    })),
    // The fan at the back of the case.
    {
      device_id: "openrgb:fan-hub-3",
      zone_index: 0,
      x: 186,
      y: 38,
      width: 30,
      height: 30,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "ring" },
      part: { kind: "fan", facing: "viewer", mapped: false },
      label: "Rear fan",
    },
    // The two sticks of RAM, standing on the board.
    {
      device_id: "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x71",
      zone_index: 0,
      x: 140,
      y: 42,
      width: 6,
      height: 34,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "strip" },
      part: { kind: "ram", facing: "viewer", mapped: false },
      label: "RAM 1",
    },
    {
      device_id: "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x73",
      zone_index: 0,
      x: 148,
      y: 42,
      width: 6,
      height: 34,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "strip" },
      part: { kind: "ram", facing: "viewer", mapped: false },
      label: "RAM 2",
    },
    // The stick of RAM that was taken out: its part stays where it was.
    {
      device_id: "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x75",
      zone_index: 0,
      x: 156,
      y: 42,
      width: 6,
      height: 34,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "strip" },
      part: { kind: "ram", facing: "viewer", mapped: false },
      label: "RAM 3",
    },
    // The graphics card, under the board.
    {
      device_id: "openrgb:gpu-1",
      zone_index: 0,
      x: 140,
      y: 120,
      width: 70,
      height: 14,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "strip" },
      part: { kind: "gpu", facing: "viewer", mapped: false },
    },
    // The keyboard, the mouse and the microphone, on the desk.
    {
      device_id: "royuan:3151:4015",
      zone_index: 0,
      x: 16,
      y: 158,
      width: 110,
      height: 30,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "grid", columns: 6 },
      part: { kind: "keyboard", facing: "viewer", mapped: false },
    },
    {
      device_id: "openrgb:mouse-1",
      zone_index: 0,
      x: 136,
      y: 158,
      width: 20,
      height: 30,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "point" },
      part: { kind: "mouse", facing: "viewer", mapped: false },
    },
    {
      device_id: "openrgb:HID: \\\\?\\HID#VID_03F0&PID_028C&MI_00#a&eb32b1c&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
      zone_index: 0,
      x: 166,
      y: 160,
      width: 16,
      height: 28,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: { type: "point" },
      part: { kind: "microphone", facing: "viewer", mapped: false },
    },
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
  blocked_by: null,
};

/** What a display shows before anything has been saved for it: nothing at all. */
export const DISPLAY_DEFAULT: DisplayConfig = {
  device: CHIONE_DISPLAY.id,
  source: { kind: "cpu_temp" },
  unit: "celsius",
  decimals: 1,
  blink_above: null,
};
