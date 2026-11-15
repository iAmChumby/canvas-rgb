"use client";

import { useCallback, useEffect, useRef } from "react";
import { onFrame } from "@/lib/preview";
import { getSnap } from "@/lib/store";
import { loadThumb } from "@/lib/thumbs";
import { onLightFrame, useLightFeed, type LightColours } from "@/views/scenes/useLightFeed";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";

/**
 * The strip of the running lights' colours.
 *
 * With a scene (the normal case) the strip shows the lights themselves: the real LED colours of
 * the active scene, from the per-light feed. The scene's groups are laid end to end in the
 * scene's order (each group's lights in their order, each light's LEDs in theirs), and the
 * twelve cells are twelve equal slices of that one run, so the strip stands for the whole scene.
 * A light that is not connected has no colours in the feed and adds nothing. Until the first
 * frame arrives the cells rest at the neutral well colour; they never show a guess.
 *
 * Without a scene the strip falls back to what it was before scenes: the full-canvas preview
 * feed when it is arriving, and the effect's own thumbnail when it is not (the socket can be
 * refused). Twelve cells, one for each column of the picture, so the strip reads as the effect
 * rather than as a list of colours.
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

/**
 * The LEDs of a scene as one run of RGB triples: its groups in order, each group's lights in
 * order, each light's LEDs in order. A light with no colours (not connected, or the feed has not
 * named it yet) contributes nothing.
 */
export function sceneRun(scene: Scene | null, lights: LightView[] | null, colours: LightColours): Uint8Array {
  const parts: Uint8Array[] = [];
  let bytes = 0;
  const byId = new Map((lights ?? []).map((l) => [l.light.id, l]));
  for (const group of scene?.groups ?? []) {
    for (const id of group.members) {
      const view = byId.get(id);
      if (view && !view.connected) continue;
      const rgb = colours(id);
      if (!rgb || rgb.length < 3) continue;
      parts.push(rgb);
      bytes += rgb.length - (rgb.length % 3);
    }
  }
  const out = new Uint8Array(bytes);
  let at = 0;
  for (const rgb of parts) {
    const whole = rgb.length - (rgb.length % 3);
    out.set(rgb.subarray(0, whole), at);
    at += whole;
  }
  return out;
}

/** The twelve cells, painted by hand through `paint`; an empty colour puts a cell back to rest. */
function useCells() {
  const cells = useRef<(HTMLSpanElement | null)[]>([]);
  const paint = useCallback((colours: string[]) => {
    for (let i = 0; i < CELLS; i++) {
      const el = cells.current[i];
      if (!el) continue;
      // No inline colour: the cell shows the neutral well colour of the stylesheet.
      el.style.backgroundColor = colours[i] ?? "";
    }
  }, []);
  const setRef = useCallback(
    (i: number) => (el: HTMLSpanElement | null) => {
      cells.current[i] = el;
    },
    [],
  );
  return { paint, setRef };
}

function Cells({ paused, setRef }: { paused: boolean; setRef: (i: number) => (el: HTMLSpanElement | null) => void }) {
  return (
    <span className="rail-strip" data-paused={paused ? "true" : "false"} aria-hidden="true">
      {Array.from({ length: CELLS }, (_, i) => (
        <span key={i} className="rail-strip-cell" ref={setRef(i)} />
      ))}
    </span>
  );
}

/** The strip while a scene is active: the real LED colours, one shared feed. */
function SceneStrip({ paused }: { paused: boolean }) {
  const feed = useLightFeed();
  const { paint, setRef } = useCells();

  useEffect(() => {
    const draw = () => {
      const snap = getSnap();
      const activeId = snap.status?.active_scene?.id ?? snap.scenes?.active ?? null;
      const scene = snap.scenes?.scenes.find((x) => x.id === activeId) ?? null;
      const run = sceneRun(scene, snap.lights, feed);
      const leds = run.length / 3;
      paint(leds === 0 ? [] : sampleColumns(run, leds, 1, CELLS));
    };
    draw();
    return onLightFrame(draw);
  }, [feed, paint]);

  return <Cells paused={paused} setRef={setRef} />;
}

/** The strip with no scene: the canvas preview feed, else the effect's thumbnail. */
function CanvasStrip({ effectId, paused }: { effectId: string | null; paused: boolean }) {
  const { paint, setRef } = useCells();

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
      void loadThumb(effectId, { first: true }).then((thumb) => {
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

  return <Cells paused={paused} setRef={setRef} />;
}

export function RailStrip({
  scene,
  effectId,
  paused,
}: {
  /** True while a scene is active: the strip then reads the lights, never a thumbnail. */
  scene: boolean;
  effectId: string | null;
  paused: boolean;
}) {
  return scene ? <SceneStrip paused={paused} /> : <CanvasStrip effectId={effectId} paused={paused} />;
}
