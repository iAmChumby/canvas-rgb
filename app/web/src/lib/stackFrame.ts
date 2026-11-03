// The demo's stack compositor: one frame of a stack from the demo effects, with the compositor's
// own blend maths (D95.3.3). Used by the builder's preview feed and the stack's library tile; a
// real daemon draws both itself.

import { demoFrameFor, THUMB_FRAMES, THUMB_H, THUMB_W } from "@/lib/thumb-demo";
import type { Layout } from "@/proto/Layout";
import type { PropValue } from "@/proto/PropValue";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";
import { areaBounds, CANVAS_H, CANVAS_W } from "@/views/effects/stackDraft";

function colorOf(value: PropValue | undefined, fallback: string): [number, number, number] {
  const hex = value?.type === "color" ? value.value : fallback;
  const raw = hex.replace("#", "");
  if (raw.length !== 6) return [0, 0, 0];
  return [
    Number.parseInt(raw.slice(0, 2), 16),
    Number.parseInt(raw.slice(2, 4), 16),
    Number.parseInt(raw.slice(4, 6), 16),
  ];
}

function numberProp(layer: StackLayer, key: string, fallback: number): number {
  const value = layer.props[key];
  return value?.type === "number" ? value.value : fallback;
}

/** The degrees an angle prop holds, including the hue a preset saved before the kind existed. */
function angleProp(layer: StackLayer, key: string, fallback: number): number {
  const value = layer.props[key];
  return value?.type === "angle" || value?.type === "hue" ? value.value : fallback;
}

/** One layer's demo picture. A handful of effects have a look built from their props. */
function demoLayerPicture(layer: StackLayer, width: number, height: number, seconds: number): Uint8Array {
  if (layer.effect === "solid") {
    const [r, g, b] = colorOf(layer.props.color, "#6000fc");
    const out = new Uint8Array(width * height * 3);
    for (let i = 0; i < width * height; i++) {
      out[i * 3] = r;
      out[i * 3 + 1] = g;
      out[i * 3 + 2] = b;
    }
    return out;
  }
  if (layer.effect === "gradient") {
    const from = colorOf(layer.props.color1, "#ff2d55");
    const to = colorOf(layer.props.color2, "#ffb703");
    const angle = ((angleProp(layer, "angle", 90) % 360) * Math.PI) / 180;
    const dx = Math.cos(angle);
    const dy = Math.sin(angle);
    const out = new Uint8Array(width * height * 3);
    let o = 0;
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const nx = (x + 0.5) / width - 0.5;
        const ny = (y + 0.5) / height - 0.5;
        const t = Math.max(0, Math.min(1, 0.5 + nx * dx + ny * dy));
        out[o++] = from[0] + (to[0] - from[0]) * t;
        out[o++] = from[1] + (to[1] - from[1]) * t;
        out[o++] = from[2] + (to[2] - from[2]) * t;
      }
    }
    return out;
  }
  if (layer.effect === "fire") {
    const palette = layer.props.palette?.type === "choice" ? layer.props.palette.value : "Classic";
    const cool: [number, number, number] =
      palette === "Blue"
        ? [40, 90, 255]
        : palette === "Green"
          ? [40, 255, 110]
          : palette === "Mono"
            ? [255, 255, 255]
            : [255, 170, 60];
    const intensity = Math.max(0, Math.min(100, numberProp(layer, "intensity", 55))) / 100;
    const out = new Uint8Array(width * height * 3);
    let o = 0;
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const nx = (x + 0.5) / width - 0.5;
        const ny = (y + 0.5) / height;
        const flicker = 0.16 * Math.sin(x * 0.31 + seconds * 3.1) * Math.sin(y * 0.17 - seconds * 1.7);
        const heat = Math.max(0, 1 - Math.hypot(nx * 2.1, 1 - ny) + flicker) * intensity;
        out[o++] = Math.min(255, cool[0] * heat + 12);
        out[o++] = Math.min(255, cool[1] * heat);
        out[o++] = Math.min(255, cool[2] * heat);
      }
    }
    return out;
  }
  if (layer.effect === "rain") {
    // A sparse layer: a dark sky with bright grains, which is what a sparkle layer shows over a
    // base. (The real Rain draws falling drops; the demo reads as sparkles, as the mockups do.)
    const drop = colorOf(layer.props.drop_color, "#a8d8ff");
    const bg = colorOf(layer.props.background, "#050814");
    const out = new Uint8Array(width * height * 3);
    for (let i = 0; i < width * height; i++) {
      out[i * 3] = bg[0];
      out[i * 3 + 1] = bg[1];
      out[i * 3 + 2] = bg[2];
    }
    for (let i = 0; i < 30; i++) {
      const x = Math.floor((((i * 61 + 17) / 67) % 1) * width) % width;
      const y = Math.floor((((i * 37 + seconds * (12 + (i % 5) * 4)) / 43) % 1) * height) % height;
      const twinkle = 0.55 + 0.45 * Math.sin(seconds * 3 + i);
      const at = (y * width + x) * 3;
      out[at] = Math.min(255, drop[0] * twinkle);
      out[at + 1] = Math.min(255, drop[1] * twinkle);
      out[at + 2] = Math.min(255, drop[2] * twinkle);
    }
    return out;
  }
  return demoFrameFor(layer.effect, seconds, width, height);
}

export function blendChannel(mode: StackLayer["blend"], d: number, s: number, a: number): number {
  if (mode === "add") return Math.min(1, d + a * (Math.min(1, d + s) - d));
  if (mode === "multiply") return d * (1 - a + a * s);
  if (mode === "screen") return Math.min(1, d + a * s * (1 - d));
  return s * a + d * (1 - a);
}

/** One frame of the stack: every visible layer mapped into its area, bottom first. */
export function composeStack(
  stack: Stack,
  layout: Layout | null,
  seconds: number,
  width: number,
  height: number,
): Uint8Array {
  const out = new Uint8Array(width * height * 3);
  const acc = new Float32Array(width * height * 3);
  for (const layer of stack.layers) {
    if (!layer.visible || layer.opacity === 0) continue;
    const bounds = areaBounds(layer.area, layout);
    if (!bounds || bounds.width <= 0 || bounds.height <= 0) continue;
    const picture = demoLayerPicture(layer, width, height, seconds);
    const x0 = Math.max(0, Math.floor((bounds.x / CANVAS_W) * width));
    const y0 = Math.max(0, Math.floor((bounds.y / CANVAS_H) * height));
    const x1 = Math.min(width, Math.ceil(((bounds.x + bounds.width) / CANVAS_W) * width));
    const y1 = Math.min(height, Math.ceil(((bounds.y + bounds.height) / CANVAS_H) * height));
    const spanX = Math.max(1, x1 - x0);
    const spanY = Math.max(1, y1 - y0);
    const a = Math.max(0, Math.min(100, layer.opacity)) / 100;
    for (let y = y0; y < y1; y++) {
      const sy = Math.min(height - 1, Math.floor(((y + 0.5 - y0) / spanY) * height));
      for (let x = x0; x < x1; x++) {
        const sx = Math.min(width - 1, Math.floor(((x + 0.5 - x0) / spanX) * width));
        const d = (y * width + x) * 3;
        const s = (sy * width + sx) * 3;
        for (let c = 0; c < 3; c++) {
          const under = acc[d + c] ?? 0;
          acc[d + c] = blendChannel(layer.blend, under, (picture[s + c] ?? 0) / 255, a);
        }
      }
    }
  }
  for (let i = 0; i < acc.length; i++) out[i] = Math.round(Math.max(0, Math.min(1, acc[i] ?? 0)) * 255);
  return out;
}

/** A stack's library tile: the same binary a daemon thumbnail uses, composed in the demo. */
export function stackThumbnailBytes(stack: Stack, layout: Layout | null, loopSeconds = 2): ArrayBuffer {
  const frameBytes = THUMB_W * THUMB_H * 3;
  const buffer = new ArrayBuffer(6 + frameBytes * THUMB_FRAMES);
  const view = new DataView(buffer);
  view.setUint16(0, THUMB_W, true);
  view.setUint16(2, THUMB_H, true);
  view.setUint16(4, THUMB_FRAMES, true);
  const out = new Uint8Array(buffer, 6);
  for (let frame = 0; frame < THUMB_FRAMES; frame++) {
    const rgb = composeStack(stack, layout, (frame / THUMB_FRAMES) * loopSeconds, THUMB_W, THUMB_H);
    out.set(rgb, frame * frameBytes);
  }
  return buffer;
}
