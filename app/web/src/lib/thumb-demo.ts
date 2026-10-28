// Demo-mode thumbnails and preview frames: one generic procedural source (no per-effect look),
// in the same binary format the daemon sends. Only used when there is no Tauri host. They loop: frame t of
// 24 is the phase t/24 of one period.

export const THUMB_W = 64;
export const THUMB_H = 40;
export const THUMB_FRAMES = 24;

type RGB = [number, number, number];
type Shade = (x: number, y: number, t: number) => RGB;

const TAU = Math.PI * 2;
const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);
const fract = (v: number) => v - Math.floor(v);

/** HSV to RGB on the 0..255 scale every shade uses. */
function hsv(h: number, s: number, v: number): RGB {
  const c = hsvUnit(h, s, v);
  return [c[0] * 255, c[1] * 255, c[2] * 255];
}

function hsvUnit(h: number, s: number, v: number): RGB {
  const hh = fract(h) * 6;
  const i = Math.floor(hh);
  const f = hh - i;
  const p = v * (1 - s);
  const q = v * (1 - s * f);
  const r = v * (1 - s * (1 - f));
  switch (i % 6) {
    case 0:
      return [v, r, p];
    case 1:
      return [q, v, p];
    case 2:
      return [p, v, r];
    case 3:
      return [p, q, v];
    case 4:
      return [r, p, v];
    default:
      return [v, p, q];
  }
}

/** A stable number in 0..1 from an effect id, so each demo effect gets its own colours. */
function seedOf(id: string): number {
  let h = 2166136261;
  for (let i = 0; i < id.length; i++) h = Math.imul(h ^ id.charCodeAt(i), 16777619);
  return ((h >>> 0) % 1000) / 1000;
}

/**
 * The demo's one generic source: a drifting two-hue gradient with a soft noise ripple, coloured
 * by the id's seed. It stands in for the daemon's renderer when there is no daemon, and it knows
 * nothing about any effect. It loops: `t` runs 0..1 and every term is periodic in it.
 */
function shadeFor(id: string): Shade {
  const base = seedOf(id);
  return (x, y, t) => {
    const ripple = Math.sin(TAU * (x * 2 + y - t)) * Math.sin(TAU * (y * 3 - t));
    return hsv(base + 0.18 * x + 0.12 * Math.sin(TAU * (y + t)), 0.8, 0.55 + 0.4 * (0.5 + 0.5 * ripple));
  };
}

/** The loop the demo shades run on, in seconds. The same one the thumbnails use. */
export const DEMO_LOOP_SECONDS = 2;

/**
 * One frame of an effect's demo look, at the size the preview feed uses.
 *
 * The phase runs on the thumbnail's loop, so the preview and the card's thumbnail show the
 * same effect at the same moment.
 */
export function demoFrameFor(id: string, seconds: number, w: number, h: number): Uint8Array {
  const shade = shadeFor(id);
  const t = ((seconds / DEMO_LOOP_SECONDS) % 1 + 1) % 1;
  const rgb = new Uint8Array(w * h * 3);
  let o = 0;
  for (let py = 0; py < h; py++) {
    for (let px = 0; px < w; px++) {
      const c = shade((px + 0.5) / w, (py + 0.5) / h, t);
      rgb[o++] = Math.round(clamp01(c[0] / 255) * 255);
      rgb[o++] = Math.round(clamp01(c[1] / 255) * 255);
      rgb[o++] = Math.round(clamp01(c[2] / 255) * 255);
    }
  }
  return rgb;
}

/** The same binary the daemon sends: u16 LE w, u16 LE h, u16 LE frames, then the RGB frames. */
export function thumbnailBytes(id: string): ArrayBuffer {
  const shade = shadeFor(id);
  const frameBytes = THUMB_W * THUMB_H * 3;
  const buf = new ArrayBuffer(6 + frameBytes * THUMB_FRAMES);
  const view = new DataView(buf);
  view.setUint16(0, THUMB_W, true);
  view.setUint16(2, THUMB_H, true);
  view.setUint16(4, THUMB_FRAMES, true);
  const out = new Uint8Array(buf, 6);
  let o = 0;
  for (let f = 0; f < THUMB_FRAMES; f++) {
    const t = f / THUMB_FRAMES;
    for (let py = 0; py < THUMB_H; py++) {
      for (let px = 0; px < THUMB_W; px++) {
        const c = shade((px + 0.5) / THUMB_W, (py + 0.5) / THUMB_H, t);
        out[o++] = Math.round(clamp01(c[0] / 255) * 255);
        out[o++] = Math.round(clamp01(c[1] / 255) * 255);
        out[o++] = Math.round(clamp01(c[2] / 255) * 255);
      }
    }
  }
  return buf;
}

/**
 * The demo's thumbnail for one effect: the real render the daemon's own thumbnail code made
 * (crates/canvas-daemon/examples/render_demo_thumbs.rs), a PNG frame strip in
 * public/demo-thumbs/{id}.png, repacked into the daemon's binary format. An id with no file
 * (a fixture effect) gets the generic source above.
 */
export async function demoThumbnail(id: string): Promise<ArrayBuffer> {
  try {
    const res = await fetch(`/demo-thumbs/${encodeURIComponent(id)}.png`);
    if (!res.ok) return thumbnailBytes(id);
    const bitmap = await createImageBitmap(await res.blob());
    const w = bitmap.width;
    const h = THUMB_H;
    const frames = Math.floor(bitmap.height / h);
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = bitmap.height;
    const ctx = canvas.getContext("2d");
    if (!ctx || frames < 1) return thumbnailBytes(id);
    ctx.drawImage(bitmap, 0, 0);
    const rgba = ctx.getImageData(0, 0, w, h * frames).data;
    const buf = new ArrayBuffer(6 + w * h * frames * 3);
    const view = new DataView(buf);
    view.setUint16(0, w, true);
    view.setUint16(2, h, true);
    view.setUint16(4, frames, true);
    const out = new Uint8Array(buf, 6);
    for (let i = 0, o = 0; i < w * h * frames; i++) {
      out[o++] = rgba[i * 4] ?? 0;
      out[o++] = rgba[i * 4 + 1] ?? 0;
      out[o++] = rgba[i * 4 + 2] ?? 0;
    }
    return buf;
  } catch {
    return thumbnailBytes(id);
  }
}
