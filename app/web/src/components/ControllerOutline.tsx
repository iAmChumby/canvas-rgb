"use client";

import type { KeyboardEvent, ReactNode } from "react";

/**
 * The 8BitDo Ultimate 2, drawn flat: a front view with every remappable button as a target,
 * and a small rear view holding two back paddles.
 *
 * It is a plan of the pad, not a picture of it: a hairline body, the buttons where they sit,
 * and nothing else. The research does not say which physical button is P1, P2, P3 or P4, so the
 * targets carry the schema's names (P1 to P4) and the plan does not claim which is which. P3 and
 * P4 are drawn as the small buttons beside the bumpers and P1 and P2 on the rear view; that is a
 * place to put four targets, not a statement from the adapter.
 *
 * Every target is at least 34 drawing units across, which is 28 px or more at the narrowest
 * width the plan is drawn at.
 */

export interface PadButton {
  /** The property key the button's mapping is stored under. */
  key: string;
  /** The name the schema gives the button. */
  label: string;
  /** Kept for callers that print their own short name; the plan prints its own. */
  print?: string;
}

type GlyphName = "select" | "start" | "home" | "menu" | "up" | "down" | "left" | "right";

/** What is drawn on a button: letters, or a small glyph. */
type Mark = { text: string } | { glyph: GlyphName };

interface Spot {
  cx: number;
  cy: number;
  /** The drawn size; a circle when `round`. */
  w: number;
  h: number;
  round?: boolean;
  mark: Mark;
}

const FRONT_W = 340;

/** The mirror of a spot across the pad's centre line. */
const mirror = (s: Spot, mark: Mark): Spot => ({ ...s, cx: FRONT_W - s.cx, mark });

const L2: Spot = { cx: 78, cy: 17, w: 60, h: 24, mark: { text: "L2" } };
const L1: Spot = { cx: 80, cy: 51, w: 60, h: 24, mark: { text: "L1" } };
const P3: Spot = { cx: 26, cy: 60, w: 34, h: 26, mark: { text: "P3" } };
const L3: Spot = { cx: 72, cy: 100, w: 44, h: 44, round: true, mark: { text: "L3" } };

/** Where each button of the front view sits. */
const FRONT: Record<string, Spot> = {
  l2: L2,
  r2: mirror(L2, { text: "R2" }),
  l1: L1,
  r1: mirror(L1, { text: "R1" }),
  "paddle-3": P3,
  "paddle-4": mirror(P3, { text: "P4" }),
  l3: L3,
  r3: { cx: 222, cy: 152, w: 44, h: 44, round: true, mark: { text: "R3" } },
  select: { cx: 132, cy: 86, w: 26, h: 26, round: true, mark: { glyph: "select" } },
  home: { cx: 170, cy: 86, w: 30, h: 30, round: true, mark: { glyph: "home" } },
  start: { cx: 208, cy: 86, w: 26, h: 26, round: true, mark: { glyph: "start" } },
  menu: { cx: 170, cy: 128, w: 24, h: 24, round: true, mark: { glyph: "menu" } },
  "dpad-up": { cx: 118, cy: 126, w: 26, h: 26, round: true, mark: { glyph: "up" } },
  "dpad-down": { cx: 118, cy: 178, w: 26, h: 26, round: true, mark: { glyph: "down" } },
  "dpad-left": { cx: 92, cy: 152, w: 26, h: 26, round: true, mark: { glyph: "left" } },
  "dpad-right": { cx: 144, cy: 152, w: 26, h: 26, round: true, mark: { glyph: "right" } },
  y: { cx: 268, cy: 80, w: 28, h: 28, round: true, mark: { text: "Y" } },
  x: { cx: 242, cy: 106, w: 28, h: 28, round: true, mark: { text: "X" } },
  b: { cx: 294, cy: 106, w: 28, h: 28, round: true, mark: { text: "B" } },
  a: { cx: 268, cy: 132, w: 28, h: 28, round: true, mark: { text: "A" } },
};

/** The two paddles of the rear view. */
const REAR: Record<string, Spot> = {
  "paddle-1": { cx: 42, cy: 82, w: 32, h: 46, mark: { text: "P1" } },
  "paddle-2": { cx: 158, cy: 82, w: 32, h: 46, mark: { text: "P2" } },
};

/** The body's outline: a wide top and two grips. */
const BODY =
  "M 78 58 L 262 58 C 286 58 304 70 312 96 C 322 128 334 168 334 192 C 334 214 322 236 306 236 " +
  "C 290 236 282 220 276 208 C 270 200 262 196 244 196 L 96 196 C 78 196 70 200 64 208 " +
  "C 58 220 50 236 34 236 C 18 236 6 214 6 192 C 6 168 18 128 28 96 C 36 70 54 58 78 58 Z";

/** The D-pad's cross, drawn once under its four targets. */
const DPAD =
  "M 105 126 L 131 126 L 131 139 L 157 139 L 157 165 L 131 165 L 131 178 " +
  "L 105 178 L 105 165 L 79 165 L 79 139 L 105 139 Z";

/** The name each key carries on the plan: the schema's key without its prefix. */
function placeOf(key: string): string {
  if (key.startsWith("button-")) return key.slice("button-".length);
  const paddle = /paddle-(\d+)$/.exec(key);
  if (paddle) return `paddle-${paddle[1]}`;
  return key;
}

/** A glyph, drawn around the point (0, 0). */
function Glyph({ name }: { name: GlyphName }): ReactNode {
  switch (name) {
    case "select":
      return (
        <>
          <rect className="pad-glyph" x={-5} y={-3} width={7} height={7} rx={1} />
          <rect className="pad-glyph" x={-2} y={-6} width={7} height={7} rx={1} />
        </>
      );
    case "start":
      return <path className="pad-glyph" d="M -5 -4 H 5 M -5 0 H 5 M -5 4 H 5" />;
    case "home":
      return <path className="pad-glyph" d="M -6 1 L 0 -6 L 6 1 M -4 0 V 6 H 4 V 0" />;
    case "menu":
      return <path className="pad-glyph" d="M -5 0 H -4 M -0.5 0 H 0.5 M 4 0 H 5" />;
    case "up":
      return <path className="pad-glyph" d="M -4 2 L 0 -3 L 4 2" />;
    case "down":
      return <path className="pad-glyph" d="M -4 -2 L 0 3 L 4 -2" />;
    case "left":
      return <path className="pad-glyph" d="M 2 -4 L -3 0 L 2 4" />;
    default:
      return <path className="pad-glyph" d="M -2 -4 L 3 0 L -2 4" />;
  }
}

/**
 * The plan of the pad. `buttons` are the remappable ones; each one is a target that picks it.
 */
export function ControllerOutline({
  buttons,
  selected,
  onPick,
}: {
  buttons: PadButton[];
  selected: string | null;
  onPick: (key: string) => void;
}) {
  const onKey = (e: KeyboardEvent<SVGGElement>, key: string) => {
    if (e.key !== "Enter" && e.key !== " ") return;
    e.preventDefault();
    onPick(key);
  };

  const target = (b: PadButton, spot: Spot) => {
    const on = selected === b.key;
    // The hit area is never smaller than 34 units, whatever is drawn inside it.
    const hw = Math.max(spot.w, 34);
    const hh = Math.max(spot.h, 34);
    return (
      <g
        key={b.key}
        className="pad-target"
        role="button"
        tabIndex={0}
        aria-label={`Map ${b.label}`}
        aria-pressed={on}
        onClick={() => onPick(b.key)}
        onKeyDown={(e) => onKey(e, b.key)}
      >
        <title>{b.label}</title>
        <rect className="pad-hit" x={spot.cx - hw / 2} y={spot.cy - hh / 2} width={hw} height={hh} />
        {spot.round ? (
          <circle className="pad-btn" data-on={on ? "true" : "false"} cx={spot.cx} cy={spot.cy} r={spot.w / 2} />
        ) : (
          <rect
            className="pad-btn"
            data-on={on ? "true" : "false"}
            x={spot.cx - spot.w / 2}
            y={spot.cy - spot.h / 2}
            width={spot.w}
            height={spot.h}
            rx={7}
          />
        )}
        {"text" in spot.mark ? (
          <text className="pad-print" x={spot.cx} y={spot.cy} textAnchor="middle" dominantBaseline="central">
            {spot.mark.text}
          </text>
        ) : (
          <g transform={`translate(${spot.cx} ${spot.cy})`} pointerEvents="none">
            <Glyph name={spot.mark.glyph} />
          </g>
        )}
      </g>
    );
  };

  const front = buttons.filter((b) => FRONT[placeOf(b.key)]);
  const rear = buttons.filter((b) => REAR[placeOf(b.key)]);

  return (
    <div className="pad-views">
      <svg className="pad-plan" viewBox="0 0 340 244" role="group" aria-label="The 8BitDo Ultimate 2, front">
        <path className="pad-body" d={BODY} />
        <path className="pad-body" d={DPAD} />
        {front.map((b) => target(b, FRONT[placeOf(b.key)]!))}
      </svg>
      <div className="pad-rear">
        <svg className="pad-plan" viewBox="0 0 200 124" role="group" aria-label="The 8BitDo Ultimate 2, rear">
          <g transform="translate(15 2) scale(0.5)">
            <path className="pad-body" d={BODY} />
          </g>
          {rear.map((b) => target(b, REAR[placeOf(b.key)]!))}
        </svg>
        <span className="pad-caption">Rear</span>
      </div>
    </div>
  );
}
