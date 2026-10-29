"use client";

import type { ReactNode } from "react";
import { DESK_Y } from "@/lib/caseForms";
import desk from "./desk.module.css";

/**
 * The desk: the surface the keyboard, the mouse and the microphone sit on, and the line the case
 * stands on. It is one quiet rule across the canvas, with no writing on it: the canvas carries
 * no text by default.
 *
 * It is drawn in canvas units inside one group, whose transform is the desk's view (written
 * straight onto the element while the view zooms or pans). The line keeps its pixel width
 * (`vector-effect: non-scaling-stroke`).
 */
export function DeskLine({
  svgRef,
  gRef,
}: {
  svgRef: React.RefObject<SVGSVGElement | null>;
  gRef: React.RefObject<SVGGElement | null>;
}) {
  return (
    <svg ref={svgRef} className={desk.caseSvg} aria-hidden="true">
      <g ref={gRef}>
        <line className={desk.deskRule} x1={-2} y1={DESK_Y} x2={322} y2={DESK_Y} />
      </g>
    </svg>
  );
}

/**
 * The inside of the case box: the panel we look through, and the four panels a part can sit
 * against (the top, the floor, the front and the back). They are the case's own lines, quiet,
 * so the reader can see where the panels are without the canvas turning into a drawing.
 *
 * The drawing is in a 100 x 100 box stretched to the case's own box, so it grows with the case.
 */
export function CasePanels() {
  return (
    <svg className={desk.casePanels} viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
      {/* The glass side, seen through. */}
      <rect x={3} y={3} width={94} height={94} rx={4} />
      {/* The panels: the roof, the floor, the front and the back. */}
      <path d="M6 14 H94" className={desk.casePanel} />
      <path d="M6 86 H94" className={desk.casePanel} />
      <path d="M13 6 V94" className={desk.casePanel} />
      <path d="M87 6 V94" className={desk.casePanel} />
    </svg>
  );
}

/**
 * The case box as a landmark: its rule, its glass and nothing else. It is never sampled and it
 * never glows: it is a box a person drew, and the parts sit against it.
 */
export function CaseBox({ children }: { children?: ReactNode }) {
  return (
    <>
      {children}
      <CasePanels />
    </>
  );
}
