"use client";

// The frame strip (#468): the pattern's frames in play order, with the way each passes to the next.
//
//   [ ◔1 ]─fade 400─[ ◔2 ]─cut─[ ◔3 ]─fade 600─╮ loop        + ⧉ 🗑 ▶   Hold | fps
//    500ms            300ms        500ms
//
// A frame is a key with a thumbnail of its LEDs (a mini ring, line or grid) and its hold under it in
// mono. The connector between two frames is the transition: click it to choose Cut or Fade, how long
// the fade takes and its pace. The last connector curls back to frame 1 and is the Loop key.
//
// Timing is the holds: edit the selected frame's, or give an fps, which sets every hold to
// `round(1000 / fps)` and every transition to a cut (no fps is stored). The strip keeps its height
// and never reflows under the pointer: the keys are fixed boxes and the strip scrolls sideways.
//
// Conventions come from the scene strip (`views/scenes/SceneStrip.tsx`): press and travel to
// reorder with a marker in the gap, ArrowLeft/ArrowRight on a key move it one place and say where
// it went, and the Popover (`components/Popover.tsx`) carries the transition's fields.

import { Fragment, useCallback, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ArrowBendUpLeft, Copy, FilmStrip, Hourglass, Play, Plus, Stop, Trash } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";
import { NumberField } from "@/components/NumberField";
import { Popover } from "@/components/Popover";
import { useMediaQuery } from "@/components/MediaQuery";
import { Segmented } from "@/components/Segmented";
import { Toggle } from "@/components/Toggle";
import { BLACK_HEX, parseHex } from "@/lib/color";
import { canAdd, FPS_MAX, FPS_MIN, NEW_FADE_MS } from "@/lib/patternFrames";
import { HOLD_MS_MAX, TRANSITION_MS_MAX } from "@/lib/patternPlay";
import { layoutDots } from "@/views/patterns/dotLayout";
import { useFrameReorder } from "@/views/patterns/useFrameReorder";
import type { BuiltOn } from "@/proto/BuiltOn";
import type { Easing } from "@/proto/Easing";
import type { Frame } from "@/proto/Frame";
import type { Transition } from "@/proto/Transition";
import s from "@/views/patterns/patterns.module.css";

/** The thumbnail's box, in css pixels. */
const THUMB_W = 56;
const THUMB_H = 40;
/** A thumbnail draws at most this many dots: a strip of 4096 LEDs is a sample of itself at this size. */
const THUMB_DOTS = 96;

const EASINGS: { value: Easing; label: string }[] = [
  { value: "linear", label: "Linear" },
  { value: "ease_in", label: "In" },
  { value: "ease_out", label: "Out" },
  { value: "ease_in_out", label: "In-out" },
];

export interface FrameStripProps {
  builtOn: BuiltOn;
  frames: Frame[];
  selected: number;
  loop: boolean;
  /** The fps the holds come to, or null when they differ. */
  fps: number | null;
  /** The preview is playing the document. */
  playing: boolean;
  /** Why the preview cannot play, or null when it can. */
  cannotPlay: string | null;
  onSelect(index: number): void;
  onAdd(): void;
  onDuplicate(): void;
  onDelete(): void;
  onMove(from: number, to: number): void;
  onHold(index: number, ms: number): void;
  onTransition(index: number, patch: Partial<Transition>): void;
  onLoop(loop: boolean): void;
  onFps(fps: number): void;
  onPlay(): void;
}

export function FrameStrip(props: FrameStripProps) {
  const { builtOn, frames, selected, loop, fps, playing, cannotPlay } = props;
  const n = frames.length;
  const strip = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState("");
  // Timing: per-frame holds, or one fps for all. Editor state only; the holds are the one truth.
  const [timing, setTiming] = useState<"hold" | "fps">(() => (fps !== null && n > 1 ? "fps" : "hold"));
  const dots = useMemo(() => thumbDots(builtOn), [builtOn]);
  // A phone: the timing switch is two icons, so the keys and the timing share one row.
  const phone = useMediaQuery("(max-width: 720px)");

  const move = props.onMove;
  const reorder = useFrameReorder(strip, content, move);

  const moveOne = useCallback(
    (index: number, delta: number) => {
      const to = index + delta;
      if (to < 0) return setStatus(`Frame ${index + 1} is already first.`);
      if (to >= n) return setStatus(`Frame ${index + 1} is already last.`);
      setStatus(`Frame ${index + 1} moved to position ${to + 1} of ${n}.`);
      move(index, to);
    },
    [move, n],
  );

  const keyDown = (index: number, event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    event.preventDefault();
    const delta = event.key === "ArrowLeft" ? -1 : 1;
    moveOne(index, delta);
    // The key keeps the focus where it now is: the same place in the list is a new element.
    const to = Math.min(n - 1, Math.max(0, index + delta));
    window.requestAnimationFrame(() => content.current?.querySelector<HTMLElement>(`[data-frame="${to}"]`)?.focus());
  };

  const current = frames[selected];
  const lastHold = current?.hold ?? 0;

  return (
    <div className={s.frameStrip} role="group" aria-label="Frames">
      <div className={s.frameScroll} ref={strip}>
        <div className={s.frameTrack} ref={content}>
          {frames.map((frame, i) => (
            <Fragment key={i}>
              <div className={s.frameCol}>
                <button
                  type="button"
                  className={s.frameKey}
                  data-frame={i}
                  data-lifted={reorder.drag?.index === i ? "true" : undefined}
                  aria-pressed={i === selected}
                  aria-label={`Frame ${i + 1} of ${n}, holds ${frame.hold} milliseconds`}
                  onPointerDown={(event) => reorder.begin(i, event)}
                  onDragStart={reorder.refuse}
                  onKeyDown={(event) => keyDown(i, event)}
                  onClick={() => {
                    if (reorder.swallowClick()) return;
                    props.onSelect(i);
                  }}
                >
                  <Thumb frame={frame} dots={dots} />
                  <span className={s.frameNo}>{i + 1}</span>
                </button>
                <span className={s.frameHold}>{frame.hold}ms</span>
              </div>
              <Connector
                index={i}
                last={i === n - 1}
                transition={frame.transition}
                loop={loop}
                onTransition={props.onTransition}
                onLoop={props.onLoop}
              />
            </Fragment>
          ))}
          {reorder.marker !== null && (
            <span className={s.frameMarker} style={{ left: reorder.marker }} data-testid="frame-marker" aria-hidden="true" />
          )}
        </div>
      </div>

      <div className={s.frameKeys} role="group" aria-label="Frame keys">
        <IconButton
          label="Add frame"
          icon={<Plus size={16} weight="bold" aria-hidden="true" />}
          disabled={!canAdd({ frames })}
          onClick={props.onAdd}
        />
        <IconButton
          label="Duplicate frame"
          icon={<Copy size={16} weight="bold" aria-hidden="true" />}
          disabled={!canAdd({ frames })}
          onClick={props.onDuplicate}
        />
        <IconButton
          label="Delete frame"
          icon={<Trash size={16} weight="bold" aria-hidden="true" />}
          disabled={n <= 1}
          onClick={props.onDelete}
        />
        <IconButton
          label={playing ? "Stop preview" : cannotPlay ?? "Play preview"}
          icon={playing ? <Stop size={16} weight="fill" aria-hidden="true" /> : <Play size={16} weight="fill" aria-hidden="true" />}
          pressed={playing}
          disabled={cannotPlay !== null && !playing}
          onClick={props.onPlay}
        />
      </div>

      <div className={s.frameTiming} role="group" aria-label="Timing">
        <Segmented<"hold" | "fps">
          label="Timing"
          value={timing}
          iconOnly={phone}
          options={[
            { value: "hold", label: "Hold", icon: <Hourglass size={14} weight="bold" aria-hidden="true" /> },
            { value: "fps", label: "fps", icon: <FilmStrip size={14} weight="bold" aria-hidden="true" /> },
          ]}
          onChange={setTiming}
        />
        <span className={s.frameField}>
          {timing === "hold" ? (
            <NumberField
              label={`Frame ${selected + 1} hold`}
              unit="ms"
              value={lastHold}
              min={0}
              max={HOLD_MS_MAX}
              step={50}
              onCommit={(ms) => props.onHold(selected, ms)}
            />
          ) : (
            <NumberField
              label="Frames per second"
              unit="fps"
              value={fps}
              // Unequal holds have no fps; an empty field says so instead of a made-up 10 (#481).
              placeholder="mixed"
              min={FPS_MIN}
              max={FPS_MAX}
              onCommit={props.onFps}
            />
          )}
        </span>
      </div>

      <span role="status" className="sr-only">
        {status}
      </span>
      {reorder.drag &&
        createPortal(
          <span
            className={s.frameCopy}
            style={{
              left: reorder.drag.x - reorder.drag.grabX,
              top: reorder.drag.y - reorder.drag.grabY,
              width: reorder.drag.width,
              height: reorder.drag.height,
            }}
            aria-hidden="true"
          >
            {reorder.drag.index + 1}
          </span>,
          document.body,
        )}
    </div>
  );
}

/** The dots of one thumbnail: the device laid out in the thumbnail's box, a sample of it when it is large. */
function thumbDots(builtOn: BuiltOn): { index: number; x: number; y: number; r: number }[] {
  const all = layoutDots(builtOn, THUMB_W, THUMB_H);
  const step = Math.max(1, Math.ceil(all.length / THUMB_DOTS));
  return all.filter((_, i) => i % step === 0).map((d) => ({ ...d, r: Math.min(d.r, 4.4) }));
}

/** A frame as a mini device: one dot per LED (a sample past 96), unlit LEDs as sockets. */
function Thumb({ frame, dots }: { frame: Frame; dots: { index: number; x: number; y: number; r: number }[] }) {
  return (
    <svg className={s.thumb} width={THUMB_W} height={THUMB_H} viewBox={`0 0 ${THUMB_W} ${THUMB_H}`} aria-hidden="true" focusable="false">
      {dots.map((dot) => {
        const hex = parseHex(frame.colors[dot.index] ?? BLACK_HEX);
        const lit = hex !== null && hex !== BLACK_HEX;
        return lit ? (
          <circle key={dot.index} cx={dot.x} cy={dot.y} r={Math.max(1, dot.r - 0.3)} fill={hex} />
        ) : (
          <circle key={dot.index} cx={dot.x} cy={dot.y} r={Math.max(1, dot.r - 0.3)} className={s.thumbOff} />
        );
      })}
    </svg>
  );
}

/**
 * The connector after frame `index`: the transition into the next frame, or, after the last, the
 * curl back to frame 1 that is the Loop key. Click it for Cut | Fade, how long, and the pace.
 */
function Connector({
  index,
  last,
  transition,
  loop,
  onTransition,
  onLoop,
}: {
  index: number;
  last: boolean;
  transition: Transition | undefined;
  loop: boolean;
  onTransition(index: number, patch: Partial<Transition>): void;
  onLoop(loop: boolean): void;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const kind = transition?.kind ?? "cut";
  const ms = transition?.ms ?? NEW_FADE_MS;
  const easing = transition?.easing ?? "linear";
  // The last connector only matters when the pattern loops: otherwise nothing follows the last frame.
  const live = !last || loop;
  const fading = kind === "fade" && live;
  // The ramp says Fade; the word says how long, or "cut". After the last frame it says loop or stop.
  const word = last ? (loop ? "loop" : "stop") : fading ? `${ms}ms` : "cut";
  const name = last
    ? `After frame ${index + 1}: ${loop ? `loops to frame 1, ${fading ? `fading ${ms} milliseconds` : "cutting"}` : "stops here"}`
    : `Frame ${index + 1} to ${index + 2}: ${fading ? `fade, ${ms} milliseconds` : "cut"}`;

  return (
    <div className={s.connector} data-last={last ? "true" : undefined}>
      <button
        type="button"
        ref={ref}
        className={s.connectorKey}
        data-kind={fading ? "fade" : "cut"}
        data-live={live ? "true" : "false"}
        data-open={open ? "true" : undefined}
        aria-label={name}
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((v) => !v)}
      >
        {last ? (
          <ArrowBendUpLeft size={16} weight="bold" aria-hidden="true" />
        ) : (
          <svg className={s.connectorGlyph} width="28" height="14" viewBox="0 0 28 14" aria-hidden="true" focusable="false">
            {fading ? (
              <path d="M1 12 L27 2" className={s.connectorLine} />
            ) : (
              <path d="M14 1 V13" className={s.connectorLine} />
            )}
          </svg>
        )}
      </button>
      <span className={s.connectorWord}>{word}</span>
      <Popover
        anchor={ref}
        open={open}
        onClose={() => setOpen(false)}
        label={`Transition after frame ${index + 1}`}
        align="start"
      >
        <div className={s.transition}>
          <p className={s.transitionTitle}>
            {last ? `Frame ${index + 1} to frame 1` : `Frame ${index + 1} to frame ${index + 2}`}
          </p>
          {last && (
            <label className={s.transitionRow}>
              <span>Loop</span>
              <Toggle checked={loop} label="Loop back to frame 1" onChange={onLoop} />
            </label>
          )}
          <div className={s.transitionRow} data-off={live ? "false" : "true"}>
            <Segmented<"cut" | "fade">
              label={`Frame ${index + 1} transition`}
              value={kind}
              disabled={!live}
              options={[
                { value: "cut", label: "Cut" },
                { value: "fade", label: "Fade" },
              ]}
              onChange={(next) => onTransition(index, { kind: next })}
            />
          </div>
          <div className={s.transitionRow} data-off={fading ? "false" : "true"}>
            <span>Duration</span>
            <NumberField
              label={`Frame ${index + 1} fade duration`}
              unit="ms"
              value={ms}
              min={0}
              max={TRANSITION_MS_MAX}
              step={50}
              disabled={!fading}
              onCommit={(next) => onTransition(index, { ms: next })}
            />
          </div>
          <div className={s.transitionRow} data-stack="true" data-off={fading ? "false" : "true"}>
            <span>Pace</span>
            <Segmented<Easing>
              label={`Frame ${index + 1} fade pace`}
              value={easing}
              disabled={!fading}
              options={EASINGS}
              onChange={(next) => onTransition(index, { easing: next })}
            />
          </div>
        </div>
      </Popover>
    </div>
  );
}
