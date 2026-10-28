"use client";

import type { ReactNode, RefObject } from "react";
import { DESK_Y, zoneInSlot, type CaseSpec, type Slot } from "@/lib/caseForms";
import type { PlacedZone } from "@/proto/PlacedZone";
import desk from "./desk.module.css";

/**
 * The generic case drawn behind the zones: one calm line drawing of the PC form factor the
 * owner picked, so the zones sit where their hardware sits. It is a picture and a set of snap
 * slots, nothing the daemon reads: the effect is sampled where each zone is, wherever that is.
 *
 * It is drawn in canvas units inside one group, whose transform is the desk's view (written
 * straight onto the element while the view zooms or pans). Lines keep their pixel width
 * (`vector-effect: non-scaling-stroke`) and text is 14 css pixels at any zoom (`--k` is the
 * stage's scale, set beside the transform).
 *
 * Each line is drawn twice, a dark keyline under a light line, as the zones are, so it holds on
 * any picture in light and dark.
 */
export function CaseBackdrop({
  spec,
  zones,
  k,
  panX,
  panY,
  svgRef,
  gRef,
}: {
  spec: CaseSpec;
  zones: PlacedZone[];
  /** Css pixels per canvas unit. */
  k: number;
  panX: number;
  panY: number;
  svgRef: RefObject<SVGSVGElement | null>;
  gRef: RefObject<SVGGElement | null>;
}) {
  const { dims: d, board, cpu, slots } = spec;
  const visible = zones.filter((z) => z.visible);
  const taken = (s: Slot) => visible.some((z) => zoneInSlot(z, s));
  const unit = 14 / k; // 14 css pixels, in canvas units.
  const textFits = (text: string, w: number, h: number) => text.length * 7.8 + 8 <= w * k && 17 <= h * k;
  const textBox = (x: number, y: number, text: string, anchor: "start" | "end" | "middle") => {
    const w = (text.length * 7.8) / k;
    const left = anchor === "start" ? x : anchor === "end" ? x - w : x - w / 2;
    return { x: left - 2 / k, y: y - unit, w: w + 4 / k, h: unit * 1.25 };
  };
  /** True when a rectangle touches a slot (edges excluded: they are lines) or the CPU. */
  const clash = (b: { x: number; y: number; w: number; h: number }, extra: Slot[] = []) =>
    [...slots.filter((s) => s.kind !== "edge"), ...extra].some(
      (s) => b.x < s.x + s.w + 1 && s.x < b.x + b.w + 1 && b.y < s.y + s.h + 1 && s.y < b.y + b.h + 1,
    );
  const cpuSlot: Slot = { id: "cpu", kind: "gpu", label: "CPU", ...cpu };

  const fans = slots.filter((s) => s.kind === "fan");
  const rams = slots.filter((s) => s.kind === "ram");
  const gpu = slots.find((s) => s.kind === "gpu");
  const edges = slots.filter((s) => s.kind === "edge");
  const deskSlots = slots.filter((s) => s.kind === "desk" && s.peripheral);

  // The board's name goes in the first bottom corner of the board that touches no slot.
  const boardText = "Motherboard";
  const boardAt = (
    [
      { x: cpu.x + cpu.w + 12, y: board.y + unit + 4, a: "start" },
      { x: board.x + 6, y: board.y + board.h - 4, a: "start" },
      { x: board.x + board.w - 6, y: board.y + board.h - 4, a: "end" },
    ] as const
  ).find((c) => {
    const b = textBox(c.x, c.y, boardText, c.a);
    const inside = b.x >= board.x && b.x + b.w <= board.x + board.w && b.y >= board.y && b.y + b.h <= board.y + board.h + 2;
    return inside && !clash(b, [cpuSlot]);
  });
  const deskText = "Desk";
  const deskAt = (
    [
      { x: 314, y: 197, a: "end" },
      { x: 6, y: 197, a: "start" },
    ] as const
  ).find((c) => !clash(textBox(c.x, c.y, deskText, c.a)));

  const twice = (kids: ReactNode) => (
    <>
      <g className={desk.caseHalo}>{kids}</g>
      <g className={desk.caseLine}>{kids}</g>
    </>
  );

  return (
    <svg ref={svgRef} className={desk.caseSvg} aria-hidden="true" style={{ ["--k" as string]: k }}>
      <g ref={gRef} transform={`scale(${k}) translate(${panX} ${panY})`}>
        {/* The desk: the surface the keyboard, mouse and microphone sit on. */}
        <rect className={desk.caseDesk} x={-2} y={DESK_Y} width={324} height={200 - DESK_Y + 2} />
        {twice(<line x1={-2} y1={DESK_Y} x2={322} y2={DESK_Y} />)}
        {deskAt && (
          <text className={desk.caseText} x={deskAt.x} y={deskAt.y} textAnchor={deskAt.a}>
            {deskText}
          </text>
        )}

        {/* The case. */}
        {d.panels ? (
          <>
            <rect className={desk.caseBody} x={d.x} y={d.y} width={d.w} height={d.h} rx={7} />
            {twice(
              <>
                <rect x={d.x} y={d.y} width={d.w} height={d.h} rx={7} />
                {/* The glass side panel. */}
                <rect x={d.x + 3} y={d.y + 3} width={d.w - 6} height={d.h - 6} rx={5} className={desk.caseSoft} />
                <line className={desk.caseSoft} x1={d.x + d.w - 26} y1={d.y + d.h - 3} x2={d.x + d.w - 8} y2={d.y + d.h - 23} />
                <line className={desk.caseSoft} x1={d.x + d.w - 18} y1={d.y + d.h - 3} x2={d.x + d.w - 5} y2={d.y + d.h - 17} />
                {/* Feet: the case stands on the desk line. */}
                <rect x={d.x + 8} y={d.y + d.h} width={14} height={DESK_Y - (d.y + d.h)} rx={1} />
                <rect x={d.x + d.w - 22} y={d.y + d.h} width={14} height={DESK_Y - (d.y + d.h)} rx={1} />
              </>,
            )}
          </>
        ) : (
          twice(
            <>
              {/* An open frame: a base plate and two posts, nothing closed. */}
              <rect x={d.x} y={DESK_Y - 5} width={d.w} height={5} rx={2} />
              <rect x={d.x + 3} y={d.y} width={4} height={DESK_Y - 5 - d.y} rx={1.5} />
              <rect x={d.x + d.w - 7} y={d.y} width={4} height={DESK_Y - 5 - d.y} rx={1.5} />
            </>,
          )
        )}

        {/* The motherboard and its CPU. */}
        <rect className={desk.caseBoard} x={board.x} y={board.y} width={board.w} height={board.h} rx={3} />
        {twice(
          <>
            <rect x={board.x} y={board.y} width={board.w} height={board.h} rx={3} className={desk.caseSoft} />
            <rect x={cpu.x} y={cpu.y} width={cpu.w} height={cpu.h} rx={3} className={desk.caseSoft} />
            {cpu.w * k < 56 && (
              <rect x={cpu.x + 6} y={cpu.y + 6} width={cpu.w - 12} height={cpu.h - 12} rx={1.5} className={desk.caseSoft} />
            )}
          </>,
        )}
        {cpu.w * k >= 56 && (
          <text className={desk.caseText} x={cpu.x + cpu.w / 2} y={cpu.y + cpu.h / 2 + unit * 0.35} textAnchor="middle">
            CPU
          </text>
        )}
        {boardAt && (
          <text className={desk.caseText} x={boardAt.x} y={boardAt.y} textAnchor={boardAt.a}>
            {boardText}
          </text>
        )}

        {/* Case edges: where an ARGB strip runs. */}
        {twice(
          edges.map((s) => (
            <line
              key={s.id}
              className={desk.caseDash}
              x1={s.x + s.w / 2 - (s.w >= s.h ? s.w / 2 : 0)}
              y1={s.y + s.h / 2 - (s.w >= s.h ? 0 : s.h / 2)}
              x2={s.x + s.w / 2 + (s.w >= s.h ? s.w / 2 : 0)}
              y2={s.y + s.h / 2 + (s.w >= s.h ? 0 : s.h / 2)}
            />
          )),
        )}

        {/* RAM slots, each with its two latches. */}
        {twice(
          rams.map((s) => (
            <g key={s.id}>
              <rect x={s.x - 0.8} y={s.y - 1} width={s.w + 1.6} height={s.h + 2} rx={1.2} />
              <rect x={s.x + s.w / 2 - 1.5} y={s.y - 4} width={3} height={3} rx={0.8} className={desk.caseSoft} />
              <rect x={s.x + s.w / 2 - 1.5} y={s.y + s.h + 1} width={3} height={3} rx={0.8} className={desk.caseSoft} />
            </g>
          )),
        )}

        {/* The graphics card slot. */}
        {gpu &&
          twice(
            <>
              <rect x={gpu.x - 6} y={gpu.y + gpu.h + 2} width={gpu.w + 10} height={3} rx={1} />
              <rect x={gpu.x - 7} y={gpu.y - 4} width={3} height={gpu.h + 10} rx={1} className={desk.caseSoft} />
              {!taken(gpu) && <rect x={gpu.x} y={gpu.y} width={gpu.w} height={gpu.h} rx={2} className={desk.caseDash} />}
            </>,
          )}
        {gpu && !taken(gpu) && textFits("Graphics card", gpu.w, gpu.h) && (
          <text className={desk.caseText} x={gpu.x + gpu.w / 2} y={gpu.y + gpu.h / 2 + unit * 0.35} textAnchor="middle">
            Graphics card
          </text>
        )}

        {/* Fan positions: a square frame, a round opening and four screws. */}
        {twice(
          fans.map((s) => {
            const r = s.w / 2;
            const cx = s.x + r;
            const cy = s.y + r;
            const empty = !taken(s);
            return (
              <g key={s.id}>
                <rect x={s.x - 1} y={s.y - 1} width={s.w + 2} height={s.h + 2} rx={4} />
                <circle cx={cx} cy={cy} r={r - 2.5} className={desk.caseSoft} />
                {[
                  [s.x + 1.5, s.y + 1.5],
                  [s.x + s.w - 1.5, s.y + 1.5],
                  [s.x + 1.5, s.y + s.h - 1.5],
                  [s.x + s.w - 1.5, s.y + s.h - 1.5],
                ].map(([sx, sy], i) => (
                  <circle key={i} cx={sx} cy={sy} r={0.7} className={desk.caseSoft} />
                ))}
                {empty && (
                  <>
                    <circle cx={cx} cy={cy} r={r * 0.2} className={desk.caseSoft} />
                    {[0, 1, 2, 3, 4, 5, 6].map((i) => {
                      const a = (i / 7) * Math.PI * 2;
                      const x1 = cx + Math.cos(a) * r * 0.22;
                      const y1 = cy + Math.sin(a) * r * 0.22;
                      const x2 = cx + Math.cos(a + 0.9) * (r - 4);
                      const y2 = cy + Math.sin(a + 0.9) * (r - 4);
                      return <path key={i} d={`M${x1} ${y1} Q${cx + Math.cos(a + 0.2) * r * 0.7} ${cy + Math.sin(a + 0.2) * r * 0.7} ${x2} ${y2}`} className={desk.caseSoft} />;
                    })}
                  </>
                )}
              </g>
            );
          }),
        )}
        {/* The desk's places that are waiting for their device. */}
        {deskSlots.map((s) =>
          taken(s) ? null : (
            <g key={s.id}>
              {twice(<rect x={s.x} y={s.y} width={s.w} height={s.h} rx={3} className={desk.caseDash} />)}
              {textFits(s.label, s.w, s.h) && (
                <text className={desk.caseText} x={s.x + s.w / 2} y={s.y + s.h / 2 + unit * 0.35} textAnchor="middle">
                  {s.label}
                </text>
              )}
            </g>
          ),
        )}
      </g>
    </svg>
  );
}
