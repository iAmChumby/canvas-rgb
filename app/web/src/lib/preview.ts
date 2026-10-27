// The live preview feed. In the tray it is the daemon's WebSocket (binary frames: u16 LE width,
// u16 LE height, then width*height*3 RGB bytes). In a plain browser it is the running effect's
// own look, drawn from the demo shades. The socket is open only while the window is visible. The
// dominant hue of the frame is written to --live-h at no more than 2 Hz, so the chrome tints with
// the light.
//
// The feed also keeps a status the desk canvas shows: whether the socket is connecting, live or
// refused, and why. A refused socket is diagnosed against the daemon's own answer, so the canvas
// can say what is wrong instead of waiting for a frame that never comes.

import { useSyncExternalStore } from "react";
import { apiConfig } from "@/lib/api";
import { isDemo } from "@/lib/host";
import { demoFrameFor } from "@/lib/thumb-demo";
import { dominantHue, hueDistance } from "@/lib/hue";
import { getSnap } from "@/lib/store";
import { routes } from "@/proto/routes";

export interface Frame {
  width: number;
  height: number;
  rgb: Uint8Array;
}

type FrameListener = (frame: Frame) => void;

export const PREVIEW_W = 320;
export const PREVIEW_H = 200;

export type PreviewState = "idle" | "connecting" | "live" | "refused";

export interface PreviewStatus {
  /** idle: not started. connecting: the socket is opening. live: frames are arriving. */
  state: PreviewState;
  /** Why the socket was refused, in the daemon's own words. Null while it is not refused. */
  error: string | null;
  /** Frames received since the page opened. */
  frames: number;
  /** Frames per second over the last second. */
  fps: number;
}

const listeners = new Set<FrameListener>();
const statusListeners = new Set<() => void>();
let active = false;
let socket: WebSocket | null = null;
let reconnect: ReturnType<typeof setTimeout> | null = null;
let demoTimer: ReturnType<typeof setInterval> | null = null;
let lastFrame: Frame | null = null;
const frameTimes: number[] = [];
let status: PreviewStatus = { state: "idle", error: null, frames: 0, fps: 0 };
/** How many times the socket has been refused; the retry waits grow with it. */
let refusals = 0;

function setStatus(partial: Partial<PreviewStatus>): void {
  status = { ...status, ...partial };
  for (const l of statusListeners) l();
}

function emit(frame: Frame): void {
  lastFrame = frame;
  const now = performance.now();
  frameTimes.push(now);
  while (frameTimes.length > 0 && now - (frameTimes[0] ?? 0) > 2000) frameTimes.shift();
  const span = frameTimes.length > 1 ? (now - (frameTimes[0] ?? now)) / 1000 : 0;
  const fps = span > 0 ? Math.round((frameTimes.length / span) * 10) / 10 : 0;
  setStatus({ state: "live", frames: status.frames + 1, fps });
  for (const l of listeners) l(frame);
}

export function onFrame(listener: FrameListener): () => void {
  listeners.add(listener);
  if (lastFrame) listener(lastFrame);
  return () => {
    listeners.delete(listener);
  };
}

export function currentFrame(): Frame | null {
  return lastFrame;
}

export function getPreviewStatus(): PreviewStatus {
  return status;
}

function subscribeStatus(listener: () => void): () => void {
  statusListeners.add(listener);
  return () => {
    statusListeners.delete(listener);
  };
}

/** The preview feed's state, for the desk canvas. */
export function usePreviewStatus(): PreviewStatus {
  return useSyncExternalStore(
    subscribeStatus,
    () => status,
    () => status,
  );
}

function parseFrame(buf: ArrayBuffer): Frame | null {
  if (buf.byteLength < 4) return null;
  const view = new DataView(buf);
  const width = view.getUint16(0, true);
  const height = view.getUint16(2, true);
  const expected = width * height * 3;
  if (buf.byteLength - 4 !== expected) return null;
  return { width, height, rgb: new Uint8Array(buf, 4, expected) };
}

async function connect(): Promise<void> {
  if (!active || socket) return;
  let url: string;
  let baseUrl: string;
  try {
    const cfg = await apiConfig();
    baseUrl = cfg.base_url;
    const wsBase = baseUrl.replace(/^http/, "ws");
    url = `${wsBase}${routes.preview}?token=${encodeURIComponent(cfg.token)}`;
  } catch {
    scheduleReconnect();
    return;
  }
  // Once the socket has been refused the state stays "refused": a retry must not paper over the
  // reason the canvas is showing.
  if (refusals === 0) setStatus({ state: "connecting", error: null });
  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  ws.onmessage = (ev: MessageEvent) => {
    if (ev.data instanceof ArrayBuffer) {
      const frame = parseFrame(ev.data);
      if (frame) emit(frame);
    }
  };
  ws.onopen = () => {
    refusals = 0;
    setStatus({ state: "connecting", error: null });
  };
  ws.onerror = () => {
    void diagnose(baseUrl);
  };
  ws.onclose = () => {
    socket = null;
    if (active) scheduleReconnect();
  };
  socket = ws;
}

/**
 * Works out what to say about a refused preview socket. The status route is open (it needs no
 * token), so it says whether the daemon is up at all: if it answers, the refusal is about the
 * socket and nothing else.
 */
async function diagnose(baseUrl: string): Promise<void> {
  let detail: string;
  try {
    const res = await fetch(`${baseUrl}${routes.status}`, { cache: "no-store" });
    if (res.ok) {
      detail = "The daemon refused the preview socket. Every other call works.";
    } else {
      detail = `The daemon answered the status route with HTTP ${res.status}.`;
    }
  } catch {
    detail = "The daemon is not answering on 127.0.0.1:9430.";
  }
  refusals += 1;
  setStatus({ state: "refused", error: detail });
}

function scheduleReconnect(): void {
  if (reconnect || !active) return;
  // 1.2 s, then 2.4 s, 4.8 s, up to 15 s: a refused socket must not spin the console.
  const wait = Math.min(15000, 1200 * Math.pow(2, Math.min(4, refusals)));
  reconnect = setTimeout(() => {
    reconnect = null;
    void connect();
  }, wait);
}

/**
 * The demo's own frame: the effect that is running, drawn from the same shades the demo
 * thumbnails use. It used to be a decorative rainbow, which had nothing to do with the effect
 * on screen, so the chrome tinted with whatever colour the rainbow happened to average to.
 */
function demoFrame(t: number): Frame {
  // The status is null until the demo backend answers its first request; the demo's own
  // default effect is Poison.
  const id = getSnap().status?.active_effect ?? "poison";
  const rgb = demoFrameFor(id, t, PREVIEW_W, PREVIEW_H);
  return { width: PREVIEW_W, height: PREVIEW_H, rgb };
}

/** Opens the preview. Idempotent. Call stopPreview when the window hides. */
export function startPreview(): void {
  if (active) return;
  active = true;
  if (isDemo()) {
    setStatus({ state: "connecting", error: null });
    const t0 = performance.now();
    demoTimer = setInterval(() => {
      emit(demoFrame((performance.now() - t0) / 1000));
    }, 1000 / 15);
    return;
  }
  void connect();
}

export function stopPreview(): void {
  active = false;
  if (reconnect) {
    clearTimeout(reconnect);
    reconnect = null;
  }
  if (demoTimer) {
    clearInterval(demoTimer);
    demoTimer = null;
  }
  if (socket) {
    socket.onclose = null;
    socket.close();
    socket = null;
  }
  setStatus({ state: "idle", fps: 0 });
}

// ---- the dominant hue of a frame ----

/**
 * How close two samples have to be to count as the same colour, and how far the accent has to
 * move before it does move. Together they keep the chrome calm.
 */
const HUE_STEP = 10;
/** How many samples in a row have to agree before the accent follows them. */
const HUE_AGREEMENT = 3;
/** The shortest time between two moves of the accent, in milliseconds. */
const HUE_COOLDOWN_MS = 2000;

/**
 * Writes the frame's dominant hue to --live-h on the root element, at no more than 2 Hz.
 *
 * The accent follows a light that settles. A frame with no dominant hue (a rainbow) leaves it
 * alone, and a light that is still moving (a colour cycle) never agrees with itself long
 * enough to be followed, so the chrome keeps the colour it had instead of chasing the
 * spectrum. A near-grey frame keeps the previous hue too.
 *
 * Returns a stop function.
 */
export function startHueTint(): () => void {
  let last = 0;
  let lastMove = 0;
  let lastHue = readLiveHue();
  let candidate: number | null = null;
  let agree = 0;
  // The first frame of the feed sets the accent at once: the page has only just opened and
  // there is nothing to be steady about yet.
  let first = true;
  const off = onFrame((frame) => {
    const now = performance.now();
    if (now - last < 500) return;
    last = now;
    const found = dominantHue(frame.rgb);
    if (!found) {
      candidate = null;
      agree = 0;
      return;
    }
    const next = Math.round(found.hue);
    if (candidate !== null && hueDistance(next, candidate) < HUE_STEP) agree += 1;
    else {
      candidate = next;
      agree = 1;
    }
    const settled = first || agree >= HUE_AGREEMENT;
    first = false;
    if (!settled) return;
    if (hueDistance(next, lastHue) < HUE_STEP) return;
    if (now - lastMove < HUE_COOLDOWN_MS) return;
    lastMove = now;
    lastHue = next;
    document.documentElement.style.setProperty("--live-h", String(next));
  });
  return off;
}

/** The hue the chrome is on now, from the token the page starts with. */
function readLiveHue(): number {
  if (typeof document === "undefined") return 215;
  const raw = getComputedStyle(document.documentElement).getPropertyValue("--live-h").trim();
  const value = Number.parseFloat(raw);
  return Number.isFinite(value) ? value : 215;
}

