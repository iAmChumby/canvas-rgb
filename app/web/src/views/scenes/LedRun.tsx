"use client";

// The live LEDs of one group, drawn on one canvas element: each light's LEDs in their live
// colours from the feed, in member order, a gap between lights. One shape per kind of light, at
// every size: a ring-shaped light (a fan) is a ring of dots, a single LED is a dot, and anything
// else (a strip, a memory stick, a keyboard) is a row of bars. A light that is not connected is
// drawn hollow, a light with no colours yet as dark ones. The canvas redraws at the feed's own
// 15 fps and never re-renders React: the colours are read from the feed's lookup inside the draw
// loop. A canvas that is scrolled out of view, or in a hidden window, does not paint at all.

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
const MAX_UNIT = 14;
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

const isRing = (l: LightView): boolean => l.light.arrangement === "ring" && l.led_count > 1;

/**
 * Places the lights across `width`: a ring takes a round slot of its own, the other lights share
 * what is left in proportion to their LEDs. Order is the group's order. Pure, so it is tested
 * without a canvas. Whatever the group holds and however narrow the canvas, every slot lies inside
 * `[0, width]`: a light drawn off the end is a light the owner cannot see.
 */
export function layoutRun(lights: LightView[], width: number, height: number): RunLayout {
  const w = Math.max(0, width);
  // The margin gives way before the run does: a canvas narrower than two margins has none.
  const pad = Math.min(PAD, w / 2);
  const usable = Math.max(0, w - pad * 2);
  // The gaps keep their width until the lights crowd: then they close up, never taking more than a
  // fifth of the room, so sixteen devices in a phone's run still fit side by side.
  const gap = lights.length > 1 ? Math.min(LIGHT_GAP, (usable * 0.2) / (lights.length - 1)) : 0;
  const avail = Math.max(0, usable - Math.max(0, lights.length - 1) * gap);
  const rings = lights.filter(isRing).length;
  const runLeds = lights.reduce((n, l) => n + (isRing(l) ? 0 : Math.max(1, l.led_count)), 0);
  let ringD = Math.max(0, Math.min(height - 8, 44));
  // Rings keep a round slot of up to 44 px. When strips share the room, the rings take a quarter
  // of it, a little over 20 px each when there is room for that (a ring any smaller is a speck) but
  // never more than two fifths of the room in all: the strips then take what is left, thin as that
  // is.
  if (rings > 0 && runLeds > 0) {
    ringD = Math.min(ringD, Math.max((avail * 0.25) / rings, Math.min(20, (avail * 0.4) / rings)));
  }
  if (rings > 0 && runLeds === 0) ringD = Math.min(ringD, avail / rings);
  const unit = runLeds > 0 ? Math.min(MAX_UNIT, Math.max(0, (avail - rings * ringD) / runLeds)) : 0;
  const slots: Slot[] = [];
  let x = pad;
  for (const light of lights) {
    const ring = isRing(light);
    const slotW = ring ? ringD : Math.max(1, light.led_count) * unit;
    slots.push({ x, w: slotW, light, ring });
    x += slotW + gap;
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

// An LED's ink as a CSS colour string. Building one per LED per frame is the paint's main
// allocation, so each colour's string is made once and kept (bounded: a feed that never repeats
// itself empties the table rather than growing it).
const INK_LIMIT = 8192;
const inks = new Map<number, string>();

function inkFor(r: number, g: number, b: number): string {
  const key = (r << 16) | (g << 8) | b;
  let ink = inks.get(key);
  if (ink === undefined) {
    if (inks.size >= INK_LIMIT) inks.clear();
    ink = `rgb(${r} ${g} ${b})`;
    inks.set(key, ink);
  }
  return ink;
}

/** The ink of LED `i` of a light's colours, or null for a dark one. */
function inkOf(rgb: Uint8Array | null, i: number): string | null {
  if (!rgb || i * 3 + 2 >= rgb.length) return null;
  const r = rgb[i * 3] ?? 0;
  const g = rgb[i * 3 + 1] ?? 0;
  const b = rgb[i * 3 + 2] ?? 0;
  // A black LED is an unlit one: it shows as the dark dot, not as a hole.
  return r + g + b < 18 ? null : inkFor(r, g, b);
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
  if (slot.light.light.arrangement === "single") {
    // One LED: one dot, in the middle of its slot.
    dot(ctx, slot.x + slot.w / 2, cy, Math.max(2, Math.min(slot.w / 2 - 1, height / 2 - 9, 9)), hollow ? null : inkOf(rgb, 0), colours.off, colours.line, hollow);
    return;
  }
  // A strip, a stick or a keyboard: every LED is a bar, as a lit strip looks on the desk.
  const bar = Math.max(1, unit - Math.min(3, unit * 0.25));
  const h = Math.max(10, height - 22);
  const top = cy - h / 2;
  let fill = "";
  for (let i = 0; i < n; i += 1) {
    const x = slot.x + i * unit;
    if (hollow) {
      ctx.strokeStyle = colours.line;
      ctx.strokeRect(x + 0.5, top + 0.5, Math.max(0, bar - 1), h - 1);
    } else {
      const ink = inkOf(rgb, i) ?? colours.off;
      // The same colour as the last LED needs no new style.
      if (ink !== fill) {
        ctx.fillStyle = ink;
        fill = ink;
      }
      ctx.fillRect(x, top, bar, h);
    }
  }
}

/** What a screen reader hears for the run: the lights and how many LEDs. */
export function runLabel(name: string, lights: LightView[]): string {
  const leds = lights.reduce((n, l) => n + l.led_count, 0);
  const away = lights.filter((l) => !l.connected).length;
  return `${name}: ${lights.length} ${lights.length === 1 ? "device" : "devices"}, ${leds} LEDs${away > 0 ? `, ${away} not connected` : ""}`;
}

/**
 * What a run is drawn from: each light's id, size, shape and connection. Two arrays with the same
 * signature draw the same picture, so the poll that hands the page a new, equal array of lights
 * every few seconds does not restart a canvas.
 */
export function runSignature(lights: LightView[]): string {
  let sig = "";
  for (const l of lights) sig += `${l.light.id}|${l.led_count}|${l.light.arrangement}|${l.connected ? 1 : 0};`;
  return sig;
}

export function LedRun({ lights, label = "Live lights" }: { lights: LightView[]; label?: string }) {
  const feed = useContext(LightFeedContext);
  const ref = useRef<HTMLCanvasElement>(null);
  // The newest lights, read by the draw loop, so the loop itself outlives a new array.
  const latest = useRef(lights);
  const signature = runSignature(lights);

  useEffect(() => {
    latest.current = lights;
  });

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
    // Off screen, or in a hidden window: nothing is painted until it is seen again.
    let onScreen = true;
    let layout: RunLayout | null = null;

    const shown = () => onScreen && (typeof document === "undefined" || document.visibilityState !== "hidden");

    const paint = (now: number) => {
      if (width <= 0 || height <= 0) return;
      if (now - colourAt > 1000) {
        colours = readColours(canvas);
        colourAt = now;
      }
      const dpr = canvas.width / width;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);
      // The layout changes only with the size or the lights: it is made again then, not per frame.
      layout ??= layoutRun(latest.current, width, height);
      for (const slot of layout.slots) drawSlot(ctx, slot, layout.unit, height, feed(slot.light.light.id), colours);
    };

    const size = () => {
      const box = canvas.getBoundingClientRect();
      width = box.width;
      height = box.height;
      const dpr = Math.min(3, window.devicePixelRatio || 1);
      canvas.width = Math.max(1, Math.round(width * dpr));
      canvas.height = Math.max(1, Math.round(height * dpr));
      layout = null;
      last = -Infinity;
      paint(performance.now());
    };

    const loop = (now: number) => {
      if (stopped) return;
      raf = requestAnimationFrame(loop);
      if (now - last < FRAME_MS || !shown()) return;
      last = now;
      paint(now);
    };

    size();
    const observer = new ResizeObserver(size);
    observer.observe(canvas);
    let seen: IntersectionObserver | null = null;
    if (typeof IntersectionObserver === "function") {
      seen = new IntersectionObserver((entries) => {
        const entry = entries[entries.length - 1];
        if (entry) onScreen = entry.isIntersecting;
      });
      seen.observe(canvas);
    }
    if (typeof requestAnimationFrame === "function") raf = requestAnimationFrame(loop);
    return () => {
      stopped = true;
      observer.disconnect();
      seen?.disconnect();
      if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(raf);
    };
    // The signature stands for the lights: a new array with the same content restarts nothing.
  }, [signature, feed]);

  return <canvas ref={ref} className={s.ledRun} role="img" aria-label={label} />;
}
