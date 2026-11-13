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

type Rgb = [number, number, number];

/** A smooth stand-in for noise: a few sines at unrelated rates, 0..1. */
function wobble(x: number, t: number, seed: number): number {
  return (
    0.5 +
    0.25 * Math.sin(x * 9.1 + t * 2.3 + seed) +
    0.15 * Math.sin(x * 21.7 - t * 3.7 + seed * 1.7) +
    0.1 * Math.sin(x * 47.3 + t * 6.1 + seed * 2.9)
  );
}

/** A number from an effect id, so an effect the demo has no picture for still gets its own colour. */
function seedOf(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i += 1) h = (h * 31 + id.charCodeAt(i)) % 360;
  return h;
}

/**
 * What an effect looks like at one LED, in the demo: `p` is the LED's place along its run
 * (0..1, the whole group's run when the lights flow across, each light's own when each plays its
 * own copy), `t` the time in seconds. Each id the demo's scenes use has its own picture, so the
 * page never draws a rainbow for "White".
 */
function paint(effect: string, p: number, t: number, gi: number): Rgb {
  switch (effect) {
    case "white":
      // Static, a touch warm.
      return [255, 246, 232];
    case "fire": {
      const f = wobble(p, t, gi);
      return hsv(6 + 34 * f, 1, 0.45 + 0.55 * f);
    }
    case "aurora": {
      const f = wobble(p * 0.9, t * 0.35, gi);
      return hsv(150 + 120 * f, 0.8, 0.35 + 0.65 * f);
    }
    case "breathing": {
      // The whole run breathes together, in one colour.
      const b = 0.5 + 0.5 * Math.sin((t / 4) * Math.PI * 2);
      return hsv(190 + gi * 23, 0.9, 0.15 + 0.85 * b);
    }
    case "rainbow_wave":
      return hsv(p * 360 + t * 70, 1, 1);
    case "spiral":
      return hsv(p * 720 + t * 110, 0.9, 0.5 + 0.5 * Math.sin(p * 18 - t * 4) ** 2);
    default:
      return hsv(seedOf(effect) + p * 140 + t * 30, 0.85, 0.9);
  }
}

/**
 * The demo's colours for every light: a group that plays an effect shows that effect, a group set
 * to Off, a group whose effect is gone and a light in no group stay black. A group that flows
 * across draws one picture over all its LEDs in order; one set to "each its own" draws the whole
 * picture on every light.
 */
export function demoLightColours(seconds: number): Map<string, Uint8Array> {
  const snap = getSnap();
  const out = new Map<string, Uint8Array>();
  const scenes = snap.scenes;
  const active = scenes?.scenes.find((s) => s.id === scenes.active);
  const counts = new Map((snap.lights ?? []).map((l) => [l.light.id, l.led_count]));
  for (const [gi, group] of (active?.groups ?? []).entries()) {
    const failed = scenes?.group_states.some((s) => s.id === group.id && s.failed);
    const effect = group.plays.type === "effect" && !failed ? group.plays.effect : null;
    const total = group.members.reduce((n, id) => n + (counts.get(id) ?? 0), 0);
    const level = group.brightness / 100;
    let along = 0;
    for (const light of group.members) {
      const n = counts.get(light) ?? 0;
      const rgb = new Uint8Array(n * 3);
      if (effect) {
        for (let i = 0; i < n; i += 1) {
          const p = group.mode === "flow_across" ? (along + i) / Math.max(1, total) : i / Math.max(1, n);
          const [r, g, b] = paint(effect, p, seconds, gi);
          rgb.set([Math.round(r * level), Math.round(g * level), Math.round(b * level)], i * 3);
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
