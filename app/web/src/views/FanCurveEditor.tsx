"use client";

import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import type { CurvePoint } from "@/proto/CurvePoint";
import type { HotLimitStatus } from "@/proto/HotLimitStatus";
import { HOT_TEMP_C, hotFloor } from "@/lib/fan-rules";
import { clamp } from "@/lib/format";
import { dragThresholdPx } from "@/lib/hitTest";
import { cancelPress } from "@/lib/pressDrag";

const TEMP_MIN = 20;
const TEMP_MAX = 100;
const MARGIN = { left: 48, right: 14, top: 14, bottom: 46 };
const ASPECT = 0.62;

/**
 * A 2 to 8 point curve on a temperature × percent grid. The shaded band at the bottom is the
 * floor. From the source's hot limit (issue #130: the user's "don't exceed" line, drawn here
 * and draggable) a manual header is raised to its hot floor and, since issue #255, speeds up
 * further until the source is back under the line: that band is the speed-up zone. Past the
 * fixed safety ceiling a hatched band says every manual header runs at full speed. Points drag,
 * or take the arrow keys. The coordinates are read from the points list beside it.
 */
export function FanCurveEditor({
  header,
  points,
  floor,
  currentTemp,
  currentDuty,
  hot,
  onChange,
  onCommit,
  onHotCommit,
}: {
  header: string;
  points: CurvePoint[];
  floor: number;
  currentTemp: number | null;
  currentDuty: number | null;
  /** The hot line in force for this curve's source kind, or null when it is not known. */
  hot?: HotLimitStatus | null;
  onChange: (points: CurvePoint[]) => void;
  onCommit: (points: CurvePoint[]) => void;
  /** The hot line moved: drag end, arrow key, or the typed control beside the chart.
   * Resolves true when the service accepted it, false when the PUT failed (422). */
  onHotCommit?: (value: number) => Promise<boolean>;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(360);
  const drag = useRef<number | null>(null);
  /** The hot line while it is being dragged; null means the line in force from the status. */
  const [hotDrag, setHotDrag] = useState<number | null>(null);
  const hotDragRef = useRef(false);
  const hotPress = useRef<{ x: number; y: number; moved: boolean; touch: boolean; id: number } | null>(null);
  /** Where the press landed, and whether it has travelled far enough to be a drag. */
  const press = useRef<{ x: number; y: number; moved: boolean; touch: boolean; start: CurvePoint[]; id: number } | null>(null);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(Math.max(260, el.clientWidth)));
    ro.observe(el);
    setWidth(Math.max(260, el.clientWidth));
    return () => ro.disconnect();
  }, []);

  const height = Math.round(width * ASPECT);
  const plotW = width - MARGIN.left - MARGIN.right;
  const plotH = height - MARGIN.top - MARGIN.bottom;

  const xOf = (t: number) => MARGIN.left + ((t - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)) * plotW;
  const yOf = (p: number) => MARGIN.top + (1 - p / 100) * plotH;
  const tempOfX = (x: number) => TEMP_MIN + ((x - MARGIN.left) / plotW) * (TEMP_MAX - TEMP_MIN);
  const pctOfY = (y: number) => (1 - (y - MARGIN.top) / plotH) * 100;

  const toData = (e: PointerEvent<Element>) => {
    const box = wrapRef.current?.getBoundingClientRect();
    if (!box) return null;
    const x = e.clientX - box.left;
    const y = e.clientY - box.top;
    return { t: clamp(tempOfX(x), TEMP_MIN, TEMP_MAX), p: clamp(pctOfY(y), floor, 100) };
  };

  // Points, sorted by temperature, drawn as a flat-then-linear line, as the daemon evaluates it.
  const sorted = [...points].sort((a, b) => a.temp_c - b.temp_c);
  const line = sorted.map((p) => `${xOf(p.temp_c).toFixed(1)},${yOf(p.percent).toFixed(1)}`).join(" ");

  // During a drag the array keeps its order, so the dragged index stays valid. The bounds keep
  // every point between its neighbours; the sort happens on commit only.
  const update = (i: number, next: Partial<CurvePoint>, commit: boolean) => {
    const copy = points.map((p, j) => (j === i ? { ...p, ...next } : p));
    if (commit) onCommit([...copy].sort((a, b) => a.temp_c - b.temp_c));
    else onChange(copy);
  };

  const boundsFor = (i: number): { lo: number; hi: number } => {
    const me = points[i];
    let lo = TEMP_MIN;
    let hi = TEMP_MAX;
    points.forEach((p, j) => {
      if (j === i || !me) return;
      if (p.temp_c <= me.temp_c) lo = Math.max(lo, p.temp_c + 1);
      else hi = Math.min(hi, p.temp_c - 1);
    });
    return { lo, hi };
  };

  const onDown = (i: number, e: PointerEvent<SVGCircleElement>) => {
    // A press never starts a native drag or a text selection (#105); a second finger never
    // takes a point that is already held.
    cancelPress(e);
    if (drag.current !== null) return;
    drag.current = i;
    press.current = { x: e.clientX, y: e.clientY, moved: false, touch: e.pointerType === "touch", start: points.map((p) => ({ ...p })), id: e.pointerId };
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onMove = (e: PointerEvent<SVGCircleElement>) => {
    const i = drag.current;
    if (i === null) return;
    if (press.current && e.pointerId !== press.current.id) return;
    // A press that has not travelled a few pixels is a click: it does not nudge the point.
    const pr = press.current;
    if (pr && !pr.moved) {
      if (Math.hypot(e.clientX - pr.x, e.clientY - pr.y) < dragThresholdPx(pr.touch ? "touch" : "mouse")) return;
      pr.moved = true;
    }
    const d = toData(e);
    if (!d) return;
    const { lo, hi } = boundsFor(i);
    update(i, { temp_c: clamp(Math.round(d.t), lo, Math.max(lo, hi)), percent: Math.round(d.p) }, false);
  };

  const onUp = (e: PointerEvent<SVGCircleElement>) => {
    const i = drag.current;
    if (i === null) return;
    if (press.current && e.pointerId !== press.current.id) return;
    drag.current = null;
    const moved = press.current?.moved ?? true;
    press.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    // A click that never became a drag leaves the curve as it was: nothing to commit.
    if (!moved) return;
    const d = toData(e);
    if (!d) return;
    const { lo, hi } = boundsFor(i);
    update(i, { temp_c: clamp(Math.round(d.t), lo, Math.max(lo, hi)), percent: Math.round(d.p) }, true);
  };

  /** The browser took the pointer (a cancel): the drag is over and the curve goes back to how it was. */
  const onCancel = (e: PointerEvent<SVGCircleElement>) => {
    const pr = press.current;
    if (drag.current === null) return;
    if (press.current && e.pointerId !== press.current.id) return;

    drag.current = null;
    press.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    if (pr?.moved) onChange(pr.start);
  };

  const onKey = (i: number, e: KeyboardEvent<SVGCircleElement>) => {
    const p = points[i];
    if (!p) return;
    const big = e.shiftKey ? 5 : 1;
    let next: Partial<CurvePoint> | null = null;
    if (e.key === "ArrowUp") next = { percent: clamp(p.percent + big, floor, 100) };
    if (e.key === "ArrowDown") next = { percent: clamp(p.percent - big, floor, 100) };
    if (e.key === "ArrowRight") next = { temp_c: clamp(p.temp_c + big, TEMP_MIN, TEMP_MAX) };
    if (e.key === "ArrowLeft") next = { temp_c: clamp(p.temp_c - big, TEMP_MIN, TEMP_MAX) };
    if (!next) return;
    e.preventDefault();
    update(i, next, true);
  };

  // Issue #130: the "don't exceed" line. It moves by dragging or by the arrow keys,
  // clamped to what the user may set (40 °C up to the ceiling minus the gap). With no
  // hot line in force the handle is disabled instead of allowing up to 100 °C.
  const HOT_MIN = 40;
  const hotDisabled = !hot;
  const hotMax = hot ? hot.max_settable_c : HOT_MIN;
  const hotValue = hotDrag ?? hot?.limit_c ?? HOT_TEMP_C;
  const commitHot = async (v: number) => {
    const c = clamp(Math.round(v), HOT_MIN, hotMax);
    setHotDrag(c);
    // A failed PUT (a 422) clears the drag so the line snaps back to the status.
    const ok = (await onHotCommit?.(c)) ?? true;
    if (!ok) setHotDrag(null);
  };

  const onHotDown = (e: PointerEvent<SVGRectElement>) => {
    if (hotDisabled) return;
    cancelPress(e);
    if (hotDragRef.current) return;
    hotDragRef.current = true;
    hotPress.current = { x: e.clientX, y: e.clientY, moved: false, touch: e.pointerType === "touch", id: e.pointerId };
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onHotMove = (e: PointerEvent<SVGRectElement>) => {
    if (!hotDragRef.current) return;
    if (hotPress.current && e.pointerId !== hotPress.current.id) return;
    const pr = hotPress.current;
    if (pr && !pr.moved) {
      if (Math.hypot(e.clientX - pr.x, e.clientY - pr.y) < dragThresholdPx(pr.touch ? "touch" : "mouse")) return;
      pr.moved = true;
    }
    const d = toData(e);
    if (!d) return;
    setHotDrag(clamp(Math.round(d.t), HOT_MIN, hotMax));
  };

  const onHotUp = (e: PointerEvent<SVGRectElement>) => {
    if (!hotDragRef.current) return;
    if (hotPress.current && e.pointerId !== hotPress.current.id) return;
    hotDragRef.current = false;
    const moved = hotPress.current?.moved ?? true;
    hotPress.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    if (!moved) {
      setHotDrag(null);
      return;
    }
    const d = toData(e);
    if (!d) {
      setHotDrag(null);
      return;
    }
    commitHot(d.t);
  };

  const onHotCancel = (e: PointerEvent<SVGRectElement>) => {
    if (!hotDragRef.current) return;
    if (hotPress.current && e.pointerId !== hotPress.current.id) return;
    hotDragRef.current = false;
    hotPress.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    setHotDrag(null);
  };

  const onHotKey = (e: KeyboardEvent<SVGRectElement>) => {
    if (hotDisabled) return;
    const big = e.shiftKey ? 5 : 1;
    let next: number | null = null;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") next = hotValue + big;
    if (e.key === "ArrowLeft" || e.key === "ArrowDown") next = hotValue - big;
    if (next === null) return;
    e.preventDefault();
    commitHot(next);
  };

  const hotLeft = xOf(hotValue);
  // Issue #255 review: the flat hot floor (50 % CPU_FAN, 35 % others) still applies inside the
  // speed-up zone, so it is drawn as a dashed level from the line to the ceiling.
  const hotFloorY = yOf(hotFloor(header, floor));
  const hotLabel = `Don't exceed ${hotValue} °C${hot ? (hot.user_set ? " · set by you" : " · automatic") : ""}`;
  // The chart shows the figure only; the full words are on the handle (aria-valuetext) and in
  // the controls above the chart. The label sits at the top, left of its line when it fits (a
  // fan curve rises, so the top-left of any point is clear), else right of it, else against
  // the plot's right edge: never past either edge.
  const hotShort = `${hotValue} °C`;
  const plotRight = MARGIN.left + plotW;
  const CHAR_W = 8.5; // 14 px var(--fm), a monospace face: about 0.6 em a character
  const placeLabel = (x: number, text: string) => {
    const w = text.length * CHAR_W;
    if (x - 6 - w >= MARGIN.left) return { x: x - 6, anchor: "end" as const };
    if (x + 6 + w <= plotRight) return { x: x + 6, anchor: "start" as const };
    return { x: plotRight, anchor: "end" as const };
  };
  const hotPlace = placeLabel(hotLeft, hotShort);
  const ceilX = hot ? xOf(clamp(hot.ceiling_c, TEMP_MIN, TEMP_MAX)) : 0;
  // The ceiling's words read up its line, inside the hatch, so they fit at every chart width.
  const ceilLabelX = ceilX + 16 <= plotRight ? ceilX + 16 : ceilX - 6;
  const floorTop = yOf(floor);
  const grid = [20, 40, 60, 80, 100];
  const pctGrid = [0, 20, 40, 60, 80, 100];

  return (
    <div ref={wrapRef} className="curve" style={{ height }}>
      <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="group" aria-label="Fan curve">
        <defs>
          <pattern id="ceiling-hatch" width="8" height="8" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
            <line x1="0" y1="0" x2="0" y2="8" stroke="var(--down)" strokeWidth="1.5" opacity="0.45" />
          </pattern>
          <pattern id="speedup-hatch" width="8" height="8" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
            <line x1="0" y1="0" x2="0" y2="8" stroke="var(--warn)" strokeWidth="1.5" opacity="0.5" />
          </pattern>
        </defs>
        {pctGrid.map((p) => (
          <g key={`p${p}`}>
            <line x1={MARGIN.left} x2={width - MARGIN.right} y1={yOf(p)} y2={yOf(p)} stroke="var(--line)" />
            <text x={MARGIN.left - 8} y={yOf(p) + 5} textAnchor="end" fill="var(--mute)" fontSize={14} fontFamily="var(--fm)">
              {p}%
            </text>
          </g>
        ))}
        {grid.map((t) => (
          <g key={`t${t}`}>
            <line x1={xOf(t)} x2={xOf(t)} y1={MARGIN.top} y2={MARGIN.top + plotH} stroke="var(--line)" />
            <text x={xOf(t)} y={MARGIN.top + plotH + 20} textAnchor="middle" fill="var(--mute)" fontSize={14} fontFamily="var(--fm)">
              {t}
            </text>
          </g>
        ))}
        <text x={MARGIN.left + plotW} y={height - 6} textAnchor="end" fill="var(--mute)" fontSize={14}>
          temperature °C
        </text>

        {/* Floor: shaded from 0% up to the floor, across the whole plot. */}
        <rect
          x={MARGIN.left}
          y={floorTop}
          width={plotW}
          height={MARGIN.top + plotH - floorTop}
          fill="color-mix(in srgb, var(--down) 18%, transparent)"
        />
        {/* Safety ceiling (issue #130): fixed, never draggable. From the ceiling to the
            right edge every manual header on this source runs at full speed. */}
        {hot && (
          <>
            <rect
              x={xOf(clamp(hot.ceiling_c, TEMP_MIN, TEMP_MAX))}
              y={MARGIN.top}
              width={MARGIN.left + plotW - xOf(clamp(hot.ceiling_c, TEMP_MIN, TEMP_MAX))}
              height={plotH}
              fill="url(#ceiling-hatch)"
            />
            <line
              x1={xOf(clamp(hot.ceiling_c, TEMP_MIN, TEMP_MAX))}
              x2={xOf(clamp(hot.ceiling_c, TEMP_MIN, TEMP_MAX))}
              y1={MARGIN.top}
              y2={MARGIN.top + plotH}
              stroke="var(--down)"
              strokeDasharray="2 3"
            />
            <text
              x={ceilLabelX}
              y={MARGIN.top + plotH - 8}
              transform={`rotate(-90 ${ceilLabelX} ${MARGIN.top + plotH - 8})`}
              textAnchor="start"
              fill="var(--down)"
              stroke="var(--panel)"
              strokeWidth={4}
              paintOrder="stroke"
              fontSize={14}
              fontFamily="var(--fm)"
            >
              Full speed
            </text>
          </>
        )}
        {/* Issue #255 review: from the hot line to the ceiling is the speed-up zone. The flat
            floor (G4) still applies inside it, and the closed loop raises the header further
            until the source is back under the line. */}
        {hot && ceilX > hotLeft && (
          <g>
            <title>{`${header} speeds up above the don't exceed line`}</title>
            <rect x={hotLeft} y={MARGIN.top} width={ceilX - hotLeft} height={plotH} fill="url(#speedup-hatch)" />
            <line x1={hotLeft} x2={ceilX} y1={hotFloorY} y2={hotFloorY} stroke="var(--warn)" strokeDasharray="2 4" />
            <text
              x={hotLeft + 14}
              y={MARGIN.top + plotH - 8}
              transform={`rotate(-90 ${hotLeft + 14} ${MARGIN.top + plotH - 8})`}
              textAnchor="start"
              fill="var(--warn)"
              stroke="var(--panel)"
              strokeWidth={4}
              paintOrder="stroke"
              fontSize={14}
              fontFamily="var(--fm)"
            >
              speeds up
            </text>
          </g>
        )}
        <line x1={hotLeft} x2={hotLeft} y1={MARGIN.top} y2={MARGIN.top + plotH} stroke="var(--warn)" strokeDasharray="4 4" />
        {/* The hot line's handle: drag it, or use the arrow keys. */}
        <rect
          x={hotLeft - 7}
          y={MARGIN.top}
          width={14}
          height={plotH}
          fill="transparent"
          className="curve-hot-handle"
          tabIndex={hotDisabled ? -1 : 0}
          role="slider"
          aria-label="Don't exceed temperature line"
          aria-disabled={hotDisabled || undefined}
          aria-valuemin={HOT_MIN}
          aria-valuemax={hotMax}
          aria-valuenow={hotValue}
          aria-valuetext={`${hotLabel}`}
          onPointerDown={onHotDown}
          onPointerMove={onHotMove}
          onPointerUp={onHotUp}
          onPointerCancel={onHotCancel}
          onLostPointerCapture={onHotUp}
          onKeyDown={onHotKey}
        />

        {currentTemp !== null && (
          <line
            x1={xOf(clamp(currentTemp, TEMP_MIN, TEMP_MAX))}
            x2={xOf(clamp(currentTemp, TEMP_MIN, TEMP_MAX))}
            y1={MARGIN.top}
            y2={MARGIN.top + plotH}
            stroke="var(--ink)"
            strokeDasharray="2 3"
          />
        )}

        {/* Where the curve sits right now: the temperature on the left, the duty on the curve. */}
        {currentTemp !== null && currentDuty !== null && (
          <g className="curve-now">
            <circle
              cx={xOf(clamp(currentTemp, TEMP_MIN, TEMP_MAX))}
              cy={yOf(clamp(currentDuty, 0, 100))}
              r={6}
              fill="var(--live)"
              stroke="var(--panel)"
              strokeWidth={2}
            />
            <circle
              cx={xOf(clamp(currentTemp, TEMP_MIN, TEMP_MAX))}
              cy={yOf(clamp(currentDuty, 0, 100))}
              r={10}
              fill="none"
              stroke="var(--live)"
              strokeWidth={1.5}
              opacity={0.5}
            />
          </g>
        )}

        <polyline points={line} fill="none" stroke="var(--a2)" strokeWidth={2.5} strokeLinejoin="round" />
        {sorted.map((p, i) => {
          const idx = points.indexOf(p);
          return (
            <circle
              // The key is the point's place in the array, not its numbers: a point that moves must
              // keep its node, or every step of a drag would destroy and rebuild the circle.
              key={idx}
              cx={xOf(p.temp_c)}
              cy={yOf(p.percent)}
              r={7}
              fill="var(--a2)"
              stroke="var(--panel)"
              strokeWidth={2}
              tabIndex={0}
              role="slider"
              aria-label={`Point ${i + 1}: ${p.temp_c} °C`}
              aria-valuetext={`${p.temp_c} °C, ${p.percent}%`}
              className="curve-point"
              onPointerDown={(e) => onDown(idx, e)}
              onPointerMove={onMove}
              onPointerUp={onUp}
              onPointerCancel={onCancel}
              onLostPointerCapture={onUp}
              onKeyDown={(e) => onKey(idx, e)}
            />
          );
        })}
        {/* Drawn last, over the curve, with a halo; it never takes the pointer. */}
        <text
          x={hotPlace.x}
          y={MARGIN.top + 16}
          textAnchor={hotPlace.anchor}
          fill="var(--warn)"
          stroke="var(--panel)"
          strokeWidth={4}
          paintOrder="stroke"
          fontSize={14}
          fontFamily="var(--fm)"
          pointerEvents="none"
        >
          {hotShort}
        </text>
      </svg>
    </div>
  );
}
