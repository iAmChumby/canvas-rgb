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
// numbers do not move, so the page is not drawn again at all, and the strip's line is written
// only when the line changes.
//
// Which pad: with one pad connected there is nothing to choose. With several, the page reads
// the pad whose browser name carries the device's own ids (Chromium prints "Vendor: 2dc8
// Product: 310b" for a pad it does not drive through XInput). An XInput pad's name carries no
// ids, so when nothing matches the page takes the first pad in the standard layout and says
// which one it is showing. A pad in a layout the browser cannot place lights no key at all.
//
// The read starts before the first pad is seen: while the editor is in front and the list is
// empty, the loop asks the list once every fifteenth frame, so a pad whose
// `gamepadconnected` never arrives (a WebView may not send it) is picked up all the same.
//
// The stub (`?pad=stub`, or the global the harness sets) exists only in a build made for the
// screenshot harness: `NEXT_PUBLIC_PAD_STUB=1`. In the shipped export the flag is unset, the
// bundler drops the branch, and the stub's query value, global key and words are not there.

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

/**
 * The strip's second line when the pad's buttons are not in the standard layout: the indices
 * would be the wrong ones, so no key is lit and the line says why.
 */
export const PAD_UNREADABLE_WORD =
  "This pad reports its buttons in a layout the app cannot read.";

/** The strip's second line when several pads are connected and the page had to guess. */
export function showingWord(place: number, count: number): string {
  return `Showing pad ${place} of ${count}.`;
}

/** As much of a gamepad as this reads. */
export interface PadGamepad {
  connected?: boolean;
  /**
   * The browser's own name for the pad. Chromium prints the ids of a pad it does not drive
   * through XInput ("Vendor: 2dc8 Product: 310b"); an XInput pad's id carries none.
   */
  id?: string;
  /** "standard" when the buttons are in the standard layout, "" when they are not. */
  mapping?: string;
  buttons: ArrayLike<{ pressed: boolean; value: number }>;
  axes: ArrayLike<number>;
}

/** The ids of the pad the page is about, as hex, without the "0x". */
export interface PadMatch {
  /** The vendor id, e.g. "2dc8". */
  vendor: string | null;
  /** The product id, e.g. "310b"; null when only the vendor is known. */
  product: string | null;
}

/**
 * The vendor and product ids of the device on the page, from what the daemon reports it as.
 *
 * The daemon describes the pad by where it sits on the hub (`hid:2dc8:310b`), and its id ends
 * in the vendor id (`eightbitdo-ultimate-2:2dc8`). Chromium prints both in the browser's own
 * name for the pad when it is not driven through XInput, so this is what the page matches on.
 */
export function devicePadMatch(info: { id?: string; location?: string }): PadMatch | null {
  const where = /hid:([0-9a-f]{4}):([0-9a-f]{4})/i.exec(info.location ?? "");
  if (where !== null && where[1] && where[2]) {
    return { vendor: where[1].toLowerCase(), product: where[2].toLowerCase() };
  }
  // The id ends in the vendor id, and in the product id too when the adapter knows it:
  // "eightbitdo-ultimate-2:2dc8", "attack-shark-x6:1d57:fa60".
  const ids = /:([0-9a-f]{4})(?::([0-9a-f]{4}))?$/i.exec(info.id ?? "");
  if (ids !== null && ids[1]) {
    return { vendor: ids[1].toLowerCase(), product: ids[2]?.toLowerCase() ?? null };
  }
  return null;
}

/** The pads the list carries, and which of them the page is showing. */
export interface PadPick {
  /** The pad the page reads, or null when none is connected. */
  pad: PadGamepad | null;
  /** How many pads are connected. */
  count: number;
  /**
   * Where the pad sits in the connected list (1-based) when several are connected and none
   * carries this device's ids: the page had to guess, so it says which one it shows.
   */
  showing: number | null;
}

/**
 * Which pad the page reads.
 *
 * With one pad connected there is nothing to choose. With several, the page takes the pad
 * whose browser name carries the device's own ids; an XInput pad reports a name with no ids in
 * it, so when nothing matches the page takes the first pad in the standard layout and says
 * which one it is showing.
 */
export function pickPad(
  pads: ArrayLike<PadGamepad | null> | null | undefined,
  match: PadMatch | null,
): PadPick {
  if (!pads) return { pad: null, count: 0, showing: null };
  const live: PadGamepad[] = [];
  for (let i = 0; i < pads.length; i++) {
    const pad = pads[i];
    if (pad && pad.connected !== false) live.push(pad);
  }
  if (live.length === 0) return { pad: null, count: 0, showing: null };
  if (match?.vendor) {
    const vendor = match.vendor;
    const product = match.product;
    for (const pad of live) {
      const id = (pad.id ?? "").toLowerCase();
      if (!id.includes(vendor)) continue;
      if (product && !id.includes(product)) continue;
      return { pad, count: live.length, showing: null };
    }
  }
  const only = live[0];
  if (live.length === 1 && only) return { pad: only, count: 1, showing: null };
  let at = 0;
  for (let i = 0; i < live.length; i++) {
    if (live[i]?.mapping === "standard") {
      at = i;
      break;
    }
  }
  return { pad: live[at] ?? null, count: live.length, showing: at + 1 };
}

/**
 * True when the pad's buttons are the standard ones.
 *
 * Chromium says "standard" for a pad in the standard layout and "" for one it cannot place,
 * whose button indices would be the wrong ones. A pad that says nothing at all (the harness's
 * own stub) is read as standard: only a pad that states another layout is refused.
 */
export function padReadable(pad: PadGamepad): boolean {
  return pad.mapping === undefined || pad.mapping === "standard";
}

/** One frame of the pad: what is held, how far the triggers are, where the sticks stand. */
export interface PadFrame {
  /** False when the pad is gone or the editor is in the background: the keys are cleared. */
  connected: boolean;
  /** False when the pad's buttons are not in the standard layout: no key is lit. */
  readable: boolean;
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

/**
 * The travel, of 255, at which a trigger counts as pressed for the keys: about 10%.
 *
 * The travel fill and the number beside it stay raw, so 1/255 of resting noise still shows as
 * one step; only the key is kept quiet until the trigger has really moved.
 */
export const TRIGGER_PRESS_AT = 26;

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
  // A pad whose buttons are not in the standard layout reports them at other indices: the keys
  // it would light are the wrong ones, so none is lit and the strip says why.
  if (!frame.readable) return lit;

  const held = new Set<string>();
  for (let i = 0; i < PAD_BUTTONS.length; i++) {
    const name = PAD_BUTTONS[i];
    if (name && frame.buttons[i]) held.add(name);
  }
  // A trigger with 10% of its travel under it is a button that is down. The number beside the
  // slot stays the raw travel: only the key waits for the trigger to have moved.
  if (frame.lt >= TRIGGER_PRESS_AT) held.add("left-trigger");
  if (frame.rt >= TRIGGER_PRESS_AT) held.add("right-trigger");

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
  /** The pad's own name, for a shot of a page that has to choose between pads. */
  id?: string;
  /** "standard", or "" for a pad whose buttons the app cannot place. */
  mapping?: string;
  /**
   * How many more pads the list carries after this one, each with nothing held, so a shot can
   * show the page naming which pad it reads.
   */
  others?: number;
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

/**
 * The build-time switch for the stub.
 *
 * Next inlines `process.env.NEXT_PUBLIC_PAD_STUB` when it builds, so in the shipped editor the
 * test is `undefined === "1"`, the early return is taken, and the bundler drops what follows:
 * the query value, the global key and the stub's own words are not in the export at all. Only
 * the screenshot harness builds with `NEXT_PUBLIC_PAD_STUB=1`.
 */
function stubBuilt(): boolean {
  return process.env.NEXT_PUBLIC_PAD_STUB === "1";
}

/** The stub the harness drives, or null when there is none. */
export function padStub(): PadStub | null {
  if (!stubBuilt()) return null;
  const holder = globalThis as unknown as Record<string, PadStub | null | undefined>;
  return holder[STUB_KEY] ?? null;
}

/**
 * Which source is in use.
 *
 * The stub is only ever chosen on purpose, in a build made for the harness: a query value
 * (`?pad=stub`) or a global the harness sets. The default is always the browser's own list, so
 * a real pad in a plain browser is never replaced by the script, and the shipped app has no
 * way to reach the stub at all.
 */
export function padSource(): PadSource {
  if (!stubBuilt()) return "browser";
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
      id: stub.id,
      mapping: stub.mapping,
      buttons,
      axes: [stub.lx ?? 0, stub.ly ?? 0, stub.rx ?? 0, stub.ry ?? 0],
    },
  ];
  // The other pads a shot needs: each holds nothing, and each says it is in a layout the page
  // cannot read. That keeps the page on the pad the harness stamps: when several pads are
  // connected and none carries the device's ids, the page reads the first one it can place
  // (`pickPad`), so a readable extra would take the shot away from the pad the harness drives.
  // The page still has to name its choice: it says "Showing pad 1 of N.".
  const up = Array.from({ length: count }, () => ({ pressed: false, value: 0 }));
  for (let i = 0; i < (stub.others ?? 0); i++) {
    pads.push({ connected: true, mapping: "", buttons: up, axes: [0, 0, 0, 0] });
  }
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
  /**
   * The ids of the device on the page, so the page reads that pad and not another one. The
   * default is no match: the first pad in the list is read.
   */
  match?: PadMatch | null;
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
  /** True when the pad's buttons are not in the standard layout, so no key is lit. */
  unreadable: boolean;
  /** "Showing pad 1 of 2." when several pads are connected and none is this device. */
  showing: string | null;
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
function toFrame(n: Numbers, source: PadSource, readable: boolean): PadFrame {
  const buttons: boolean[] = [];
  for (let i = 0; i < PAD_BUTTONS.length + PAD_PADDLE_SLOTS.length; i++) {
    buttons.push((n.mask & (1 << i)) !== 0);
  }
  return {
    connected: true,
    readable,
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
    readable: true,
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
 * How many frames pass between reads of the pad list while no pad is connected.
 *
 * A browser reports a pad only after its first button press: `gamepadconnected` follows that
 * press, not the plug, and a WebView may not send the event at all. So the list has to be read
 * while it still looks empty. Reading it every frame would call `getGamepads()` at 60 Hz for
 * nothing, so the wake state reads it once every fifteenth frame (about four times a second at
 * 60 Hz); the event still moves the page to the live loop at once when it arrives.
 */
export const WAKE_POLL_EVERY = 15;

/** What the strip says, and whether the page has to be drawn for it. */
export interface StatusWrite {
  /** The line the strip says now. */
  said: PadStatus;
  /** True when the line changed, so the setter was called. */
  changed: boolean;
}

/**
 * Writes the strip's line, and only when it changed.
 *
 * The loop goes through this one step every frame, so the page is asked for a draw only on a
 * real change. It is exported so a test can hand it a counting setter: React drops a same-value
 * update before it draws anything, so a count of commits cannot show a write that repeats every
 * frame, and this is the write itself.
 */
export function writeStatus(
  said: PadStatus | null,
  next: PadStatus,
  write: (status: PadStatus) => void,
): StatusWrite {
  if (said === next) return { said: next, changed: false };
  write(next);
  return { said: next, changed: true };
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
  const [showing, setShowing] = useState<string | null>(null);

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
    // What was last drawn. Each of these is written only when it changes, so a pad held still
    // sets no state at all: React is asked 0 times a frame, not 1.
    let readableNow: boolean | null = null;
    let said: PadStatus | null = null;
    let shown: string | null = null;
    let cleared = false;
    // How many frames the wake poll has run, so the list is read once every WAKE_POLL_EVERY.
    let idle = 0;

    /** Writes the strip's line, and asks React to draw it, only when the line changes. */
    const say = (next: PadStatus) => {
      said = writeStatus(said, next, setStatus).said;
    };

    /** The strip's second line: written, and drawn, only when it changes. */
    const showLine = (next: string | null) => {
      if (shown === next) return;
      shown = next;
      setShowing(next);
    };

    const read = (): ArrayLike<PadGamepad | null> | null => {
      if (opts.getGamepads) return opts.getGamepads() ?? null;
      if (src === "stub") return stubPads();
      if (typeof navigator === "undefined" || typeof navigator.getGamepads !== "function") return null;
      return navigator.getGamepads();
    };

    /** Which pad the page reads this frame, and whether it had to guess. */
    const pick = (): PadPick => pickPad(read(), optionsRef.current.match ?? null);

    const paint = (next: PadFrame) => {
      cleared = !next.connected;
      frameRef.current = next;
      for (const painter of painters.current) painter(next);
    };

    /** Nothing is held: the keys are cleared and the frame says so, once. */
    const clear = () => {
      if (cleared) return;
      numbers = null;
      readableNow = null;
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

    /**
     * The wake state's own frame: at most one read of the list, and nothing else. It runs while
     * the editor is in front and no pad is in the list, and it moves to the live loop as soon
     * as one appears. The list is read on every WAKE_POLL_EVERYth frame, not on every frame:
     * the other frames cost no read at all, and their answer is the same as the last one, so
     * they set no state.
     */
    const wake = () => {
      if (raf !== null || !focused || !visible) return;
      raf = window.requestAnimationFrame(wakeTick);
    };

    const wakeTick = () => {
      raf = null;
      if (!focused || !visible) return;
      const poll = idle % WAKE_POLL_EVERY === 0;
      idle += 1;
      if (poll) {
        const picked = pick();
        if (picked.pad) {
          seen = true;
          say("live");
          raf = window.requestAnimationFrame(tick);
          return;
        }
      }
      say(seen ? "gone" : "wake");
      showLine(null);
      clear();
      raf = window.requestAnimationFrame(wakeTick);
    };

    const tick = () => {
      raf = null;
      const picked = pick();
      const pad = picked.pad;
      if (!pad) {
        say(seen ? "gone" : "wake");
        showLine(null);
        clear();
        // No pad in the list yet: go back to the wake poll, which reads the list every
        // WAKE_POLL_EVERYth frame, so a pad whose event never comes (the WebView may not send
        // `gamepadconnected`) is picked up all the same.
        wake();
        return;
      }
      seen = true;
      say("live");
      // Only when several pads are connected and none carries this device's ids does the page
      // say which one it is showing.
      showLine(picked.showing === null ? null : showingWord(picked.showing, picked.count));
      const readable = padReadable(pad);
      const next = readNumbers(pad);
      if (!sameNumbers(numbers, next) || readableNow !== readable) {
        numbers = next;
        readableNow = readable;
        const drawn = toFrame(next, src, readable);
        paint(drawn);
        setFrame(drawn);
      }
      raf = window.requestAnimationFrame(tick);
    };

    const start = () => {
      if (raf !== null || !focused || !visible) return;
      if (pick().pad) {
        say("live");
        raf = requestAnimationFrame(tick);
      } else {
        wake();
      }
    };

    /**
     * The editor is in front again. The strip stops saying "paused" at once: with no pad in
     * the list it says what the list holds, rather than waiting for the next polled frame.
     */
    const resume = () => {
      if (!focused || !visible) return;
      if (!pick().pad) {
        say(seen ? "gone" : "wake");
        showLine(null);
      }
      start();
    };

    const onConnected = () => {
      // The strip says "paused" while the editor is away, and nothing is read then: the change
      // the event announces is taken up by `start()` when the window comes back.
      if (!focused || !visible) return;
      seen = true;
      say("live");
      start();
    };
    const onDisconnected = () => {
      if (!focused || !visible) return;
      seen = true;
      stop();
      say("gone");
      clear();
      // The pad may come back: keep reading the list while the editor is in front.
      start();
    };
    const onBlur = () => {
      focused = false;
      stop();
      say("paused");
      clear();
    };
    const onFocus = () => {
      focused = true;
      resume();
    };
    const onVisibility = () => {
      visible = document.visibilityState !== "hidden";
      if (!visible) {
        stop();
        say("paused");
        clear();
        return;
      }
      resume();
    };

    window.addEventListener("blur", onBlur);
    window.addEventListener("focus", onFocus);
    window.addEventListener("gamepadconnected", onConnected);
    window.addEventListener("gamepaddisconnected", onDisconnected);
    document.addEventListener("visibilitychange", onVisibility);

    // A pad that is already in the list starts the live loop; no pad starts the wake poll.
    start();

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
    unreadable: frame !== null && frame.connected && !frame.readable,
    showing,
    register,
  };
}
