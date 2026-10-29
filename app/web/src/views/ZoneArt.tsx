import type { ReactNode } from "react";
import type { Facing } from "@/proto/Facing";
import type { PartKind } from "@/proto/PartKind";
import { isFlat } from "@/views/editor/parts";
import desk from "./desk.module.css";

/**
 * What a part looks like, drawn inside its own box in thin lines.
 *
 * One glyph per kind of part, and the `facing` says which way it points, which is what decides
 * the drawing: a fan that points at you shows its ring of blades, one that points up or down is
 * seen from the side and is a thin bar with its LEDs along it. The picture matches what the
 * engine samples, so the reader can trust it.
 *
 * The drawing is crude on purpose and it is the same everywhere: one stroke weight, two fills,
 * no gradients, no shadows. The LEDs are the dots on the overlay canvas, at the places the
 * daemon samples; this is only the picture around them.
 *
 * The drawing is in a 100 x 100 box stretched to the part's own box (`preserveAspectRatio="none"`)
 * with lines that keep their pixel width.
 */
export function PartArt({
  kind,
  facing = "viewer",
  className,
}: {
  kind: PartKind;
  facing?: Facing;
  className?: string;
}) {
  const art = pick(kind, facing);
  if (!art) return null;
  return (
    <svg className={`${desk.art} ${className ?? ""}`} viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
      {art}
    </svg>
  );
}

function pick(kind: PartKind, facing: Facing): ReactNode {
  switch (kind) {
    case "fan":
      return isFlat(facing) ? FAN_FLAT : facing === "left" || facing === "right" ? FAN_STANDING : FAN;
    case "ram":
      return RAM;
    case "gpu":
      return GPU;
    case "cpu":
      return CPU;
    case "pump":
      return PUMP;
    case "radiator":
      return RADIATOR;
    case "strip":
      return STRIP;
    case "keyboard":
      return KEYBOARD;
    case "mouse":
      return MOUSE;
    case "microphone":
      return MIC;
    case "headset":
      return HEADSET;
    case "case":
      return CASE;
    default:
      return GENERIC;
  }
}

/** A fan pointing at the reader: the bracket, its four holes, the blades and the hub notch. */
const FAN = (
  <>
    <rect x={4} y={4} width={92} height={92} rx={6} />
    {([
      [13, 13],
      [87, 13],
      [13, 87],
      [87, 87],
    ] as const).map(([cx, cy]) => (
      <circle key={`${cx}-${cy}`} cx={cx} cy={cy} r={2.6} />
    ))}
    <circle cx={50} cy={50} r={44} />
    <circle cx={50} cy={50} r={9} className={desk.artFill} />
    {/* The hub notch: the little tab at the top of the hub. */}
    <path d="M46 6 H54 V14 H46 Z" className={desk.artFill} />
  </>
);

/** A fan seen from the side, lying flat: a bar with its LEDs along it. */
const FAN_FLAT = (
  <>
    <rect x={2} y={28} width={96} height={44} rx={5} />
    <line x1={2} y1={50} x2={98} y2={50} />
    {[10, 22, 34, 46, 58, 70, 82, 92].map((x) => (
      <line key={x} x1={x} y1={34} x2={x} y2={46} />
    ))}
  </>
);

/** A fan seen from the side, standing up. */
const FAN_STANDING = (
  <>
    <rect x={28} y={2} width={44} height={96} rx={5} />
    <line x1={50} y1={2} x2={50} y2={98} />
    {[10, 22, 34, 46, 58, 70, 82, 92].map((y) => (
      <line key={y} x1={34} y1={y} x2={46} y2={y} />
    ))}
  </>
);

/** A stick of RAM: the slot it sits in, the notch in its foot and its LEDs along the side. */
const RAM = (
  <>
    <path d="M26 2 H74 V88 H58 V96 H42 V88 H26 Z" />
    {[18, 30, 42, 54, 66, 78].map((y) => (
      <line key={y} x1={74} y1={y} x2={88} y2={y} />
    ))}
  </>
);

/** A graphics card: a long box with the fans it carries, faint. */
const GPU = (
  <>
    <rect x={2} y={6} width={96} height={86} rx={4} />
    {/* The card's fans are panels, not lit details: the light of a card is in its LED row. */}
    <circle cx={28} cy={49} r={19} className={desk.artPanel} />
    <circle cx={64} cy={49} r={19} className={desk.artPanel} />
    {/* The bracket at the back of the card. */}
    <rect x={2} y={20} width={7} height={58} className={desk.artFill} />
  </>
);

/** A CPU: a square with a notch in one corner. */
const CPU = (
  <>
    <path d="M24 4 H96 V96 H4 V24 Z" />
    {/* The chip's face is a panel: it is not lit by the part's own LEDs. */}
    <rect x={26} y={26} width={48} height={48} rx={2} className={desk.artPanel} />
  </>
);

/** A pump head: the cap, the motor and its mount. */
const PUMP = (
  <>
    <circle cx={50} cy={42} r={32} />
    <circle cx={50} cy={42} r={16} className={desk.artFill} />
    <path d="M44 74 H56 V90 H44 Z" />
    <line x1={26} y1={90} x2={74} y2={90} />
  </>
);

/** A radiator: a wide box with its fins. */
const RADIATOR = (
  <>
    <rect x={2} y={14} width={96} height={72} rx={3} />
    {[12, 22, 32, 42, 52, 62, 72, 82, 92].map((x) => (
      <line key={x} x1={x} y1={20} x2={x} y2={80} />
    ))}
  </>
);

/** A strip: a bar with its LEDs along it. */
const STRIP = (
  <>
    <rect x={1} y={36} width={98} height={28} rx={3} />
    {[8, 20, 32, 44, 56, 68, 80, 92].map((x) => (
      <circle key={x} cx={x} cy={50} r={4} className={desk.artFill} />
    ))}
  </>
);

/** A keyboard: a slab and the rows of keys. */
const KEYBOARD = (
  <>
    <rect x={1} y={8} width={98} height={84} rx={5} />
    <line x1={8} y1={30} x2={92} y2={30} />
    <line x1={8} y1={50} x2={92} y2={50} />
    <line x1={8} y1={70} x2={92} y2={70} />
    <line x1={8} y1={86} x2={92} y2={86} />
  </>
);

/** A mouse: its shell, the split and the wheel. */
const MOUSE = (
  <>
    <rect x={10} y={2} width={80} height={96} rx={38} />
    <line x1={10} y1={42} x2={90} y2={42} />
    <line x1={50} y1={4} x2={50} y2={42} />
    <rect x={44} y={14} width={12} height={18} rx={5} className={desk.artFill} />
  </>
);

/** A microphone: the capsule, its grille and the stand. */
const MIC = (
  <>
    <rect x={26} y={2} width={48} height={48} rx={24} />
    <line x1={26} y1={18} x2={74} y2={18} />
    <line x1={26} y1={32} x2={74} y2={32} />
    <path d="M14 30 V42 Q14 66 50 66 Q86 66 86 42 V30" />
    <line x1={50} y1={66} x2={50} y2={90} />
    <line x1={26} y1={94} x2={74} y2={94} />
  </>
);

/** A headset: the band and its two cups. */
const HEADSET = (
  <>
    <path d="M10 56 C10 -6 90 -6 90 56" />
    <rect x={2} y={46} width={24} height={46} rx={10} />
    <rect x={74} y={46} width={24} height={46} rx={10} />
  </>
);

/** The case: the box, and the panel we look through. */
const CASE = (
  <>
    <rect x={3} y={3} width={94} height={94} rx={6} />
    <rect x={11} y={11} width={78} height={78} rx={4} className={desk.artFill} />
  </>
);

/** A box: a plain rectangle with a corner mark, so it reads as "something else". */
const GENERIC = (
  <>
    <rect x={6} y={6} width={88} height={88} rx={4} />
    <rect x={6} y={6} width={22} height={10} className={desk.artFill} />
  </>
);
