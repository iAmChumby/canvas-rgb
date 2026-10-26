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
