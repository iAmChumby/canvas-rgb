"use client";

// The live LEDs of one group, drawn on one canvas element: each light's LEDs in their live
// colours from the feed, in member order, a gap between lights. A ring-shaped light (a fan) is a
// ring of dots, anything else a run. A light that is not connected is drawn as hollow dots, a
// light with no colours yet as dark ones. The canvas redraws at the feed's own 15 fps and never
// re-renders React: the colours are read from the feed's lookup inside the draw loop.

import { createContext, useContext, useEffect, useRef } from "react";
import type { LightColours } from "@/views/scenes/useLightFeed";
import type { LightView } from "@/proto/LightView";
import s from "@/views/scenes/scenes.module.css";

/** Where the colours come from. The page opens the feed once and hands it to every run. */
export const LightFeedContext = createContext<LightColours>(() => null);

/** The space between two lights, in CSS pixels. */
export const LIGHT_GAP = 14;
const PAD = 6;
/** The widest one LED of a run is spaced. */
const MAX_UNIT = 22;
/** The draw period: the feed arrives at 15 fps, so a faster loop only repeats a picture. */
const FRAME_MS = 60;

export interface Slot {
  /** Left edge in CSS pixels. */
  x: number;
  /** Width in CSS pixels. */
  w: number;
  light: LightView;
  ring: boolean;
}

export interface RunLayout {
  slots: Slot[];
  /** The spacing of one LED in a run. */
  unit: number;
}

/**
 * Places the lights across `width`: a ring takes a round slot of its own, the other lights share
 * what is left in proportion to their LEDs. Order is the group's order. Pure, so it is tested
 * without a canvas.
 */
export function layoutRun(lights: LightView[], width: number, height: number): RunLayout {
  const gaps = Math.max(0, lights.length - 1) * LIGHT_GAP;
  const avail = Math.max(0, width - PAD * 2 - gaps);
  const isRing = (l: LightView) => l.light.arrangement === "ring" && l.led_count > 1;
  const rings = lights.filter(isRing).length;
  const runLeds = lights.reduce((n, l) => n + (isRing(l) ? 0 : Math.max(1, l.led_count)), 0);
  let ringD = Math.min(height - 8, 44);
  if (rings > 0 && runLeds > 0) ringD = Math.min(ringD, Math.max(16, (avail - runLeds) / rings));
  if (rings > 0 && runLeds === 0) ringD = Math.min(ringD, Math.max(16, avail / rings));
  const unit = runLeds > 0 ? Math.min(MAX_UNIT, Math.max(0.5, (avail - rings * ringD) / runLeds)) : 0;
  const slots: Slot[] = [];
  let x = PAD;
  for (const light of lights) {
    const ring = isRing(light);
    const w = ring ? ringD : Math.max(1, light.led_count) * unit;
    slots.push({ x, w, light, ring });
    x += w + LIGHT_GAP;
  }
  return { slots, unit };
}

interface Colours {
  off: string;
  line: string;
}

function readColours(el: HTMLElement): Colours {
  const style = getComputedStyle(el);
  return {
    off: style.getPropertyValue("--led-off").trim() || "rgba(128,128,128,0.2)",
    line: style.getPropertyValue("--line-2").trim() || "rgba(128,128,128,0.4)",
  };
}

/** The ink of LED `i` of a light's colours, or null for a dark one. */
function inkOf(rgb: Uint8Array | null, i: number): string | null {
  if (!rgb || i * 3 + 2 >= rgb.length) return null;
  const r = rgb[i * 3] ?? 0;
  const g = rgb[i * 3 + 1] ?? 0;
  const b = rgb[i * 3 + 2] ?? 0;
  // A black LED is an unlit one: it shows as the dark dot, not as a hole.
  return r + g + b < 18 ? null : `rgb(${r} ${g} ${b})`;
}

function dot(ctx: CanvasRenderingContext2D, x: number, y: number, r: number, ink: string | null, off: string, line: string, hollow: boolean) {
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  if (hollow) {
    ctx.strokeStyle = line;
    ctx.lineWidth = 1;
    ctx.stroke();
  } else {
    ctx.fillStyle = ink ?? off;
    ctx.fill();
  }
}

function drawSlot(
  ctx: CanvasRenderingContext2D,
  slot: Slot,
  unit: number,
  height: number,
  rgb: Uint8Array | null,
  colours: Colours,
) {
  const hollow = !slot.light.connected;
  const n = Math.max(1, slot.light.led_count);
  const cy = height / 2;
  if (slot.ring) {
    const cx = slot.x + slot.w / 2;
    const rr = slot.w / 2 - 4;
    const r = Math.max(1.6, Math.min(5.5, (Math.PI * rr * 0.62) / n));
    const path = Math.max(1, rr - r + 1.5);
    for (let i = 0; i < n; i += 1) {
      const a = -Math.PI / 2 + (i / n) * Math.PI * 2;
      dot(ctx, cx + Math.cos(a) * path, cy + Math.sin(a) * path, r, hollow ? null : inkOf(rgb, i), colours.off, colours.line, hollow);
    }
    return;
  }
  if (unit >= 7) {
    const r = Math.min(unit / 2 - 1.5, height / 2 - 9, 9);
    for (let i = 0; i < n; i += 1) {
      dot(ctx, slot.x + (i + 0.5) * unit, cy, r, hollow ? null : inkOf(rgb, i), colours.off, colours.line, hollow);
    }
    return;
  }
  // Too many LEDs for dots: each is a bar, as a lit strip looks on the desk.
  const bar = Math.max(1, unit - (unit > 3 ? 1 : 0));
  const h = Math.max(10, height - 22);
  for (let i = 0; i < n; i += 1) {
    const x = slot.x + i * unit;
    if (hollow) {
      ctx.strokeStyle = colours.line;
      ctx.strokeRect(x + 0.5, cy - h / 2 + 0.5, Math.max(0, bar - 1), h - 1);
    } else {
      ctx.fillStyle = inkOf(rgb, i) ?? colours.off;
      ctx.fillRect(x, cy - h / 2, bar, h);
    }
  }
}

/** What a screen reader hears for the run: the lights and how many LEDs. */
export function runLabel(name: string, lights: LightView[]): string {
  const leds = lights.reduce((n, l) => n + l.led_count, 0);
  const away = lights.filter((l) => !l.connected).length;
  return `${name}: ${lights.length} ${lights.length === 1 ? "device" : "devices"}, ${leds} LEDs${away > 0 ? `, ${away} not connected` : ""}`;
}

export function LedRun({ lights, label = "Live lights" }: { lights: LightView[]; label?: string }) {
  const feed = useContext(LightFeedContext);
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    let width = 0;
    let height = 0;
    let colours = readColours(canvas);
    let colourAt = 0;
    let last = -Infinity;
    let raf = 0;
    let stopped = false;

    const paint = (now: number) => {
      if (width <= 0 || height <= 0) return;
      if (now - colourAt > 1000) {
        colours = readColours(canvas);
        colourAt = now;
      }
      const dpr = canvas.width / width;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);
      const { slots, unit } = layoutRun(lights, width, height);
      for (const slot of slots) drawSlot(ctx, slot, unit, height, feed(slot.light.light.id), colours);
    };

    const size = () => {
      const box = canvas.getBoundingClientRect();
      width = box.width;
      height = box.height;
      const dpr = Math.min(3, window.devicePixelRatio || 1);
      canvas.width = Math.max(1, Math.round(width * dpr));
      canvas.height = Math.max(1, Math.round(height * dpr));
      last = -Infinity;
      paint(performance.now());
    };

    const loop = (now: number) => {
      if (stopped) return;
      raf = requestAnimationFrame(loop);
      if (now - last < FRAME_MS) return;
      last = now;
      paint(now);
    };

    size();
    const observer = new ResizeObserver(size);
    observer.observe(canvas);
    if (typeof requestAnimationFrame === "function") raf = requestAnimationFrame(loop);
    return () => {
      stopped = true;
      observer.disconnect();
      if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(raf);
    };
  }, [lights, feed]);

  return <canvas ref={ref} className={s.ledRun} role="img" aria-label={label} />;
}
