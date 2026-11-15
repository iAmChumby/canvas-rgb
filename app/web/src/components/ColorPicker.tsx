"use client";

import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";
import { hexToHsv, hsvToHex, parseHex, type Hsv } from "@/lib/color";
import { motionOffNow, readCssMs } from "@/lib/motion";
import { cancelPress, refuseDragStart } from "@/lib/pressDrag";

/** A colour the effect already uses, offered as a one-click choice. */
export interface ColorSwatch {
  hex: string;
  label?: string;
}

/**
 * How a colour reached the field. `continuous` is one value of a stream - a pointer drag on the
 * square or the strip, a held arrow key - and `discrete` is one choice made once: a quick swatch
 * clicked, a hex typed and committed. The reveal belongs to the second kind only (#369).
 */
export type ColorChangeMode = "continuous" | "discrete";

/** How far one arrow press moves a handle, in percent. */
const FINE = 1;
const COARSE = 10;

/**
 * The colour field: a saturation/brightness square, a hue strip and the hex value.
 *
 * The square and the strip are drawn, not native controls, and both are reachable with the
 * keyboard. Values stream out while a handle is dragged (once per frame); the field that holds
 * this picker commits the value when the picker closes.
 */
export function ColorPicker({
  label,
  value,
  swatches,
  onChange,
}: {
  label: string;
  value: string;
  swatches?: ColorSwatch[];
  onChange: (hex: string, mode: ColorChangeMode) => void;
}) {
  const [hsv, setHsv] = useState<Hsv>(() => hexToHsv(value));
  const [draft, setDraft] = useState<string | null>(null);
  const svRef = useRef<HTMLDivElement>(null);
  const hueRef = useRef<HTMLDivElement>(null);
  const frameRef = useRef<number | null>(null);
  const queuedRef = useRef<string | null>(null);
  const sentRef = useRef<string | null>(null);
  const liveRef = useRef<Hsv>(hsv);
  // The wipe (issue #187, rule 9): a reveal belongs to a change that lands once (#369). Every
  // emitter below says which kind it is; a stream of values - a drag on the square or the strip,
  // a held arrow key - lands at once, and the preview is not replaced.
  const [prevKey, setPrevKey] = useState(() => hsvToHex(hsv));
  // The preview wipes only after a real change, never on the picker's first render.
  const [wiped, setWiped] = useState(false);
  const handledRef = useRef(prevKey);
  // When the last change happened (and the one before it), and how it was made.
  const changeRef = useRef<{ at: number | null; prevAt: number | null; mode: ColorChangeMode | "external" }>({
    at: null,
    prevAt: null,
    mode: "continuous",
  });
  const note = (mode: ColorChangeMode | "external") => {
    const prev = changeRef.current;
    changeRef.current = { at: performance.now(), prevAt: prev.at, mode };
  };

  // A value that arrives from outside (the field was reset, an import changed it) is taken
  // over; the value this picker sent is left alone.
  useEffect(() => {
    if (sentRef.current !== null && sentRef.current === value) return;
    const next = hexToHsv(value, liveRef.current.h);
    liveRef.current = next;
    // The picker did not make this change: it is a preset applied, a reset, or a value read
    // back. One of them is a choice made once, and shows; a run of them is a stream (#369).
    note("external");
    setHsv(next);
  }, [value]);

  // The picker takes the focus when it opens: the square and the strip are the controls.
  useEffect(() => {
    svRef.current?.focus({ preventScroll: true });
  }, []);

  // The frame that carries the value out. One change per frame, whatever the pointer does.
  const flush = () => {
    frameRef.current = null;
    const hex = queuedRef.current;
    queuedRef.current = null;
    if (hex) {
      sentRef.current = hex;
      onChange(hex, "continuous");
    }
  };

  const stream = (hex: string) => {
    queuedRef.current = hex;
    if (frameRef.current === null) frameRef.current = requestAnimationFrame(flush);
  };

  const now = (next: Hsv) => {
    liveRef.current = next;
    setHsv(next);
    const hex = hsvToHex(next);
    sentRef.current = hex;
    // A pointer drag or a held key: one value in a stream, never a reveal of its own.
    note("continuous");
    stream(hex);
  };

  // When the picker closes, a change that is still waiting for its frame is sent now: the
  // colour the reader last chose is the colour they get.
  useEffect(
    () => () => {
      if (frameRef.current !== null) {
        cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
      const hex = queuedRef.current;
      queuedRef.current = null;
      if (hex) {
        sentRef.current = hex;
        onChange(hex, "continuous");
      }
    },
    // The cleanup must run once, when the picker goes away: a change that depends on
    // onChange would send the colour again on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  /* ---- the saturation and brightness square ---- */

  const svAt = (clientX: number, clientY: number): Hsv | null => {
    const box = svRef.current?.getBoundingClientRect();
    if (!box) return null;
    const s = Math.min(1, Math.max(0, (clientX - box.left) / box.width));
    const v = Math.min(1, Math.max(0, 1 - (clientY - box.top) / box.height));
    return { ...liveRef.current, s, v };
  };

  const onSvDown = (e: PointerEvent<HTMLDivElement>) => {
    cancelPress(e, { focus: true });
    e.currentTarget.setPointerCapture(e.pointerId);
    const next = svAt(e.clientX, e.clientY);
    if (next) now(next);
  };

  const onSvMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    const next = svAt(e.clientX, e.clientY);
    if (next) now(next);
  };

  const onSvUp = (e: PointerEvent<HTMLDivElement>) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
  };

  const onSvKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const by = e.shiftKey ? COARSE : FINE;
    const cur = liveRef.current;
    let next: Hsv | null = null;
    if (e.key === "ArrowLeft") next = { ...cur, s: clamp01(cur.s - by / 100) };
    else if (e.key === "ArrowRight") next = { ...cur, s: clamp01(cur.s + by / 100) };
    else if (e.key === "ArrowUp") next = { ...cur, v: clamp01(cur.v + by / 100) };
    else if (e.key === "ArrowDown") next = { ...cur, v: clamp01(cur.v - by / 100) };
    else if (e.key === "Home") next = { ...cur, s: 0 };
    else if (e.key === "End") next = { ...cur, s: 1 };
    if (!next) return;
    e.preventDefault();
    now(next);
  };

  /* ---- the hue strip ---- */

  const hueAt = (clientX: number): number | null => {
    const box = hueRef.current?.getBoundingClientRect();
    if (!box || box.width <= 0) return null;
    return Math.min(359.999, Math.max(0, ((clientX - box.left) / box.width) * 360));
  };

  const onHueDown = (e: PointerEvent<HTMLDivElement>) => {
    cancelPress(e, { focus: true });
    e.currentTarget.setPointerCapture(e.pointerId);
    const h = hueAt(e.clientX);
    if (h !== null) now({ ...liveRef.current, h });
  };

  const onHueMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    const h = hueAt(e.clientX);
    if (h !== null) now({ ...liveRef.current, h });
  };

  const onHueUp = (e: PointerEvent<HTMLDivElement>) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
  };

  const onHueKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const by = e.shiftKey ? COARSE : FINE;
    const cur = liveRef.current;
    let h: number | null = null;
    if (e.key === "ArrowLeft" || e.key === "ArrowDown") h = (cur.h - by + 360) % 360;
    else if (e.key === "ArrowRight" || e.key === "ArrowUp") h = (cur.h + by) % 360;
    else if (e.key === "Home") h = 0;
    else if (e.key === "End") h = 359;
    if (h === null) return;
    e.preventDefault();
    now({ ...cur, h });
  };

  /* ---- the hex value ---- */

  const hex = hsvToHex(hsv);
  const typed = draft ?? hex;
  // A short text is still being typed, so it is not called wrong yet.
  const valid = draft === null || draft.trim().length < 4 || parseHex(draft) !== null;

  // The reveal, decided per change (#369): it plays for a change that lands once - a quick
  // swatch, a typed hex - and never for a direction of travel - a drag on the square or the
  // strip, a held arrow key, or two changes in the same breath. A stream shows the new colour at
  // once, and the preview keeps its node: no key, so no replay. Reduced motion never wipes.
  useEffect(() => {
    if (handledRef.current === hex) return;
    handledRef.current = hex;
    const c = changeRef.current;
    const gap = readCssMs("--dur", 180);
    const streaming =
      motionOffNow() ||
      c.mode === "continuous" ||
      (c.mode === "external" && c.prevAt !== null && c.at !== null && c.at - c.prevAt < gap);
    if (streaming) {
      // The colour shows at once: the class comes off and the node is kept, so nothing replays.
      setWiped(false);
      return;
    }
    setPrevKey(hex);
    setWiped(true);
    // `wiped` is read to clear a stale reveal; it is set here, and the guard above stops a loop.
  }, [hex, wiped]);

  /**
   * Takes the typed text and makes it the colour. The text is read from the field itself, so
   * a key that arrives before the page has caught up still commits what was typed.
   */
  const commitDraft = (text: string | null) => {
    if (text === null) return;
    const parsed = parseHex(text);
    setDraft(null);
    if (!parsed) return;
    const next = hexToHsv(parsed, liveRef.current.h);
    liveRef.current = next;
    setHsv(next);
    sentRef.current = parsed;
    // A code typed and committed lands once, so it reveals.
    note("discrete");
    onChange(parsed, "discrete");
  };

  const svStyle = {
    "--h": String(Math.round(hsv.h)),
    "--s": String(hsv.s),
    "--v": String(hsv.v),
  } as CSSProperties;
  const hueStyle = { "--h": String(Math.round(hsv.h)) } as CSSProperties;

  return (
    <div className="picker">
      <div
        ref={svRef}
        className="picker-sv"
        style={svStyle}
        role="slider"
        tabIndex={0}
        aria-label={`${label} saturation and brightness`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(hsv.v * 100)}
        aria-valuetext={`saturation ${Math.round(hsv.s * 100)}%, brightness ${Math.round(hsv.v * 100)}%`}
        draggable={false}
        onDragStart={refuseDragStart}
        onPointerDown={onSvDown}
        onPointerMove={onSvMove}
        onPointerUp={onSvUp}
        onKeyDown={onSvKey}
      >
        <div className="picker-sv-handle" />
      </div>

      <div
        ref={hueRef}
        className="picker-hue"
        style={hueStyle}
        role="slider"
        tabIndex={0}
        aria-label={`${label} hue`}
        aria-valuemin={0}
        aria-valuemax={360}
        aria-valuenow={Math.round(hsv.h)}
        draggable={false}
        onDragStart={refuseDragStart}
        onPointerDown={onHueDown}
        onPointerMove={onHueMove}
        onPointerUp={onHueUp}
        onKeyDown={onHueKey}
      >
        <div className="picker-hue-handle" />
      </div>

      <div className="picker-row">
        <input
          type="text"
          className={valid ? "num picker-hex" : "num picker-hex num-invalid"}
          aria-label={`${label} hex value`}
          aria-invalid={!valid}
          spellCheck={false}
          autoComplete="off"
          value={typed}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={(e) => commitDraft(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              // Text that is not a colour yet stays in the field, marked as wrong.
              if (parseHex(e.currentTarget.value) === null) return;
              commitDraft(e.currentTarget.value);
              e.currentTarget.blur();
            }
          }}
        />
        <span key={prevKey} className={wiped ? "picker-preview wipe" : "picker-preview"} style={{ "--c": hex } as CSSProperties} aria-hidden="true" />
      </div>

      {swatches && swatches.length > 0 && (
        <div className="picker-swatches" role="group" aria-label={`${label} colours`}>
          {swatches.map((sw, at) => {
            const parsed = parseHex(sw.hex);
            const swHex = parsed ?? hex;
            // The swatch that names the colour in the field is marked, not dropped (#369): the
            // list stays the same length, so the picker keeps one size.
            const current = swHex.toLowerCase() === hex.toLowerCase();
            return (
              <button
                key={`${swHex}-${at}`}
                type="button"
                className="picker-swatch"
                style={{ "--c": swHex } as CSSProperties}
                aria-label={sw.label ? `${sw.label} ${swHex}` : swHex}
                aria-current={current ? "true" : undefined}
                title={sw.label ?? swHex}
                onClick={() => {
                  const next = hexToHsv(swHex, liveRef.current.h);
                  liveRef.current = next;
                  setHsv(next);
                  sentRef.current = swHex;
                  // A swatch clicked is one choice, made once: it reveals.
                  note("discrete");
                  onChange(swHex, "discrete");
                }}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}

function clamp01(n: number): number {
  return Math.min(1, Math.max(0, n));
}
