// Demo-mode thumbnails: cheap procedural approximations of each effect's look, in the same
// binary format the daemon sends. Only used when there is no Tauri host. They loop: frame t of
// 24 is the phase t/24 of one period.

export const THUMB_W = 64;
export const THUMB_H = 40;
export const THUMB_FRAMES = 24;

type RGB = [number, number, number];
type Shade = (x: number, y: number, t: number) => RGB;

const TAU = Math.PI * 2;
const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);
const fract = (v: number) => v - Math.floor(v);

function hex(h: string): RGB {
  return [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
}

function mix(a: RGB, b: RGB, k: number): RGB {
  const j = clamp01(k);
  return [a[0] + (b[0] - a[0]) * j, a[1] + (b[1] - a[1]) * j, a[2] + (b[2] - a[2]) * j];
}

function scale(c: RGB, k: number): RGB {
  return [c[0] * k, c[1] * k, c[2] * k];
}

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

const cycle = (t: number) => Math.sin(TAU * t);

const SHADES: Record<string, Shade> = {
  solid: () => hex("#6000fc"),
  gradient: (x, _y, t) => mix(hex("#6000fc"), hex("#00d8ff"), 0.5 + 0.5 * Math.sin(TAU * (x - t))),
  rainbow_wave: (x, _y, t) => hsv(x - t, 0.9, 1),
  spiral: (x, y, t) => {
    const a = Math.atan2(y - 0.5, x - 0.5) / TAU;
    const r = Math.hypot(x - 0.5, (y - 0.5) * 0.6);
    return hsv(a * 3 - t + r * 0.6, 1, 0.4 + 0.6 * clamp01(1 - r * 1.2));
  },
  breathing: (_x, _y, t) => scale(hex("#6000fc"), 0.15 + 0.85 * (0.5 + 0.5 * Math.cos(TAU * t))),
  color_cycle: (_x, _y, t) => hsv(t, 1, 1),
  aurora: (x, y, t) => {
    const a = hex("#00d8a0");
    const b = hex("#0066ff");
    const c = hex("#a300ff");
    const band = 0.5 + 0.5 * Math.sin(TAU * (y * 2 + x * 0.7 - t) + Math.sin(TAU * (x * 1.3 + t)));
    const hue = 0.5 + 0.5 * Math.sin(TAU * (x - t * 0.5));
    return scale(mix(mix(a, b, hue), c, band * band), 0.35 + 0.65 * (1 - y * 0.6));
  },
  rain: (x, y, t) => {
    const col = Math.floor(x * THUMB_W);
    const offset = fract(Math.sin(col * 12.9898) * 43758.5453);
    const head = fract(t + offset) * THUMB_H;
    const dy = (head - y * THUMB_H + THUMB_H) % THUMB_H;
    const lit = clamp01(1 - dy / 12);
    return mix(hex("#050814"), hex("#a8d8ff"), lit);
  },
  fire: (x, y, t) => {
    const heat = Math.pow(1 - y, 1.6) * (0.55 + 0.45 * Math.sin(TAU * (x * 3 + t * 2) + Math.sin(TAU * (y * 2 - t * 3))));
    if (heat < 0.5) return mix([0, 0, 0], [200, 30, 0], heat * 2);
    return mix([200, 30, 0], [255, 220, 90], (heat - 0.5) * 2);
  },
  screen_ambience: (x, y, t) => [
    255 * (0.5 + 0.5 * Math.sin(TAU * (x + t))),
    255 * (0.5 + 0.5 * Math.sin(TAU * (y + t * 0.7) + 1)),
    255 * (0.5 + 0.5 * Math.sin(TAU * (x * 0.5 - t) + 2)),
  ],
  audio_spectrum: (x, y, t) => {
    const bar = Math.floor(x * 16);
    const h = 0.25 + 0.75 * Math.abs(Math.sin(TAU * (bar * 0.37 + t * 2)) * Math.sin(TAU * (bar * 0.11 - t)));
    if (1 - y > h) return [12, 12, 16];
    return mix(hex("#ff00a0"), hex("#00fff0"), bar / 15);
  },
  poison: (x, y, t) => {
    const d = Math.hypot((x - 0.5) * 1.6, y - 0.5);
    const v = 0.5 + 0.5 * Math.sin(TAU * (d * 3 - t));
    return mix(mix(hex("#2d0075"), hex("#6000fc"), v), hex("#b300ff"), v * v);
  },
};

function shadeFor(id: string): Shade {
  if (id.startsWith("html:")) {
    return (x, _y, t) => hsv(0.6 + 0.3 * x + 0.1 * cycle(t), 0.5, 0.8);
  }
  return SHADES[id] ?? SHADES.solid!;
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
