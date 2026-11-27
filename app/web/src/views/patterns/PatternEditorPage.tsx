"use client";
import { ErrorState } from "@/components/ErrorState";

// The pattern editor's page: the editor on one pattern, with the query saying which.
//
//   /patterns/?id=pattern:Intake      a saved pattern, by its id or its bare name
//   /patterns/?state=patterns         (demo) the first seeded pattern
//   /patterns/?built=strip:300        a device of that kind, all off, for looking at the layout
//   /patterns/?built=grid:5x4&painted=6   the same, with the first LEDs painted
//   /patterns/?built=ring:12&frames=4     the same with that many frames, each its own colours, the
//                                         transitions between them mixed (cut, fades at each pace)
//   /patterns/?note=1                 the daemon's "no device of this kind is connected" sentence
//
// The `built` form is how the editor is looked at on a device the library does not hold yet: it
// is a document in memory, and Save is what puts it in the library.

import { useCallback, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { BLACK_HEX, hsvToHex } from "@/lib/color";
import { api } from "@/lib/endpoints";
import { caught, type Failure } from "@/lib/errors";
import { refreshPatterns } from "@/lib/store";
import { PATTERN_GUARD, PatternEditor } from "@/views/patterns/PatternEditor";
import type { Arrangement } from "@/proto/Arrangement";
import type { Frame } from "@/proto/Frame";
import type { Pattern } from "@/proto/Pattern";
import type { Rgb8 } from "@/proto/Rgb8";
import s from "@/views/patterns/patterns.module.css";

/** The sentence the daemon sends when a pattern's kind has no device connected. */
const NO_DEVICE = "no device of this kind is connected";
/**
 * The colours a `built` preview is painted with, in order: pattern *data*, not chrome, so they
 * come from the colour model rather than from a palette (they are what an LED shows).
 */
const PAINT = [0, 190, 45, 285, 135, 20].map((h) => hsvToHex({ h, s: 0.85, v: 1 }));

export function PatternEditorPage() {
  const router = useRouter();
  const [pattern, setPattern] = useState<Pattern | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<Failure | null>(null);

  // An entry the editor's guard made, or one the editor left, opens no editor: the guard's copy of
  // this page stays ahead of the history after a leave (a browser cannot delete it), and a Forward
  // onto it lands here. It goes to the effects page, which is where the leave went.
  //
  // Read in the effect that opens the page, never while rendering: on a client-side navigation Next
  // changes the history in the commit of the new page, so while the page renders `history.state` is
  // still the entry being left (a push from a dirty effects panel would find the panel's guard and
  // open no editor). By the time an effect runs it is the entry the page opened on, for a push, a
  // Back and a reload alike.
  useEffect(() => {
    const state = window.history.state as Record<string, unknown> | null;
    if (state !== null && typeof state === "object" && PATTERN_GUARD in state) {
      router.replace("/effects/");
      return;
    }
    let cancelled = false;
    void (async () => {
      const query = new URLSearchParams(window.location.search);
      try {
        const loaded = await openPattern(query);
        if (cancelled) return;
        setPattern(loaded);
        if (query.get("note") === "1") setNote(NO_DEVICE);
      } catch (e) {
        if (!cancelled) setError(caught(e));
      }
    })();
    return () => {
      cancelled = true;
    };
    // Once, as the page opens: the router's identity is not a reason to read the entry again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The save's promise goes to the editor, which shows a failure beside its Save key and keeps
  // the strokes: the page never replaces the editor with an error line.
  const save = useCallback(async (document: Pattern): Promise<void> => {
    const saved = await api.putPattern(document.id, document);
    setPattern(saved);
    setError(null);
    // It keeps its own error: a save that landed is a save that landed, whatever the list does.
    await refreshPatterns();
  }, []);

  if (error !== null) return <ErrorState compact error={error} className={s.pageLine} />;
  if (pattern === null) return <p className={s.pageLine}>Opening the pattern…</p>;
  return (
    <PatternEditor
      // A saved pattern and a rebuilt one are two editors, never one document drawn twice.
      key={pattern.id}
      pattern={pattern}
      note={note}
      onSave={save}
      // Replace, not push: the editor's entry becomes the effects page, so one Back goes to where
      // the person came from and never reopens a fresh editor.
      onCancel={() => router.replace("/effects/")}
      ownsEntry
    />
  );
}

/** The pattern the query asks for: a saved one, the first saved one, or a device built in memory. */
async function openPattern(query: URLSearchParams): Promise<Pattern> {
  const built = query.get("built");
  if (built !== null && built.length > 0) {
    return preview(built, Number(query.get("painted") ?? "0"), Number(query.get("frames") ?? "1"));
  }
  const id = query.get("id");
  if (id !== null && id.length > 0) return api.pattern(id);
  const all = await api.patterns();
  const first = all.patterns[0];
  if (!first) throw new Error("there are no patterns yet");
  return first;
}

/** `ring:12`, `strip:300`, `grid:5x4`, `single:1`: a pattern on that device, all off, then painted. */
function preview(built: string, painted: number, frameCount = 1): Pattern {
  const [shape, size] = built.split(":");
  const arrangement: Arrangement =
    shape === "ring" || shape === "grid" || shape === "single" ? shape : "strip";
  const grid = arrangement === "grid" ? gridOf(size ?? "") : undefined;
  const ledCount = Math.max(1, grid ? grid.width * grid.height : Number(size ?? "0") || 12);
  const colours = Array.from({ length: ledCount }, (_, i): Rgb8 =>
    i < painted ? (PAINT[i % PAINT.length] ?? BLACK_HEX) : BLACK_HEX,
  );
  const name = `Preview ${built.replace(/[^-A-Za-z0-9 ._]/g, " ").trim()}`;
  const count = Number.isFinite(frameCount) ? Math.min(16, Math.max(1, Math.floor(frameCount))) : 1;
  // One frame is the plain device. More are for looking at the strip: every LED lit, each frame its own
  // run of colours, holds of 300 to 500 ms and the transitions mixed (a fade, a cut, an eased fade, a
  // fade at each pace in turn) with the last one looping back.
  const frames: Frame[] =
    count === 1
      ? [{ colors: colours, hold: 0 }]
      : Array.from({ length: count }, (_, f): Frame => ({
          colors: Array.from({ length: ledCount }, (_, i): Rgb8 =>
            i < (painted > 0 ? painted : ledCount) ? (PAINT[(i + 2 * f) % PAINT.length] ?? BLACK_HEX) : BLACK_HEX,
          ),
          hold: [500, 300, 500, 400][f % 4] ?? 500,
          ...(f % 4 === 1
            ? {}
            : { transition: { kind: "fade", ms: [400, 0, 600, 300][f % 4] ?? 400, easing: (["linear", "linear", "ease_in_out", "ease_out"] as const)[f % 4] ?? "linear" } }),
        }));
  return {
    id: `pattern:${name}`,
    built_on: { led_count: ledCount, arrangement, ...(grid ? { grid } : {}) },
    frames,
    motion: { type: "none" },
  };
}

/** `5x4` as the grid it names. */
function gridOf(size: string): { width: number; height: number } | undefined {
  const match = /^(\d+)x(\d+)$/.exec(size);
  if (!match) return undefined;
  const width = Number(match[1]);
  const height = Number(match[2]);
  return width > 0 && height > 0 ? { width, height } : undefined;
}