// The pad's input, read in the editor's own window (#375).
//
// The editor does not ask the daemon what the pad is doing. The browser already knows: the
// Gamepad API reports the pad's buttons, triggers and sticks. Asking the daemon would put a
// reader on the device while Luke plays, and the daemon only reads a device after ten seconds
// of the keyboard and mouse being idle, so the answer would be late anyway.
//
// The read is one frame at a time, in a `requestAnimationFrame` loop, and it runs only while the
// editor is in front: the loop stops when the window loses the focus, when the page is hidden,
// and when the controller panel goes away. Nothing is read while the editor is in the background.
//
// The frame mirrors the shape of the adapter's own XState (the buttons, the two triggers and the
// two sticks), so a source that comes from the daemon can take this one's place later. That
// change is a spine change and is not built here.
//
// The re-render rule: the loop keeps the last frame's numbers and writes to the page only when
// one of them moved. The triggers are compared as whole steps of 1/255 and the sticks as whole
// steps of 1/100, after a dead zone of 0.08 of the travel. At rest, with a pad held still, the
// numbers do not move, so the page is not drawn again at all.

import { useCallback, useEffect, useRef, useState } from "react";

/** Where the pad's state comes from: the browser's own list, or the stub the harness drives. */
export type PadSource = "browser" | "stub";

/** What the page says about the pad right now. */
export type PadStatus = "wake" | "live" | "gone" | "paused";

/** The words each state wears. The copy is fixed and the strip is always the same height. */
export const PAD_STATUS_WORDS: Record<PadStatus, string> = {
  live: "Pad live",
  wake: "No pad seen yet. Press any button on the pad.",
  gone: "No pad connected.",
  paused: "Input paused: the editor is in the background.",
};

/** As much of a gamepad as this reads. */
export interface PadGamepad {
  connected?: boolean;
  buttons: ArrayLike<{ pressed: boolean; value: number }>;
  axes: ArrayLike<number>;
}

/** One frame of the pad: what is held, how far the triggers are, where the sticks stand. */
export interface PadFrame {
  /** False when the pad is gone or the editor is in the background: the keys are cleared. */
  connected: boolean;
  /** The buttons, index by index. The first seventeen are the standard ones. */
  buttons: boolean[];
  /** The left trigger, 0 to 255. */
  lt: number;
  /** The right trigger, 0 to 255. */
  rt: number;
  /** The left stick, -100 to 100. */
  lx: number;
  ly: number;
  /** The right stick, -100 to 100. */
  rx: number;
  ry: number;
  /** Which source the frame came from. */
  source: PadSource;
}

/** A painter: it is called with every frame that changed, inside that frame's own callback. */
export type PadPainter = (frame: PadFrame) => void;

/** The standard gamepad buttons, in the order the browser reports them. */
export const PAD_BUTTONS = [
  "a",
  "b",
  "x",
  "y",
  "left-bumper",
  "right-bumper",
  "left-trigger",
  "right-trigger",
  "select",
  "start",
  "left-stick-click",
  "right-stick-click",
  "dpad-up",
  "dpad-down",
  "dpad-left",
  "dpad-right",
  "home",
] as const;

/** The pad's own buttons, after the standard seventeen: its four paddles, in order. */
export const PAD_PADDLE_SLOTS = ["paddle-1", "paddle-2", "paddle-3", "paddle-4"] as const;

/** The index the pad's own first button sits at. */
const PADDLE_INDEX = PAD_BUTTONS.length;

/**
 * The plan's own names, and the standard button each one is the same button as.
 *
 * The pad's map decides what a button does: L2 is mapped to the left trigger by default, L1 to
 * the left bumper, L3 to the left stick. The map is what the page shows in the list beside the
 * plan, so a key that is held lights through this table and through the map.
 */
export const PAD_SLOT_FUNCTIONS: Record<string, string> = {
  a: "a",
  b: "b",
  x: "x",
  y: "y",
  l1: "left-bumper",
  r1: "right-bumper",
  l2: "left-trigger",
  r2: "right-trigger",
  l3: "left-stick-click",
  r3: "right-stick-click",
  select: "select",
  start: "start",
  menu: "menu",
  home: "home",
  "dpad-up": "dpad-up",
  "dpad-down": "dpad-down",
  "dpad-left": "dpad-left",
  "dpad-right": "dpad-right",
};

/** The names a held button can carry: the standard ones, and the pad's own menu button. */
const LIGHTABLE = new Set<string>([...PAD_BUTTONS, "menu"]);

/** The stick's dead zone, as a share of the travel. XInput's own is 0.08. */
export const STICK_DEADZONE = 0.08;

/** The stick's readout: -100 to 100, the scale the page prints. */
export const STICK_SCALE = 100;

/** The trigger's readout: 0 to 255, the scale the page prints. */
export const TRIGGER_SCALE = 255;

/** The plan's name for a schema key: "button-a" is "a", "profile-1-paddle-2" is "paddle-2". */
export function padSlot(key: string): string {
  if (key.startsWith("button-")) return key.slice("button-".length);
  const paddle = /paddle-(\d+)$/.exec(key);
  if (paddle) return `paddle-${paddle[1]}`;
  return key;
}

/** One row of the pad's map: the button, and what it does. */
export interface PadMapRow {
  /** The schema's key, e.g. "button-a" or "profile-1-paddle-1". */
  key: string;
  /** What the button does, as the schema's choice names it. */
  target: string | null;
}

/**
 * The keys that are held, in the page's own words.
 *
 * A key is held when the button it is the same button as is held, or when the button it is
 * mapped to is held: the pad's map says what a button does, so a button mapped to the A button
 * lights when A is held.
 *
 * A paddle is never held here: XInput cannot see the four paddles, so they are drawn as ghosts.
 * When the browser does report one, the button it is mapped to is the one that lights.
 */
export function litKeys(frame: PadFrame, rows: PadMapRow[]): Set<string> {
  const lit = new Set<string>();
  if (!frame.connected) return lit;

  const held = new Set<string>();
  for (let i = 0; i < PAD_BUTTONS.length; i++) {
    const name = PAD_BUTTONS[i];
    if (name && frame.buttons[i]) held.add(name);
  }
  // A trigger with travel under it is a button that is down.
  if (frame.lt > 0) held.add("left-trigger");
  if (frame.rt > 0) held.add("right-trigger");

  // A paddle the browser can see lights the button it is mapped to.
  for (let i = 0; i < PAD_PADDLE_SLOTS.length; i++) {
    if (!frame.buttons[PADDLE_INDEX + i]) continue;
    const row = rows.find((r) => padSlot(r.key) === PAD_PADDLE_SLOTS[i]);
    const target = row?.target ?? null;
    if (target && LIGHTABLE.has(target)) held.add(target);
  }

  for (const row of rows) {
    const slot = padSlot(row.key);
    if (slot.startsWith("paddle-")) continue; // a paddle is a ghost: XInput cannot see it
    const same = PAD_SLOT_FUNCTIONS[slot];
    if (same && held.has(same)) {
      lit.add(row.key);
      continue;
    }
    if (row.target && held.has(row.target)) lit.add(row.key);
  }
  return lit;
}

/* --------------------------------------------------------------- the sources */

/** The state the stub holds. The harness sets it; the loop reads it every frame. */
export interface PadStub {
  /** False when the stub says no pad is there. */
  connected?: boolean;
  /** The button indices that are held down, the standard numbering. */
  buttons?: number[];
  /** The triggers, 0 to 1. */
  lt?: number;
  rt?: number;
  /** The sticks, -1 to 1. */
  lx?: number;
  ly?: number;
  rx?: number;
  ry?: number;
}

const STUB_KEY = "__CANVAS_RGB_PAD_STUB__";
const SOURCE_KEY = "__CANVAS_RGB_PAD__";

/** The stub the harness drives, or null when there is none. */
export function padStub(): PadStub | null {
  const holder = globalThis as unknown as Record<string, PadStub | null | undefined>;
  return holder[STUB_KEY] ?? null;
}

/**
 * Which source is in use.
 *
 * The stub is only ever chosen on purpose: a query value (`?pad=stub`) or a global the harness
 * sets. The default is always the browser's own list, so a real pad in a plain browser is never
 * replaced by the script.
 */
export function padSource(): PadSource {
  try {
    const search = typeof window === "undefined" ? "" : window.location.search;
    if (new URLSearchParams(search).get("pad") === "stub") return "stub";
    const holder = globalThis as unknown as Record<string, string | undefined>;
    if (holder[SOURCE_KEY] === "stub") return "stub";
  } catch {
    /* no window: the browser's own list is the only source there is */
  }
  return "browser";
}

let stubCache: { stub: PadStub; pads: PadGamepad[] } | null = null;

/** The stub as a gamepad, so the loop reads it the way it reads a real one. */
function stubPads(): PadGamepad[] | null {
  const stub = padStub();
  if (!stub || stub.connected === false) return null;
  if (stubCache && stubCache.stub === stub) return stubCache.pads;
  const held = new Set(stub.buttons ?? []);
  const count = PAD_BUTTONS.length + PAD_PADDLE_SLOTS.length;
  const buttons = Array.from({ length: count }, (_, i) => ({
    pressed: held.has(i),
    value: i === 6 ? (stub.lt ?? 0) : i === 7 ? (stub.rt ?? 0) : held.has(i) ? 1 : 0,
  }));
  const pads: PadGamepad[] = [
    {
      connected: true,
      buttons,
      axes: [stub.lx ?? 0, stub.ly ?? 0, stub.rx ?? 0, stub.ry ?? 0],
    },
  ];
  stubCache = { stub, pads };
  return pads;
}

/* --------------------------------------------------------------- the hook */

/** How the hook reads the pads. */
export interface PadInputOptions {
  /** The pads to read. The default is the browser's own list. */
  getGamepads?: () => ArrayLike<PadGamepad | null> | null | undefined;
  /** Which source is in use. The default is the switch the page carries. */
  source?: PadSource;
}

/** What the page knows about the pad. */
export interface PadInput {
  /** What the page says about the pad right now. */
  status: PadStatus;
  /** True while a pad is in the list and the editor is in front. */
  live: boolean;
  /** The last frame, or null before the first one. */
  frame: PadFrame | null;
  /** True when the source is the stub, so the page can say that the pad is simulated. */
  simulated: boolean;
  /** Adds a painter. It is called with every frame that changed, and with the frame it has now. */
  register(painter: PadPainter): () => void;
}

/** The numbers a frame carries, in the units the change rule compares them in. */
interface Numbers {
  mask: number;
  lt: number;
  rt: number;
  lx: number;
  ly: number;
  rx: number;
  ry: number;
}

function sameNumbers(a: Numbers | null, b: Numbers): boolean {
  if (!a) return false;
  return (
    a.mask === b.mask &&
    a.lt === b.lt &&
    a.rt === b.rt &&
    a.lx === b.lx &&
    a.ly === b.ly &&
    a.rx === b.rx &&
    a.ry === b.ry
  );
}

/** The stick's dead zone: under it the stick stands in the middle. */
function deadened(x: number, y: number): [number, number] {
  if (Math.hypot(x, y) < STICK_DEADZONE) return [0, 0];
  return [x, y];
}

/** What the pad holds now, in the units the rule compares. */
function readNumbers(pad: PadGamepad): Numbers {
  let mask = 0;
  const count = Math.min(pad.buttons.length, PAD_BUTTONS.length + PAD_PADDLE_SLOTS.length);
  for (let i = 0; i < count; i++) {
    const button = pad.buttons[i];
    if (button && button.pressed) mask |= 1 << i;
  }
  const axis = (i: number): number => (pad.axes.length > i ? (pad.axes[i] ?? 0) : 0);
  const trigger = (i: number) => {
    const button = pad.buttons.length > i ? pad.buttons[i] : undefined;
    const value = button ? button.value : 0;
    return Math.max(0, Math.min(TRIGGER_SCALE, Math.round(value * TRIGGER_SCALE)));
  };
  const [lx, ly] = deadened(axis(0), axis(1));
  const [rx, ry] = deadened(axis(2), axis(3));
  return {
    mask,
    lt: trigger(6),
    rt: trigger(7),
    lx: Math.round(lx * STICK_SCALE),
    ly: Math.round(ly * STICK_SCALE),
    rx: Math.round(rx * STICK_SCALE),
    ry: Math.round(ry * STICK_SCALE),
  };
}

/** The frame the numbers make. */
function toFrame(n: Numbers, source: PadSource): PadFrame {
  const buttons: boolean[] = [];
  for (let i = 0; i < PAD_BUTTONS.length + PAD_PADDLE_SLOTS.length; i++) {
    buttons.push((n.mask & (1 << i)) !== 0);
  }
  return {
    connected: true,
    buttons,
    lt: n.lt,
    rt: n.rt,
    lx: n.lx,
    ly: n.ly,
    rx: n.rx,
    ry: n.ry,
    source,
  };
}

/** The frame that clears everything: no button is held, and no travel is shown. */
function emptyFrame(source: PadSource): PadFrame {
  return {
    connected: false,
    buttons: [],
    lt: 0,
    rt: 0,
    lx: 0,
    ly: 0,
    rx: 0,
    ry: 0,
    source,
  };
}

/**
 * True when the page is on the screen.
 *
 * `document.hasFocus()` is not the rule: it answers false in a document that is not in a window
 * (a test document, a headless browser), and the editor's window tells the page when it loses
 * the focus with a `blur` of its own. So the focus is the window's own event, and this is only
 * the page's own visibility.
 */
function pageVisible(): boolean {
  if (typeof document === "undefined") return true;
  return document.visibilityState !== "hidden";
}

/**
 * Reads the pad, in the editor's own window.
 *
 * The loop runs while the editor is in front and the controller panel is on the page. It stops
 * on `blur`, when the page is hidden, and when the panel goes away, and the cleanup cancels the
 * frame that is pending and takes every listener off again.
 */
export function usePadInput(options: PadInputOptions = {}): PadInput {
  const optionsRef = useRef(options);
  const painters = useRef(new Set<PadPainter>());
  const frameRef = useRef<PadFrame | null>(null);
  const [status, setStatus] = useState<PadStatus>("wake");
  const [frame, setFrame] = useState<PadFrame | null>(null);
  const [source, setSource] = useState<PadSource | null>(null);

  // The options are read when the loop starts, and again when it is started again.
  useEffect(() => {
    optionsRef.current = options;
  });

  const register = useCallback((painter: PadPainter) => {
    painters.current.add(painter);
    const current = frameRef.current;
    if (current) painter(current);
    return () => {
      painters.current.delete(painter);
    };
  }, []);

  useEffect(() => {
    const opts = optionsRef.current;
    const src = opts.source ?? padSource();
    setSource(src);

    let raf: number | null = null;
    let seen = false;
    // The window's own focus events say whether the editor is in front; the page's own
    // visibility says whether it is on the screen. A window that is not in front reports a
    // `blur`, and that stops the loop.
    let focused = true;
    let visible = pageVisible();
    let numbers: Numbers | null = null;

    const read = (): ArrayLike<PadGamepad | null> | null => {
      if (opts.getGamepads) return opts.getGamepads() ?? null;
      if (src === "stub") return stubPads();
      if (typeof navigator === "undefined" || typeof navigator.getGamepads !== "function") return null;
      return navigator.getGamepads();
    };

    const first = (): PadGamepad | null => {
      const pads = read();
      if (!pads) return null;
      for (let i = 0; i < pads.length; i++) {
        const pad = pads[i];
        if (pad && pad.connected !== false) return pad;
      }
      return null;
    };

    const paint = (next: PadFrame) => {
      frameRef.current = next;
      for (const painter of painters.current) painter(next);
    };

    /** Nothing is held: the keys are cleared and the frame says so. */
    const clear = () => {
      numbers = null;
      const empty = emptyFrame(src);
      paint(empty);
      setFrame(empty);
    };

    const stop = () => {
      if (raf !== null) {
        window.cancelAnimationFrame(raf);
        raf = null;
      }
    };

    const tick = () => {
      raf = null;
      const pad = first();
      if (!pad) {
        stop();
        setStatus(seen ? "gone" : "wake");
        clear();
        return;
      }
      seen = true;
      setStatus("live");
      const next = readNumbers(pad);
      if (!sameNumbers(numbers, next)) {
        numbers = next;
        const drawn = toFrame(next, src);
        paint(drawn);
        setFrame(drawn);
      }
      raf = window.requestAnimationFrame(tick);
    };

    const start = () => {
      if (raf === null && focused && visible) raf = requestAnimationFrame(tick);
    };

    const onConnected = () => {
      seen = true;
      setStatus("live");
      start();
    };
    const onDisconnected = () => {
      seen = true;
      stop();
      setStatus("gone");
      clear();
    };
    const onBlur = () => {
      focused = false;
      stop();
      setStatus("paused");
      clear();
    };
    const onFocus = () => {
      focused = true;
      if (!visible) return;
      if (first()) {
        setStatus("live");
        start();
      } else {
        setStatus(seen ? "gone" : "wake");
      }
    };
    const onVisibility = () => {
      visible = document.visibilityState !== "hidden";
      if (!visible) {
        stop();
        setStatus("paused");
        clear();
        return;
      }
      if (!focused) return;
      if (first()) {
        setStatus("live");
        start();
      } else {
        setStatus(seen ? "gone" : "wake");
      }
    };

    window.addEventListener("blur", onBlur);
    window.addEventListener("focus", onFocus);
    window.addEventListener("gamepadconnected", onConnected);
    window.addEventListener("gamepaddisconnected", onDisconnected);
    document.addEventListener("visibilitychange", onVisibility);

    // A pad that is already in the list starts the loop; one that is not waits for the event.
    if (first()) start();

    return () => {
      stop();
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("gamepadconnected", onConnected);
      window.removeEventListener("gamepaddisconnected", onDisconnected);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, []);

  return {
    status,
    live: status === "live",
    frame,
    simulated: source === "stub",
    register,
  };
}
