import type { ReactNode } from "react";
import type { DeviceKind } from "@/proto/DeviceKind";
import desk from "./desk.module.css";

/**
 * What a zone is, drawn inside its box in thin lines: a fan is a hub with blades, a RAM stick
 * has a heat spreader and its gold contacts, a keyboard has rows of keys, a mouse has its
 * buttons and wheel, a microphone is a capsule on a stand, a headset is a band with two cups.
 * A strip is already a thin bar and needs none. The LEDs stay the dots on the overlay canvas, at
 * the places the daemon samples; this is only the picture around them.
 *
 * The drawing is in a 100 x 100 box stretched to the zone (`preserveAspectRatio="none"`) with
 * lines that keep their pixel width.
 */
export function ZoneArt({ kind, fan }: { kind: DeviceKind | null; fan: boolean }) {
  const art = pick(kind, fan);
  if (!art) return null;
  return (
    <svg className={desk.art} viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
      {art}
    </svg>
  );
}

function pick(kind: DeviceKind | null, fan: boolean): ReactNode {
  if (fan) return FAN;
  switch (kind) {
    case "dram":
      return RAM;
    case "keyboard":
      return KEYBOARD;
    case "mouse":
      return MOUSE;
    case "microphone":
      return MIC;
    case "headset":
      return HEADSET;
    case "gpu":
      return GPU;
    default:
      return null;
  }
}

const FAN = (
  <>
    <circle cx={50} cy={50} r={11} />
    {[0, 1, 2, 3, 4, 5, 6].map((i) => {
      const a = (i / 7) * Math.PI * 2;
      const p = (r: number, da: number) => `${50 + Math.cos(a + da) * r} ${50 + Math.sin(a + da) * r}`;
      return <path key={i} d={`M${p(11, 0)} Q${p(26, 0.25)} ${p(33, 1.0)}`} />;
    })}
  </>
);

const RAM = (
  <>
    {/* The heat spreader, with its notch at the top. */}
    <path d="M14 3 H86 V84 H14 Z" />
    <line x1={14} y1={14} x2={86} y2={14} />
    <line x1={50} y1={20} x2={50} y2={76} />
    {/* The contacts along the foot. */}
    {[22, 34, 46, 58, 70, 80].map((x) => (
      <line key={x} x1={x} y1={90} x2={x} y2={98} />
    ))}
  </>
);

const KEYBOARD = (
  <>
    <rect x={1} y={4} width={98} height={92} rx={6} />
    {[12, 31, 50].map((y, r) =>
      Array.from({ length: r === 1 ? 12 : 13 }, (_, i) => {
        const w = (94 - (r === 1 ? 11 : 12) * 1.6) / (r === 1 ? 12 : 13);
        return <rect key={`${y}-${i}`} x={3 + i * (w + 1.6) + (r === 1 ? 3 : 0)} y={y} width={w} height={14} rx={1.5} />;
      }),
    )}
    <rect x={3} y={72} width={14} height={14} rx={1.5} />
    <rect x={20} y={72} width={56} height={14} rx={1.5} />
    <rect x={79} y={72} width={18} height={14} rx={1.5} />
  </>
);

const MOUSE = (
  <>
    <rect x={8} y={2} width={84} height={96} rx={40} />
    <line x1={8} y1={40} x2={92} y2={40} />
    <line x1={50} y1={2} x2={50} y2={40} />
    <rect x={44} y={14} width={12} height={18} rx={5} />
  </>
);

const MIC = (
  <>
    {/* The capsule and its grille. */}
    <rect x={26} y={2} width={48} height={48} rx={24} />
    <line x1={26} y1={18} x2={74} y2={18} />
    <line x1={26} y1={30} x2={74} y2={30} />
    {/* The shock mount, the stem and the foot. */}
    <path d="M14 30 V42 Q14 66 50 66 Q86 66 86 42 V30" />
    <line x1={50} y1={66} x2={50} y2={90} />
    <line x1={26} y1={96} x2={74} y2={96} />
  </>
);

const HEADSET = (
  <>
    <path d="M10 56 C10 -6 90 -6 90 56" />
    <rect x={2} y={46} width={24} height={46} rx={10} />
    <rect x={74} y={46} width={24} height={46} rx={10} />
  </>
);

const GPU = (
  <>
    <rect x={0.5} y={3} width={99} height={88} rx={6} />
    <line x1={33.5} y1={3} x2={33.5} y2={91} />
    <line x1={66.5} y1={3} x2={66.5} y2={91} />
    <line x1={8} y1={97} x2={92} y2={97} />
  </>
);
