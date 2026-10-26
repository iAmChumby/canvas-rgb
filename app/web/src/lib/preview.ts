// The live preview feed. In the tray it is the daemon's WebSocket (binary frames: u16 LE width,
// u16 LE height, then width*height*3 RGB bytes). In a plain browser it is a local demo gradient.
// The socket is open only while the window is visible. The dominant hue of the frame is written
// to --live-h at no more than 2 Hz, so the chrome tints with the light.

import { apiConfig } from "@/lib/api";
import { isDemo } from "@/lib/host";
import { routes } from "@/proto/routes";

export interface Frame {
  width: number;
  height: number;
  rgb: Uint8Array;
}

type FrameListener = (frame: Frame) => void;

const listeners = new Set<FrameListener>();
let active = false;
let socket: WebSocket | null = null;
let reconnect: ReturnType<typeof setTimeout> | null = null;
let demoTimer: ReturnType<typeof setInterval> | null = null;
let lastFrame: Frame | null = null;

export const PREVIEW_W = 320;
export const PREVIEW_H = 200;

function emit(frame: Frame): void {
  lastFrame = frame;
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
  try {
    const cfg = await apiConfig();
    const base = cfg.base_url.replace(/^http/, "ws");
    url = `${base}${routes.preview}?token=${encodeURIComponent(cfg.token)}`;
  } catch {
    scheduleReconnect();
    return;
  }
  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  ws.onmessage = (ev: MessageEvent) => {
    if (ev.data instanceof ArrayBuffer) {
      const frame = parseFrame(ev.data);
      if (frame) emit(frame);
    }
  };
  ws.onclose = () => {
    socket = null;
    if (active) scheduleReconnect();
  };
  socket = ws;
}

function scheduleReconnect(): void {
  if (reconnect || !active) return;
  reconnect = setTimeout(() => {
    reconnect = null;
    void connect();
  }, 1200);
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
