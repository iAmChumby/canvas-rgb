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

/**
 * At most this many thumbnail requests are on the wire at once (#296 review). The daemon renders
 * two at a time (`CONCURRENT_RENDERS`), and a thumbnail may take up to `THUMBNAIL_TIMEOUT_MS` to
 * answer, so sending every tile at once would hold every HTTP/1.1 socket the browser keeps for the
 * daemon (6 per host) and stall the fan poll and Activate behind them. The rest wait here, in
 * order, and their deadline starts only when they are sent. A picture the person is looking at
 * (the selected effect's preview, the rail, the desk) asks with `first` and goes to the front.
 */
export const THUMBS_IN_FLIGHT = 2;
let inFlight = 0;
const waiting: Array<{ id: string; start: () => void }> = [];

/** Runs `send` when one of the THUMBS_IN_FLIGHT places is free; `first` jumps the queue. */
function queued<T>(id: string, send: () => Promise<T>, first: boolean): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const start = () => {
      inFlight += 1;
      send()
        .then(resolve, reject)
        .finally(() => {
          inFlight -= 1;
          waiting.shift()?.start();
        });
    };
    if (inFlight < THUMBS_IN_FLIGHT) start();
    else if (first) waiting.unshift({ id, start });
    else waiting.push({ id, start });
  });
}

/** How many thumbnail requests are on the wire now (for tests). */
export function thumbsInFlight(): number {
  return inFlight;
}

/**
 * Loads one effect's thumbnail once. A failed load resolves to null (the tile shows "no picture")
 * and is not kept, so the next tile that mounts asks again (a daemon hiccup is not cached).
 */
export function loadThumb(id: string, opts: { first?: boolean } = {}): Promise<Thumb | null> {
  let hit = cache.get(id);
  if (hit && opts.first) {
    // Asked for already but still waiting: the picture the person is looking at goes first.
    const at = waiting.findIndex((w) => w.id === id);
    if (at > 0) waiting.unshift(...waiting.splice(at, 1));
  }
  if (!hit) {
    hit = queued(id, () => api.effectThumbnail(id), opts.first === true)
      .then(parseThumbnail)
      .catch(() => {
        cache.delete(id);
        return null;
      });
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
