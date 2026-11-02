// The builder's live preview (D95.4.4): the daemon's second engine over `?stack=1`, or, in the
// demo, the draft stack composed from the demo effects. The frame is the same binary the daemon
// sends (u16 LE width, u16 LE height, then RGB), drawn into a 320 × 200 canvas by the builder.
//
// The feed is open only while the builder is on screen. In a plain browser it never opens a
// socket: it composites the layers here, with the compositor's own blend maths (D95.3.3), so the
// builder shows the stack without a daemon.

import { apiConfig, dropApiConfig } from "@/lib/api";
import { isDemo } from "@/lib/host";
import { composeStack } from "@/lib/stackFrame";
import { getSnap } from "@/lib/store";
import type { Frame } from "@/lib/preview";
import type { Layout } from "@/proto/Layout";
import type { Stack } from "@/proto/Stack";
import { routes } from "@/proto/routes";
import { CANVAS_H, CANVAS_W } from "@/views/effects/stackDraft";

type Listener = (frame: Frame) => void;

const listeners = new Set<Listener>();
let active = false;
let socket: WebSocket | null = null;
let reconnect: ReturnType<typeof setTimeout> | null = null;
let demoTimer: ReturnType<typeof setInterval> | null = null;
let last: Frame | null = null;
/** What the demo composites: the draft stack and the layout its device areas name. */
let stack: Stack | null = null;
let layout: Layout | null = null;
let startedAt = 0;

export function onStackFrame(listener: Listener): () => void {
  listeners.add(listener);
  if (last) listener(last);
  return () => {
    listeners.delete(listener);
  };
}

function emit(frame: Frame): void {
  last = frame;
  for (const listener of listeners) listener(frame);
}

/** The stack the demo composites. A real daemon already has it from the preview PUT. */
export function setStackPreviewStack(next: Stack | null, nextLayout: Layout | null): void {
  stack = next;
  layout = nextLayout;
}

export function startStackPreview(): void {
  if (active) return;
  active = true;
  startedAt = performance.now();
  if (isDemo()) {
    demoTimer = setInterval(() => {
      if (!stack) return;
      emit({
        width: CANVAS_W,
        height: CANVAS_H,
        rgb: composeStack(stack, layout, (performance.now() - startedAt) / 1000, CANVAS_W, CANVAS_H),
      });
    }, 1000 / 15);
    return;
  }
  void connect();
}

export function stopStackPreview(): void {
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
  last = null;
}

async function connect(): Promise<void> {
  if (!active || socket) return;
  let url: string;
  try {
    const cfg = await apiConfig();
    const wsBase = cfg.base_url.replace(/^http/, "ws");
    url = `${wsBase}${routes.preview}?token=${encodeURIComponent(cfg.token)}&stack=1`;
  } catch {
    scheduleReconnect();
    return;
  }
  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  ws.onmessage = (event: MessageEvent) => {
    if (!(event.data instanceof ArrayBuffer)) return;
    const frame = parseFrame(event.data);
    if (frame) emit(frame);
  };
  ws.onclose = () => {
    socket = null;
    dropApiConfig();
    if (active) scheduleReconnect();
  };
  ws.onerror = () => {
    // A refused socket is retried; the builder says "preview only" either way.
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

function parseFrame(buffer: ArrayBuffer): Frame | null {
  if (buffer.byteLength < 4) return null;
  const view = new DataView(buffer);
  const width = view.getUint16(0, true);
  const height = view.getUint16(2, true);
  const expected = width * height * 3;
  if (buffer.byteLength - 4 !== expected) return null;
  return { width, height, rgb: new Uint8Array(buffer, 4, expected) };
}

/** The demo layout a device or zone area names: the active layout in the snapshot. */
export function activeDemoLayout(): Layout | null {
  const snap = getSnap();
  return snap.layouts?.find((entry) => entry.id === snap.activeLayout) ?? snap.layouts?.[0] ?? null;
}

