// The hue of a frame of light. Pure maths, no imports, so the editor and the measurement
// harness run the same code.
//
// The question this answers is "what colour is this frame", not "what is the average of these
// bytes". Averaging the RGB channels is not the same thing: half red and half blue averages to
// a purple that is in neither half, and a rainbow averages to a colour that is in none of it.
// So the pixels are grouped into hue bins, the strongest bin wins, and the hue inside it is a
// circular mean, which is what makes 0 and 360 the same place.

/** How wide a hue bin is, in degrees. 24 bins cover the circle. */
export const HUE_BIN = 15;
export const HUE_BINS = 360 / HUE_BIN;

/**
 * How much of the frame's colour the winning hue has to cover before it counts as the frame's
 * colour. A rainbow sweep spreads its colour evenly over every bin, so it never reaches this:
 * a frame with no single colour has no dominant hue.
 */
export const DOMINANT_SHARE = 0.25;

export interface FrameHue {
  /** The dominant hue, in degrees. */
  hue: number;
  /** How much of the frame's colour that hue covers, 0 to 1. */
  share: number;
}

/** The hue of a pixel in degrees, and how colourful and how bright it is. */
export function pixelHue(r: number, g: number, b: number): { hue: number; sat: number; val: number } | null {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  // Grey and near-black say nothing about the hue of a frame.
  if (d < 12 || max < 16) return null;
  let h: number;
  if (max === r) h = ((g - b) / d) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  h = (h * 60 + 360) % 360;
  return { hue: h, sat: d / max, val: max / 255 };
}

/**
 * The frame's dominant hue: the hue that covers the most of the frame.
 *
 * Returns null when the frame has no single colour - a rainbow sweep - so the caller keeps the
 * hue it has instead of chasing the spectrum.
 */
export function dominantHue(rgb: Uint8Array): FrameHue | null {
  const weight = new Float64Array(HUE_BINS);
  const hueSum = new Float64Array(HUE_BINS);
  let total = 0;
  for (let i = 0; i + 2 < rgb.length; i += 3) {
    const p = pixelHue(rgb[i] ?? 0, rgb[i + 1] ?? 0, rgb[i + 2] ?? 0);
    if (!p) continue;
    const w = p.sat * p.val;
    if (w <= 0) continue;
    const bin = Math.floor(p.hue / HUE_BIN) % HUE_BINS;
    weight[bin] = (weight[bin] ?? 0) + w;
    hueSum[bin] = (hueSum[bin] ?? 0) + p.hue * w;
    total += w;
  }
  if (total <= 0) return null;
  let best = 0;
  for (let i = 1; i < HUE_BINS; i++) if ((weight[i] ?? 0) > (weight[best] ?? 0)) best = i;
  const share = (weight[best] ?? 0) / total;
  if (share < DOMINANT_SHARE) return null;
  // The circular mean of the winning bin and the two next to it, so a hue that sits on a bin
  // edge is not pulled towards the edge.
  let x = 0;
  let y = 0;
  let w = 0;
  for (let d = -1; d <= 1; d++) {
    const bin = (best + d + HUE_BINS) % HUE_BINS;
    const bw = weight[bin] ?? 0;
    if (bw <= 0) continue;
    const mean = (hueSum[bin] ?? 0) / bw;
    const rad = (mean * Math.PI) / 180;
    x += Math.cos(rad) * bw;
    y += Math.sin(rad) * bw;
    w += bw;
  }
  if (w <= 0) return null;
  let hue = (Math.atan2(y, x) * 180) / Math.PI;
  if (hue < 0) hue += 360;
  return { hue, share };
}

/**
 * The same answer, by a different road: the circular mean of every pixel's hue, with no bins
 * and no threshold. It is what the measurement harness compares the dominant hue against, so
 * the two do not share a mistake.
 */
export function meanHue(rgb: Uint8Array): FrameHue | null {
  let x = 0;
  let y = 0;
  let w = 0;
  for (let i = 0; i + 2 < rgb.length; i += 3) {
    const p = pixelHue(rgb[i] ?? 0, rgb[i + 1] ?? 0, rgb[i + 2] ?? 0);
    if (!p) continue;
    const rad = (p.hue * Math.PI) / 180;
    x += Math.cos(rad);
    y += Math.sin(rad);
    w += 1;
  }
  if (w === 0) return null;
  let hue = (Math.atan2(y, x) * 180) / Math.PI;
  if (hue < 0) hue += 360;
  // How close the pixels are to one hue: 1 when they all agree, 0 when they are spread out.
  return { hue, share: Math.hypot(x, y) / w };
}

/** The distance between two hues, in degrees, the short way round. */
export function hueDistance(a: number, b: number): number {
  const d = Math.abs((((a - b) % 360) + 360) % 360);
  return d > 180 ? 360 - d : d;
}
