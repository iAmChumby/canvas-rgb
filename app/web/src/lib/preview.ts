// The live preview feed. In the tray it is the daemon's WebSocket (binary frames: u16 LE width,
// u16 LE height, then width*height*3 RGB bytes). In a plain browser it is a local demo gradient.
// The socket is open only while the window is visible. The dominant hue of the frame is written
// to --live-h at no more than 2 Hz, so the chrome tints with the light.
//
// The feed also keeps a status the desk canvas shows: whether the socket is connecting, live or
// refused, and why. A refused socket is diagnosed against the daemon's own answer, so the canvas
// can say what is wrong instead of waiting for a frame that never comes.

import { useSyncExternalStore } from "react";
import { apiConfig } from "@/lib/api";
import { isDemo } from "@/lib/host";
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

/** A cheap moving gradient, 15 frames a second, for demo mode only. */
function demoFrame(t: number): Frame {
  const rgb = new Uint8Array(PREVIEW_W * PREVIEW_H * 3);
  let o = 0;
  for (let y = 0; y < PREVIEW_H; y++) {
    for (let x = 0; x < PREVIEW_W; x++) {
      const u = x / PREVIEW_W;
      const v = y / PREVIEW_H;
      const wave = Math.sin(u * 6.2 + t * 0.9) * 0.5 + Math.sin(v * 4.1 - t * 0.7) * 0.5;
      const hue = (t * 14 + u * 120 + v * 40) % 360;
      const k = 0.5 + 0.5 * wave;
      const [r, g, b] = hsvToRgb(hue, 0.85, 0.25 + 0.7 * k * (0.6 + 0.4 * v));
      rgb[o++] = r;
      rgb[o++] = g;
      rgb[o++] = b;
    }
  }
  return { width: PREVIEW_W, height: PREVIEW_H, rgb };
}

function hsvToRgb(h: number, s: number, v: number): [number, number, number] {
  const c = v * s;
  const hp = (h % 360) / 60;
  const x = c * (1 - Math.abs((hp % 2) - 1));
  let r = 0;
  let g = 0;
  let b = 0;
  if (hp < 1) [r, g, b] = [c, x, 0];
  else if (hp < 2) [r, g, b] = [x, c, 0];
  else if (hp < 3) [r, g, b] = [0, c, x];
  else if (hp < 4) [r, g, b] = [0, x, c];
  else if (hp < 5) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  const m = v - c;
  return [Math.round((r + m) * 255), Math.round((g + m) * 255), Math.round((b + m) * 255)];
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

// ---- dominant hue at <= 2 Hz ----

function rgbToHue(r: number, g: number, b: number): { hue: number; sat: number } {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  if (d === 0) return { hue: 0, sat: 0 };
  let h: number;
  if (max === r) h = ((g - b) / d) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  h = (h * 60 + 360) % 360;
  return { hue: h, sat: d / max };
}

/**
 * Samples the frame's colour-weighted average hue and writes --live-h on the root element.
 * A near-grey frame keeps the previous hue. Returns a stop function.
 */
export function startHueTint(): () => void {
  let last = 0;
  let lastHue = 215;
  const off = onFrame((frame) => {
    const now = performance.now();
    if (now - last < 500) return;
    last = now;
    let sr = 0;
    let sg = 0;
    let sb = 0;
    let weight = 0;
    const step = 7;
    for (let i = 0; i + 2 < frame.rgb.length; i += 3 * step) {
      const r = frame.rgb[i] ?? 0;
      const g = frame.rgb[i + 1] ?? 0;
      const b = frame.rgb[i + 2] ?? 0;
      const max = Math.max(r, g, b);
      const min = Math.min(r, g, b);
      const w = (max - min) * max + 1;
      sr += r * w;
      sg += g * w;
      sb += b * w;
      weight += w;
    }
    if (weight === 0) return;
    const { hue, sat } = rgbToHue(sr / weight, sg / weight, sb / weight);
    if (sat > 0.08) lastHue = Math.round(hue);
    document.documentElement.style.setProperty("--live-h", String(lastHue));
  });
  return off;
}
