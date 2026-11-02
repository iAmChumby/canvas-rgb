// Effect thumbnails. The daemon answers GET /api/v1/effects/{id}/thumbnail with binary data:
// u16 LE width, u16 LE height, u16 LE frame count, then frames x width x height x 3 RGB bytes
// (64 x 40, 24 frames, 12 fps). Each effect is fetched once and cached for the session.

import { api } from "@/lib/endpoints";

export interface Thumb {
  w: number;
  h: number;
  frames: number;
  /** All frames, frame after frame, RGB. */
  data: Uint8Array;
}

/** Reads the thumbnail header and checks the body is the size it declares. */
export function parseThumbnail(buf: ArrayBuffer): Thumb | null {
  if (buf.byteLength < 6) return null;
  const view = new DataView(buf);
  const w = view.getUint16(0, true);
  const h = view.getUint16(2, true);
  const frames = view.getUint16(4, true);
  const need = w * h * 3 * frames;
  if (w === 0 || h === 0 || frames === 0 || buf.byteLength - 6 < need) return null;
  return { w, h, frames, data: new Uint8Array(buf, 6, need) };
}

const cache = new Map<string, Promise<Thumb | null>>();

/** Loads one effect's thumbnail once. A failed load resolves to null (the tile stays blank). */
export function loadThumb(id: string): Promise<Thumb | null> {
  let hit = cache.get(id);
  if (!hit) {
    hit = api
      .effectThumbnail(id)
      .then(parseThumbnail)
      .catch(() => null);
    cache.set(id, hit);
  }
  return hit;
}

/** A canvas that frames are copied onto, with the buffer they are copied through. */
export type Surface = { ctx: CanvasRenderingContext2D; canvas: HTMLCanvasElement; img: ImageData | null };

/** Copies one RGB frame (3 bytes a pixel, starting at `offset`) onto the canvas. */
export function blit(s: Surface, rgb: Uint8Array, offset: number, w: number, h: number): void {
  if (s.canvas.width !== w || s.canvas.height !== h || !s.img) {
    s.canvas.width = w;
    s.canvas.height = h;
    s.img = s.ctx.createImageData(w, h);
  }
  const out = s.img.data;
  for (let i = 0, o = 0, p = offset; i < w * h; i++, o += 4, p += 3) {
    out[o] = rgb[p] ?? 0;
    out[o + 1] = rgb[p + 1] ?? 0;
    out[o + 2] = rgb[p + 2] ?? 0;
    out[o + 3] = 255;
  }
  s.ctx.putImageData(s.img, 0, 0);
}

/** The loop a preview route carries as base64, decoded. Null when the text is not a loop. */
export function thumbFromBase64(base64: string): Thumb | null {
  if (base64.length === 0) return null;
  try {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return parseThumbnail(bytes.buffer);
  } catch {
    return null;
  }
}
