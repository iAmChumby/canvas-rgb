"use client";

// The per-light feed: `GET /api/v1/preview/lights`, a WebSocket (canvas-proto
// `routes::PREVIEW_LIGHTS`). Its first message is JSON mapping a u16 index to a light id (sent
// again when the scene or the lights change); then binary frames: per light a u16 LE index, a
// u16 LE LED count, then that many RGB triples. The hook hands back one lookup, `(lightId) =>
// RGB bytes | null`, that always answers from the newest frame, so a canvas can read it on every
// animation frame without a render. In a plain browser the demo draws its own colours from the
// demo scene.

import { useEffect, useState } from "react";
import { apiConfig, dropApiConfig } from "@/lib/api";
import { routes } from "@/proto/routes";
import { isDemo } from "@/lib/host";
import { getSnap } from "@/lib/store";

/** The colours of one light's LEDs as RGB triples, or null while the feed has none for it. */
export type LightColours = (lightId: string) => Uint8Array | null;

/** The wait before the socket is tried again after it closed: 1.2 s, doubling, up to 15 s. */
function retryWait(failures: number): number {
  return Math.min(15000, 1200 * Math.pow(2, Math.min(4, failures)));
}

/** Reads one binary frame into `into`: each light the frame names gets its own copy of the run. */
export function decodeLightFrame(
  buf: ArrayBuffer,
  names: ReadonlyMap<number, string>,
  into: Map<string, Uint8Array>,
): void {
  const view = new DataView(buf);
  let at = 0;
  while (at + 4 <= buf.byteLength) {
    const index = view.getUint16(at, true);
    const leds = view.getUint16(at + 2, true);
    const end = at + 4 + leds * 3;
    // A frame cut short is dropped from here on: a light's run is whole or absent.
    if (end > buf.byteLength) return;
    const id = names.get(index);
    if (id !== undefined) into.set(id, new Uint8Array(buf.slice(at + 4, end)));
    at = end;
  }
}

/** Reads the index message: `{"0": "<light id>", ...}`. Anything else is no index. */
export function decodeLightIndex(text: string): Map<number, string> | null {
  try {
    const parsed: unknown = JSON.parse(text);
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    const names = new Map<number, string>();
    for (const [key, value] of Object.entries(parsed)) {
      const index = Number(key);
      if (Number.isInteger(index) && typeof value === "string") names.set(index, value);
    }
    return names;
  } catch {
    return null;
  }
}

/** HSV (h in degrees, s and v 0..1) to 0..255 RGB. */
function hsv(h: number, s: number, v: number): [number, number, number] {
  const k = (n: number) => (n + h / 60) % 6;
  const f = (n: number) => v * (1 - s * Math.max(0, Math.min(k(n), 4 - k(n), 1)));
  return [Math.round(f(5) * 255), Math.round(f(3) * 255), Math.round(f(1) * 255)];
}

/**
 * The demo's colours for every light: a group that plays something runs a hue along its lights,
 * a group set to Off and a light in no group stay black.
 */
export function demoLightColours(seconds: number): Map<string, Uint8Array> {
  const snap = getSnap();
  const out = new Map<string, Uint8Array>();
  const scenes = snap.scenes;
  const active = scenes?.scenes.find((s) => s.id === scenes.active);
  const counts = new Map((snap.lights ?? []).map((l) => [l.light.id, l.led_count]));
  for (const [gi, group] of (active?.groups ?? []).entries()) {
    const failed = scenes?.group_states.some((s) => s.id === group.id && s.failed);
    const lit = group.plays.type === "effect" && !failed;
    let along = 0;
    for (const light of group.members) {
      const n = counts.get(light) ?? 0;
      const rgb = new Uint8Array(n * 3);
      if (lit) {
        for (let i = 0; i < n; i += 1) {
          const hue = (gi * 67 + (along + i) * 9 + seconds * 40) % 360;
          const [r, g, b] = hsv(hue, 0.85, (group.brightness / 100) * 0.95);
          rgb.set([r, g, b], i * 3);
        }
      }
      out.set(light, rgb);
      along += n;
    }
  }
  return out;
}

/**
 * Opens the per-light feed while the component is mounted and answers with the lookup. The
 * lookup keeps the same identity for the life of the component. The socket stays closed while
 * the window is hidden.
 */
export function useLightFeed(): LightColours {
  // The map and the lookup over it are made once; the state holds them so the lookup keeps its
  // identity and nothing reads a ref during render.
  const [feed] = useState(() => {
    const store = new Map<string, Uint8Array>();
    const lookup: LightColours = (id) => store.get(id) ?? null;
    return { store, lookup };
  });

  useEffect(() => {
    const store = feed.store;
    let stopped = false;
    let socket: WebSocket | null = null;
    let retry: ReturnType<typeof setTimeout> | null = null;
    let timer: ReturnType<typeof setInterval> | null = null;
    let failures = 0;
    let names: ReadonlyMap<number, string> = new Map();

    const visible = () => typeof document === "undefined" || document.visibilityState === "visible";

    /**
     * The socket is gone: the colours on the canvas are stale, and the index belongs to a
     * daemon that may come back with the lights in another order. Both go, so a frame that
     * arrives before the new index never reaches a light of the old order.
     */
    const forget = () => {
      store.clear();
      names = new Map();
    };

    const close = () => {
      if (retry) {
        clearTimeout(retry);
        retry = null;
      }
      if (timer) {
        clearInterval(timer);
        timer = null;
      }
      if (socket) {
        socket.onclose = null;
        socket.onerror = null;
        socket.onmessage = null;
        socket.close();
        socket = null;
      }
      forget();
    };

    const schedule = () => {
      if (stopped || retry || !visible()) return;
      retry = setTimeout(() => {
        retry = null;
        void connect();
      }, retryWait(failures));
      failures += 1;
    };

    const connect = async (): Promise<void> => {
      if (stopped || socket || !visible()) return;
      let url: string;
      try {
        const cfg = await apiConfig();
        url = `${cfg.base_url.replace(/^http/, "ws")}${routes.previewLights}?token=${encodeURIComponent(cfg.token)}`;
      } catch {
        schedule();
        return;
      }
      // The wait for the token may have outlived the component or the window.
      if (stopped || socket || !visible()) return;
      const ws = new WebSocket(url);
      ws.binaryType = "arraybuffer";
      ws.onopen = () => {
        failures = 0;
      };
      ws.onmessage = (event: MessageEvent) => {
        if (typeof event.data === "string") {
          const next = decodeLightIndex(event.data);
          if (!next) return;
          names = next;
          // A light the daemon no longer names has no colours to show.
          const known = new Set(next.values());
          for (const id of store.keys()) if (!known.has(id)) store.delete(id);
        } else if (event.data instanceof ArrayBuffer) {
          decodeLightFrame(event.data, names, store);
        }
      };
      ws.onclose = () => {
        socket = null;
        // The socket is gone: the colours and the index go with it.
        forget();
        // The tray may have handed out a new token: ask again.
        dropApiConfig();
        schedule();
      };
      socket = ws;
    };

    const start = () => {
      if (stopped || socket || timer) return;
      if (isDemo()) {
        const t0 = performance.now();
        const tick = () => {
          const next = demoLightColours((performance.now() - t0) / 1000);
          store.clear();
          for (const [id, rgb] of next) store.set(id, rgb);
        };
        tick();
        timer = setInterval(tick, 1000 / 15);
        return;
      }
      void connect();
    };

    const onVisibility = () => {
      if (visible()) start();
      else close();
    };

    start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stopped = true;
      document.removeEventListener("visibilitychange", onVisibility);
      close();
      store.clear();
    };
  }, [feed]);

  return feed.lookup;
}
