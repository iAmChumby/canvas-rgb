"use client";

// The pattern editor's page: the editor on one pattern, with the query saying which.
//
//   /patterns/?id=pattern:Intake      a saved pattern, by its id or its bare name
//   /patterns/?state=patterns         (demo) the first seeded pattern
//   /patterns/?built=strip:300        a device of that kind, all off, for looking at the layout
//   /patterns/?built=grid:5x4&painted=6   the same, with the first LEDs painted
//   /patterns/?note=1                 the daemon's "no device of this kind is connected" sentence
//
// The `built` form is how the editor is looked at on a device the library does not hold yet: it
// is a document in memory, and Save is what puts it in the library.

import { useCallback, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { BLACK_HEX, hsvToHex } from "@/lib/color";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { refreshPatterns } from "@/lib/store";
import { PatternEditor } from "@/views/patterns/PatternEditor";
import type { Arrangement } from "@/proto/Arrangement";
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
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const query = new URLSearchParams(window.location.search);
      try {
        const loaded = await openPattern(query);
        if (cancelled) return;
        setPattern(loaded);
        if (query.get("note") === "1") setNote(NO_DEVICE);
      } catch (e) {
        if (!cancelled) setError(messageOf(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const save = useCallback(async (document: Pattern) => {
    try {
      const saved = await api.putPattern(document.id, document);
      setPattern(saved);
      await refreshPatterns();
      setError(null);
    } catch (e) {
      setError(messageOf(e));
    }
  }, []);

  if (error !== null) return <p className={s.pageLine}>{error}</p>;
  if (pattern === null) return <p className={s.pageLine}>Opening the pattern…</p>;
  return (
    <PatternEditor
      // A saved pattern and a rebuilt one are two editors, never one document drawn twice.
      key={pattern.id}
      pattern={pattern}
      note={note}
      onSave={(document) => void save(document)}
      onCancel={() => router.push("/effects/")}
    />
  );
}

/** The pattern the query asks for: a saved one, the first saved one, or a device built in memory. */
async function openPattern(query: URLSearchParams): Promise<Pattern> {
  const built = query.get("built");
  if (built !== null && built.length > 0) {
    return preview(built, Number(query.get("painted") ?? "0"));
  }
  const id = query.get("id");
  if (id !== null && id.length > 0) return api.pattern(id);
  const all = await api.patterns();
  const first = all.patterns[0];
  if (!first) throw new Error("there are no patterns yet");
  return first;
}

/** `ring:12`, `strip:300`, `grid:5x4`, `single:1`: a pattern on that device, all off, then painted. */
function preview(built: string, painted: number): Pattern {
  const [shape, size] = built.split(":");
  const arrangement: Arrangement =
    shape === "ring" || shape === "grid" || shape === "single" ? shape : "strip";
  const grid = arrangement === "grid" ? gridOf(size ?? "") : undefined;
  const ledCount = Math.max(1, grid ? grid.width * grid.height : Number(size ?? "0") || 12);
  const colours = Array.from({ length: ledCount }, (_, i): Rgb8 =>
    i < painted ? (PAINT[i % PAINT.length] ?? BLACK_HEX) : BLACK_HEX,
  );
  const name = `Preview ${built.replace(/[^-A-Za-z0-9 ._]/g, " ").trim()}`;
  return {
    id: `pattern:${name}`,
    built_on: { led_count: ledCount, arrangement, ...(grid ? { grid } : {}) },
    frames: [{ colors: colours, hold: 0 }],
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
