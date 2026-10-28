// Demo-mode fixtures for the peripherals page: the two devices canvas-rgb can configure, with
// the settings they held when they were read on this machine.
//
// The values come from the live reads in .build/s2: hw-x6-read-2.txt (the Attack Shark X6
// mouse, read 2026-10-27) and hw-pad-read-4.txt (the 8BitDo Ultimate 2 controller, read
// 2026-10-27). The sections, keys, kinds and ranges are the ones the two adapters publish
// (crates/canvas-adapters/src/attack_shark_x6 and .../eightbitdo_ultimate2), so the editor
// draws exactly the shape the daemon sends.
//
// The production path never reads this file.

import type { PeripheralInfo } from "@/proto/PeripheralInfo";
import type { PeripheralSchema } from "@/proto/PeripheralSchema";
import type { PeripheralSection } from "@/proto/PeripheralSection";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { PropKind } from "@/proto/PropKind";
import type { PropValue } from "@/proto/PropValue";
import type { Props } from "@/proto/props";

export interface DemoPeripheral {
  info: PeripheralInfo;
  schema: PeripheralSchema;
  /** What the device holds now, as the read brought it back. */
  values: Props;
  /** The battery, when the device reports one the adapter can trust. */
  battery: number | null;
  /** True while the device is charging. */
  charging: boolean;
}

const numberKind = (min: number, max: number, step: number, def: number): PropKind => ({
  type: "number",
  min,
  max,
  step,
  default: def,
});
const choiceKind = (options: string[], def: string): PropKind => ({ type: "choice", options, default: def });
const colorKind = (def: string): PropKind => ({ type: "color", default: def });
const boolKind = (def: boolean): PropKind => ({ type: "bool", default: def });

const def = (key: string, label: string, kind: PropKind): PropertyDef => ({ key, label, kind });

const c = (hex: string): PropValue => ({ type: "color", value: hex });
const n = (value: number): PropValue => ({ type: "number", value });
const b = (value: boolean): PropValue => ({ type: "bool", value });
const ch = (value: string): PropValue => ({ type: "choice", value });

const section = (id: string, title: string, properties: PropertyDef[]): PeripheralSection => ({
  id,
  title,
  properties,
});

/* ------------------------------------------------------------------ the mouse (X6) */

/** The actions the mouse's buttons can be set to, as the read printed them. */
const MOUSE_ACTIONS = [
  "off",
  "left-click",
  "right-click",
  "middle-click",
  "back",
  "forward",
  "double-click",
  "fire",
  "scroll-up",
  "scroll-down",
  "dpi-cycle",
  "dpi-up",
  "dpi-down",
  "led-loop",
  "other",
];

/** The eight DPI stages the mouse holds, with the colour each one lights in. */
const MOUSE_STAGE_DPI = [800, 1200, 1600, 3200, 5600, 26000, 50, 50];
const MOUSE_STAGE_COLOUR = [
  "#ff0000",
  "#00ff00",
  "#0000ff",
  "#ffff00",
  "#00ffff",
  "#ff00ff",
  "#ff4000",
  "#ffffff",
];

/** The buttons the mouse names, and what each one did when it was read. */
const MOUSE_BUTTONS: [string, string][] = [
  ["button-1", "left-click"],
  ["button-2", "right-click"],
  ["button-3", "middle-click"],
  ["button-4", "dpi-cycle"],
  ["button-7", "forward"],
  ["button-8", "back"],
];

function mouseSchema(): PeripheralSchema {
  const dpi: PropertyDef[] = [
    def("dpi-stage-count", "DPI stages in use", numberKind(1, 8, 1, 6)),
  ];
  for (let i = 0; i < 8; i++) {
    dpi.push(def(`dpi-stage-${i + 1}`, `DPI stage ${i + 1}`, numberKind(50, 26000, 50, MOUSE_STAGE_DPI[i] ?? 50)));
    dpi.push(
      def(`dpi-stage-${i + 1}-colour`, `DPI stage ${i + 1} colour`, colorKind(MOUSE_STAGE_COLOUR[i] ?? "#ffffff")),
    );
  }
  dpi.push(
    def(
      "dpi-active-stage",
      "Active DPI stage",
      choiceKind(["1", "2", "3", "4", "5", "6", "7", "8"], "1"),
    ),
  );

  const buttons = MOUSE_BUTTONS.map(([key, action]) =>
    def(key, `Button ${key.replace("button-", "")}`, choiceKind(MOUSE_ACTIONS, action)),
  );

  return {
    device: "attack-shark-x6:1d57:fa60",
    kind: "mouse",
    sections: [
      section("dpi", "DPI", dpi),
      section("performance", "Performance", [
        def("polling", "Polling rate (Hz)", choiceKind(["125", "250", "500", "1000"], "1000")),
        def("debounce-ms", "Key response time (ms)", numberKind(4, 50, 2, 8)),
      ]),
      section("lighting", "Lighting", [
        def(
          "led-mode",
          "LED mode",
          choiceKind(
            ["off", "static", "breathing", "neon", "color-breathing", "static-dpi", "breathing-dpi"],
            "static",
          ),
        ),
        def("led-colour", "LED colour", colorKind("#ef3fff")),
        def("led-brightness", "LED brightness", numberKind(1, 8, 1, 8)),
        def("led-speed", "LED speed", numberKind(4, 8, 1, 6)),
      ]),
      section("power", "Power", [
        def("sleep-after-min", "Sleep after (min)", numberKind(0.5, 30, 0.5, 0.5)),
        def("deep-sleep-min", "Deep sleep after (min)", numberKind(1, 60, 1, 10)),
      ]),
      section("buttons", "Buttons", buttons),
      section("profile", "Profile", [def("profile", "Profile", choiceKind(["1"], "1"))]),
    ],
    read_only: [
      ...MOUSE_BUTTONS.map(([key]) => key),
      "dpi-active-stage",
      "profile",
    ],
    unreported: [
      "button-5",
      "button-6",
      "button-9",
      "button-10",
      "button-11",
      "button-12",
      "button-13",
      "button-14",
      "button-15",
      "button-16",
      "button-17",
      "button-18",
      "ripple-control",
      "angle-snap",
      "motion-sync",
      "lift-off",
    ],
  };
}

function mouseValues(): Props {
  const values: Props = {
    "dpi-stage-count": n(6),
    "dpi-active-stage": ch("1"),
    polling: ch("1000"),
    "debounce-ms": n(8),
    "led-mode": ch("static"),
    "led-colour": c("#ef3fff"),
    "led-brightness": n(8),
    "led-speed": n(6),
    "sleep-after-min": n(0.5),
    "deep-sleep-min": n(10),
    profile: ch("1"),
  };
  for (let i = 0; i < 8; i++) {
    values[`dpi-stage-${i + 1}`] = n(MOUSE_STAGE_DPI[i] ?? 50);
    values[`dpi-stage-${i + 1}-colour`] = c(MOUSE_STAGE_COLOUR[i] ?? "#ffffff");
  }
  for (const [key, action] of MOUSE_BUTTONS) values[key] = ch(action);
  return values;
}

/* --------------------------------------------------------------- the controller (U2) */

/** The targets a button can be mapped to, as the read printed them. */
const PAD_TARGETS = [
  "none",
  "start",
  "left-stick-click",
  "right-stick-click",
  "select",
  "x",
  "y",
  "dpad-right",
  "dpad-left",
  "dpad-down",
  "dpad-up",
  "left-bumper",
  "right-bumper",
  "b",
  "a",
  "left-trigger",
  "right-trigger",
  "menu",
  "home",
  "paddle-3",
  "screenshot",
  "turbo-single",
  "turbo-auto",
  "paddle-1",
  "paddle-2",
  "paddle-4",
  "swap",
  "other",
];

/** The map slots of the active profile, with the label the schema gives each one. */
const PAD_SLOTS: [string, string, string][] = [
  ["a", "A", "a"],
  ["b", "B", "b"],
  ["x", "X", "x"],
  ["y", "Y", "y"],
  ["l1", "L1", "left-bumper"],
  ["r1", "R1", "right-bumper"],
  ["l2", "L2", "left-trigger"],
  ["r2", "R2", "right-trigger"],
  ["l3", "L3", "left-stick-click"],
  ["r3", "R3", "right-stick-click"],
  ["select", "Select", "select"],
  ["start", "Start", "start"],
  ["menu", "Menu / Share", "menu"],
  ["home", "Home", "home"],
  ["dpad-up", "D-pad up", "dpad-up"],
  ["dpad-down", "D-pad down", "dpad-down"],
  ["dpad-left", "D-pad left", "dpad-left"],
  ["dpad-right", "D-pad right", "dpad-right"],
];

/** The four paddles, as the schema names them. */
const PAD_PADDLES: [number, string][] = [
  [1, "Paddle 1 (P1)"],
  [2, "Paddle 2 (P2)"],
  [3, "Paddle 3 (P3)"],
  [4, "Paddle 4 (P4)"],
];

/** The stick and trigger range rows, with the label the schema gives each one. */
const PAD_RANGES: [string, string][] = [
  ["stick-left-start", "Left stick range start (%)"],
  ["stick-left-end", "Left stick range end (%)"],
  ["stick-right-start", "Right stick range start (%)"],
  ["stick-right-end", "Right stick range end (%)"],
  ["trigger-left-start", "Left trigger range start (%)"],
  ["trigger-left-end", "Left trigger range end (%)"],
  ["trigger-right-start", "Right trigger range start (%)"],
  ["trigger-right-end", "Right trigger range end (%)"],
];

function padSchema(): PeripheralSchema {
  const profiles: PropertyDef[] = [
    def("active-profile", "Active profile", choiceKind(["1", "2", "3"], "1")),
  ];
  for (const p of [1, 2, 3]) {
    profiles.push(def(`profile-${p}-in-use`, `Profile ${p} in use`, boolKind(false)));
  }

  const buttons = PAD_SLOTS.map(([slot, label]) =>
    def(`button-${slot}`, label, choiceKind(PAD_TARGETS, "none")),
  );

  const profileSections: PeripheralSection[] = [];
  for (const p of [1, 2, 3]) {
    const properties: PropertyDef[] = [];
    for (const [suffix, label] of PAD_RANGES) {
      properties.push(def(`profile-${p}-${suffix}`, `${label} (profile ${p})`, numberKind(0, 100, 1, 0)));
    }
    for (const [side, label] of [
      ["left", "Left motor"],
      ["right", "Right motor"],
    ]) {
      properties.push(
        def(`profile-${p}-vibration-${side}`, `${label} strength (%, profile ${p})`, numberKind(0, 100, 20, 0)),
      );
    }
    for (const [n, label] of PAD_PADDLES) {
      properties.push(def(`profile-${p}-paddle-${n}`, `${label} (profile ${p})`, choiceKind(PAD_TARGETS, "none")));
    }
    profileSections.push(section(`profile-${p}`, `Profile ${p}`, properties));
  }

  const readOnly = ["active-profile"];
  for (const p of [1, 2, 3]) {
    readOnly.push(`profile-${p}-in-use`, `profile-${p}-vibration-left`, `profile-${p}-vibration-right`);
  }
  for (const [slot] of PAD_SLOTS) readOnly.push(`button-${slot}`);

  const unreported: string[] = [];
  for (const p of [1, 2, 3]) {
    unreported.push(
      `profile-${p}-name`,
      `profile-${p}-ring-theme`,
      `profile-${p}-macros`,
      `profile-${p}-gyro-mapping`,
      `profile-${p}-special-features`,
      `profile-${p}-xinput-rumble-range`,
    );
  }
  unreported.push("trigger-mode", "firmware-version", "battery-percent");

  return {
    device: "eightbitdo-ultimate-2:2dc8",
    kind: "game_controller",
    sections: [
      section("profiles", "Profiles", profiles),
      section("buttons", "Button remap (active profile)", buttons),
      ...profileSections,
    ],
    read_only: readOnly,
    unreported,
  };
}

function padValues(): Props {
  const values: Props = {
    "active-profile": ch("1"),
    "profile-1-in-use": b(true),
    "profile-2-in-use": b(false),
    "profile-3-in-use": b(false),
  };
  for (const [slot, , target] of PAD_SLOTS) values[`button-${slot}`] = ch(target);

  // What each profile held when the pad was read.
  const held: Record<number, { ranges: number[]; motors: number[]; paddles: string[] }> = {
    1: { ranges: [7, 50, 7, 50, 0, 100, 0, 100], motors: [100, 100], paddles: ["none", "none", "none", "none"] },
    2: { ranges: [100, 100, 100, 100, 100, 100, 100, 100], motors: [0, 0], paddles: ["other", "other", "other", "other"] },
    3: { ranges: [100, 100, 100, 100, 100, 100, 100, 100], motors: [0, 0], paddles: ["other", "other", "other", "other"] },
  };
  for (const p of [1, 2, 3]) {
    const row = held[p];
    if (!row) continue;
    PAD_RANGES.forEach(([suffix], i) => {
      values[`profile-${p}-${suffix}`] = n(row.ranges[i] ?? 0);
    });
    values[`profile-${p}-vibration-left`] = n(row.motors[0] ?? 0);
    values[`profile-${p}-vibration-right`] = n(row.motors[1] ?? 0);
    PAD_PADDLES.forEach(([n], i) => {
      values[`profile-${p}-paddle-${n}`] = ch(row.paddles[i] ?? "none");
    });
  }
  return values;
}

/* ------------------------------------------------------------------ the two devices */

export const PERIPHERALS: DemoPeripheral[] = [
  {
    info: {
      id: "attack-shark-x6:1d57:fa60",
      name: "Attack Shark X6",
      vendor: "Beken",
      kind: "mouse",
      adapter: "attack-shark-x6",
      serial: "",
      location: "hid:1d57:fa60:usage000b",
      connected: true,
    },
    schema: mouseSchema(),
    values: mouseValues(),
    // The mouse sends its battery with its status reports.
    battery: 87,
    charging: false,
  },
  {
    info: {
      id: "eightbitdo-ultimate-2:2dc8",
      name: "8BitDo Ultimate 2 Wireless (XInput)",
      vendor: "8BitDo",
      kind: "game_controller",
      adapter: "eightbitdo-ultimate-2",
      serial: "",
      location: "hid:2dc8:310b",
      connected: true,
    },
    schema: padSchema(),
    values: padValues(),
    // The pad's battery is not reported by this adapter yet.
    battery: null,
    charging: false,
  },
];
