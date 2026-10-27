"use client";

import { useEffect, useRef } from "react";
import { useDocumentVisible } from "@/lib/motion";
import { useMotionOff } from "@/components/Motion";
import type { PropValue } from "@/proto/PropValue";

/**
 * The editor's own preview of an effect: it draws the effect from its property
 * values, in the browser, so a colour or a speed change shows up the instant the
 * slider moves, before the daemon has answered. It is an approximation of the
 * daemon's renderer, not the daemon's renderer; the daemon still owns the LEDs.
 */

export type PreviewValues = { [key: string]: PropValue };

const W = 112;
const H = 70;
const TAU = Math.PI * 2;

type RGB = [number, number, number];

function hexToRgb(hex: string): RGB {
  const h = hex.replace("#", "").trim();
  const full = h.length === 3 ? h.split("").map((c) => c + c).join("") : h;
  const n = Number.parseInt(full.slice(0, 6), 16);
  if (!Number.isFinite(n)) return [0, 0, 0];
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function mix(a: RGB, b: RGB, k: number): RGB {
  const t = k < 0 ? 0 : k > 1 ? 1 : k;
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
}

function scale(c: RGB, k: number): RGB {
  return [c[0] * k, c[1] * k, c[2] * k];
}

function hsv(h: number, s: number, v: number): RGB {
  const hh = ((h % 1) + 1) % 1;
  const i = Math.floor(hh * 6);
  const f = hh * 6 - i;
  const p = v * (1 - s);
  const q = v * (1 - s * f);
  const t = v * (1 - s * (1 - f));
  switch (i % 6) {
    case 0:
      return [v, t, p];
    case 1:
      return [q, v, p];
    case 2:
      return [p, v, t];
    case 3:
      return [p, q, v];
    case 4:
      return [t, p, v];
    default:
      return [v, p, q];
  }
}

function num(values: PreviewValues, key: string, fallback: number): number {
  const p = values[key];
  return p && p.type === "number" ? p.value : fallback;
}

function color(values: PreviewValues, key: string, fallback: string): RGB {
  const p = values[key];
  return hexToRgb(p && p.type === "color" ? p.value : fallback);
}

function hue(values: PreviewValues, key: string, fallback: number): number {
  const p = values[key];
  return p && p.type === "hue" ? p.value : fallback;
}

function choice(values: PreviewValues, key: string, fallback: string): string {
  const p = values[key];
  return p && p.type === "choice" ? p.value : fallback;
}

/** Writes one frame of the effect into the pixel buffer. */
function paint(id: string, v: PreviewValues, t: number, out: Uint8ClampedArray): void {
  let i = 0;
  for (let py = 0; py < H; py++) {
    const y = (py + 0.5) / H;
    for (let px = 0; px < W; px++) {
      const x = (px + 0.5) / W;
      const c = shade(id, v, x, y, t);
      out[i++] = c[0];
      out[i++] = c[1];
      out[i++] = c[2];
      out[i++] = 255;
    }
  }
}

function shade(id: string, v: PreviewValues, x: number, y: number, t: number): RGB {
  switch (id) {
    case "solid":
      return color(v, "color", "#6000fc");

    case "gradient": {
      const from = color(v, "color1", "#6000fc");
      const to = color(v, "color2", "#00d8ff");
      const deg = hue(v, "angle", 90);
      const scroll = num(v, "scroll_speed", 0) / 100;
      const rad = (deg * Math.PI) / 180;
      const dx = Math.cos(rad);
      const dy = Math.sin(rad);
      const proj = (x - 0.5) * dx + (y - 0.5) * dy;
      const k = 0.5 + proj * 0.9 + t * scroll;
      return mix(from, to, ((k % 1) + 1) % 1);
    }

    case "rainbow_wave": {
      const speed = num(v, "speed", 30) / 100;
      const width = num(v, "width", 60) / 100;
      const dir = num(v, "direction", 0);
      const cycles = num(v, "cycles", 1);
      const h = (x * cycles + dir / 360 + t * speed) % 1;
      const band = Math.abs(((h * 2) % 1) - 0.5) * 2;
      const bright = 0.35 + 0.65 * Math.max(0, 1 - band / Math.max(0.05, width));
      return scale(hsv(h, 0.9, 1), bright);
    }

    case "spiral": {
      const speed = num(v, "speed", 25) / 100;
      const arms = Math.max(1, Math.round(num(v, "arms", 3)));
      const sat = num(v, "saturation", 100) / 100;
      const shift = num(v, "hue_shift", 0) / 360;
      const a = Math.atan2(y - 0.5, x - 0.5) / TAU;
      const r = Math.hypot(x - 0.5, (y - 0.5) * 0.6);
      const h = a * arms - t * speed + r * 0.6 + shift;
      return hsv(h, sat, 0.4 + 0.6 * Math.max(0, 1 - r * 1.2));
    }

    case "breathing": {
      const c = color(v, "color", "#6000fc");
      const period = Math.max(0.2, num(v, "period", 4));
      const min = num(v, "min_brightness", 5) / 100;
      const k = min + (1 - min) * (0.5 + 0.5 * Math.cos((TAU * t) / period));
      return scale(c, k);
    }

    case "color_cycle": {
      const speed = num(v, "speed", 20) / 100;
      const sat = num(v, "saturation", 100) / 100;
      const val = num(v, "value", 100) / 100;
      return hsv(t * speed, sat, val);
    }

    case "aurora": {
      const c1 = color(v, "color1", "#00d8a0");
      const c2 = color(v, "color2", "#0066ff");
      const c3 = color(v, "color3", "#a300ff");
      const speed = num(v, "speed", 20) / 100;
      const bands = Math.max(1, num(v, "bands", 3));
      const band = 0.5 + 0.5 * Math.sin(TAU * (y * bands + x * 0.7 - t * speed) + Math.sin(TAU * (x * 1.3 + t)));
      const hueMix = 0.5 + 0.5 * Math.sin(TAU * (x - t * speed * 0.5));
      return scale(mix(mix(c1, c2, hueMix), c3, band * band), 0.35 + 0.65 * (1 - y * 0.6));
    }

    case "rain": {
      const bg = color(v, "background", "#050814");
      const drop = color(v, "drop_color", "#a8d8ff");
      const density = Math.max(1, num(v, "density", 120));
      const speed = num(v, "speed", 120);
      const trail = Math.max(2, num(v, "trail_length", 14));
      const col = Math.floor(x * 24);
      const offset = ((Math.sin(col * 12.9898) * 43758.5453) % 1 + 1) % 1;
      const visible = ((col * 7919) % density) / density < 0.55 ? 1 : 0;
      const head = (((t * speed) / 60 + offset) % 1) * 1.2 - 0.1;
      const dy = ((head - y + 1.2) % 1.2) * H;
      const lit = Math.max(0, 1 - dy / trail) * visible;
      return mix(bg, drop, lit);
    }

    case "fire": {
      const intensity = num(v, "intensity", 55) / 100;
      const palette = choice(v, "palette", "Classic");
      const cooling = num(v, "cooling", 30) / 100;
      const heat = Math.pow(1 - y, 1.6) * (0.55 + 0.45 * Math.sin(TAU * (x * 3 + t * 2) + Math.sin(TAU * (y * 2 - t * 3))));
      const k = heat * (0.6 + intensity * 0.8) * (1 - cooling * 0.4);
      if (palette === "Blue") return mix([0, 0, 0], [40, 90, 255], Math.min(1, k * 2));
      if (palette === "Green") return mix([0, 0, 0], [40, 220, 90], Math.min(1, k * 2));
      if (palette === "Mono") return mix([0, 0, 0], [255, 240, 220], Math.min(1, k * 1.6));
      if (k < 0.5) return mix([0, 0, 0], [200, 30, 0], k * 2);
      return mix([200, 30, 0], [255, 220, 90], (k - 0.5) * 2);
    }

    case "poison": {
      const bg = color(v, "bgColor", "#2d0075");
      const c1 = color(v, "color1", "#6000fc");
      const c2 = color(v, "color2", "#b300ff");
      const c3 = color(v, "color3", "#8a42ff");
      const speed = num(v, "speedRaw", 35) / 100;
      const d = Math.hypot((x - 0.5) * 1.6, y - 0.5);
      const wave = 0.5 + 0.5 * Math.sin(TAU * (d * 3 - t * speed));
      const brew = mix(mix(bg, c1, wave), c2, wave * wave);
      return mix(brew, c3, wave * wave * wave * 0.6);
    }

    case "audio_spectrum": {
      const low = color(v, "color_low", "#6000fc");
      const high = color(v, "color_high", "#00ff9c");
      const sens = num(v, "sensitivity", 150) / 150;
      const bar = Math.floor(x * 16);
      const h = (0.25 + 0.75 * Math.abs(Math.sin(TAU * (bar * 0.37 + t * 2)) * Math.sin(TAU * (bar * 0.11 - t)))) * sens;
      if (1 - y > Math.min(1, h)) return [12, 12, 16];
      return mix(low, high, bar / 15);
    }

    case "screen_ambience": {
      const sat = num(v, "saturation", 120) / 100;
      const bright = num(v, "brightness", 100) / 100;
      const base = hsv(0.55 + 0.25 * x + 0.1 * Math.sin(TAU * t), Math.min(1, 0.45 * sat), Math.min(1, 0.7 * bright));
      return base;
    }

    default: {
      // An effect we do not know: a slow hue sweep, so the tile is never dead.
      return hsv(t * 0.15 + x * 0.2, 0.55, 0.8);
    }
  }
}

/**
 * A live preview of one effect. It repaints the moment a property changes, and
 * animates while the window is visible unless motion is off.
 */
export function EffectPreview({
  id,
  values,
  className,
}: {
  id: string;
  values: PreviewValues;
  className?: string;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const valuesRef = useRef(values);
  const phaseRef = useRef(0);
  const motionOff = useMotionOff();
  const visible = useDocumentVisible();

  // Keep the loop's copy of the values current.
  useEffect(() => {
    valuesRef.current = values;
  }, [values]);

  // Repaint on every value change: the feedback is immediate, motion or not.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    if (canvas.width !== W || canvas.height !== H) {
      canvas.width = W;
      canvas.height = H;
    }
    const img = ctx.createImageData(W, H);
    paint(id, values, phaseRef.current, img.data);
    ctx.putImageData(img, 0, 0);
  }, [id, values]);

  // The animation loop.
  useEffect(() => {
    if (motionOff || !visible) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const img = ctx.createImageData(W, H);
    let raf = 0;
    let last = performance.now();
    const step = (now: number) => {
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      phaseRef.current += dt;
      paint(id, valuesRef.current, phaseRef.current, img.data);
      ctx.putImageData(img, 0, 0);
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [id, motionOff, visible]);

  return <canvas ref={canvasRef} className={className ? `preview-canvas ${className}` : "preview-canvas"} aria-hidden="true" />;
}
