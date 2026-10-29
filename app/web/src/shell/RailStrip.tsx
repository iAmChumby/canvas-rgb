"use client";

import { useCallback, useEffect, useRef } from "react";
import { onFrame } from "@/lib/preview";
import { loadThumb } from "@/lib/thumbs";

/**
 * The strip of the running effect's colours.
 *
 * The colours are the light itself: the live preview feed when it is arriving, and the
 * effect's own thumbnail when it is not (the socket can be refused). Twelve cells, one for
 * each column of the picture, so the strip reads as the effect rather than as a list of
 * colours.
 *
 * The cells are painted by hand, not by React: the feed runs at fifteen frames a second and
 * a re-render for each one would cost more than the picture is worth.
 */

/** How many cells the strip has. */
const CELLS = 12;

/** How many rows apart the samples are taken. Every fourth row is enough for a colour. */
const ROW_STEP = 4;

/**
 * The colour of each column of a picture, as the twelve colours of the strip.
 *
 * The picture is raw RGB, three bytes a pixel, rows one after another.
 *
 * The colour of a column is the colour of its brightest band, not the average of the column:
 * an effect is mostly dark with a few bright bands, and the average of a column would be the
 * dark. The band is the pixels within a tenth of the column's brightest pixel, so one hot
 * pixel cannot speak for the whole column.
 */
function sampleColumns(rgb: Uint8Array, w: number, h: number, cells: number): string[] {
  const out: string[] = [];
  for (let c = 0; c < cells; c++) {
    const x0 = Math.floor((c * w) / cells);
    const x1 = Math.max(x0 + 1, Math.floor(((c + 1) * w) / cells));
    // The brightest the column gets: the band of the effect that runs through it.
    let best = -1;
    for (let y = 0; y < h; y += ROW_STEP) {
      for (let x = x0; x < x1; x++) {
        const o = (y * w + x) * 3;
        const l = 0.2126 * (rgb[o] ?? 0) + 0.7152 * (rgb[o + 1] ?? 0) + 0.0722 * (rgb[o + 2] ?? 0);
        if (l > best) best = l;
      }
    }
    const cut = best * 0.9;
    let r = 0;
    let g = 0;
    let b = 0;
    let n = 0;
    for (let y = 0; y < h; y += ROW_STEP) {
      for (let x = x0; x < x1; x++) {
        const o = (y * w + x) * 3;
        const pr = rgb[o] ?? 0;
        const pg = rgb[o + 1] ?? 0;
        const pb = rgb[o + 2] ?? 0;
        const l = 0.2126 * pr + 0.7152 * pg + 0.0722 * pb;
        if (l < cut) continue;
        r += pr;
        g += pg;
        b += pb;
        n += 1;
      }
    }
    if (n === 0) {
      out.push("transparent");
      continue;
    }
    out.push(`rgb(${Math.round(r / n)}, ${Math.round(g / n)}, ${Math.round(b / n)})`);
  }
  return out;
}

export function RailStrip({ effectId, paused }: { effectId: string | null; paused: boolean }) {
  const cells = useRef<(HTMLSpanElement | null)[]>([]);

  const paint = useCallback((colours: string[]) => {
    for (let i = 0; i < colours.length; i++) {
      const el = cells.current[i];
      const colour = colours[i];
      if (el && colour) el.style.backgroundColor = colour;
    }
  }, []);

  useEffect(() => {
    let stopped = false;
    let live = false;

    // The live feed: the picture the daemon is drawing right now.
    const off = onFrame((frame) => {
      live = true;
      paint(sampleColumns(frame.rgb, frame.width, frame.height, CELLS));
    });

    // The fallback: the effect's own thumbnail, which the daemon renders for every effect.
    // It is only used while no live frame has arrived.
    if (effectId) {
      void loadThumb(effectId).then((thumb) => {
        if (stopped || live || !thumb) return;
        const bytes = thumb.w * thumb.h * 3;
        paint(sampleColumns(thumb.data.subarray(0, bytes), thumb.w, thumb.h, CELLS));
      });
    }

    return () => {
      stopped = true;
      off();
    };
  }, [effectId, paint]);

  return (
    <span className="rail-strip" data-paused={paused ? "true" : "false"} aria-hidden="true">
      {Array.from({ length: CELLS }, (_, i) => (
        <span
          key={i}
          className="rail-strip-cell"
          ref={(el) => {
            cells.current[i] = el;
          }}
        />
      ))}
    </span>
  );
}
