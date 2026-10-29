"use client";

import { useEffect, useMemo, useState } from "react";
import { ArrowCounterClockwise, Check } from "@phosphor-icons/react";
import { useMotionOff } from "@/components/Motion";
import { Thumb } from "@/components/Thumb";
import { StepFrame } from "@/views/editor/OnboardStep";
import { progressParts } from "@/views/editor/setup";
import { useSnap } from "@/lib/store";
import s from "@/views/editor/onboarding.module.css";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { Layout } from "@/proto/Layout";

/** How long the sweep takes to cross the PC, with a breath after it. */
const SWEEP_MS = 3000;

/** The effects worth showing a new owner first, if the daemon has them. */
const FIRST_PICKS = ["aurora", "rainbow_wave", "spiral", "gradient", "fire", "solid"];

/** Three effects that need nothing but the lights. */
export function pickThree(effects: EffectSummary[] | null): EffectSummary[] {
  const plain = (effects ?? []).filter((e) => !e.needs.screen && !e.needs.audio);
  const out: EffectSummary[] = [];
  for (const id of FIRST_PICKS) {
    const hit = plain.find((e) => e.id === id);
    if (hit && !out.includes(hit)) out.push(hit);
    if (out.length === 3) return out;
  }
  for (const e of plain) {
    if (!out.includes(e)) out.push(e);
    if (out.length === 3) break;
  }
  return out;
}

/**
 * The finish. The real lights sweep across the PC in the order the parts sit: the sweep is the
 * proof that the map is right. Then the line, the counts (worked out from the layout, never
 * typed in), and three effects as live tiles that play on the PC when they are pressed.
 *
 * Celebrated once and quietly: the dots light one after another. No confetti.
 */
export function FinishStep({
  layout,
  devices,
  scoped,
  playing,
  onSweep,
  onPick,
  onDone,
  saveError,
}: {
  layout: Layout;
  devices: DeviceInfo[] | null;
  /** One device was redone: no tiles, no big claim about the whole PC. */
  scoped: boolean;
  /** The effect the owner picked, if they did. */
  playing: string | null;
  onSweep: () => void;
  onPick: (id: string) => void;
  onDone: () => void;
  saveError: string | null;
}) {
  const motionOff = useMotionOff();
  const effects = useSnap((st) => st.effects);
  const [swept, setSwept] = useState(false);

  // The sweep runs when the screen opens; the line follows it.
  useEffect(() => {
    onSweep();
    const t = setTimeout(() => setSwept(true), motionOff ? 500 : SWEEP_MS);
    return () => clearTimeout(t);
    // The sweep is asked for once, on arrival.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const parts = useMemo(() => progressParts(layout, devices), [layout, devices]);
  const fans = parts.filter((p) => layout.zones[p.index]?.part?.kind === "fan").length;
  const tiles = useMemo(() => pickThree(effects), [effects]);
  const sweepAgain = () => {
    setSwept(false);
    onSweep();
    setTimeout(() => setSwept(true), motionOff ? 500 : SWEEP_MS);
  };

  if (!swept) {
    return (
      <StepFrame
        kicker="Finish"
        title="Watch your PC."
        lead="A band of light crosses it."
        primary={{ label: "Done", icon: <Check size={16} weight="bold" />, disabled: true }}
      >
        <p className={s.watching}>
          <span className={s.sweepBar} data-still={motionOff} aria-hidden="true" />
        </p>
      </StepFrame>
    );
  }

  return (
    <StepFrame
      kicker={
        <>
          <span className="fig">{parts.length}</span> {parts.length === 1 ? "part" : "parts"}
          {fans > 0 && (
            <>
              {" · "}
              <span className="fig">{fans}</span> {fans === 1 ? "fan" : "fans"}
            </>
          )}
        </>
      }
      title={scoped ? "That part is mapped." : "Your PC is mapped."}
      lead={scoped ? undefined : "Pick a look to try."}
      primary={{ label: "Done", icon: <Check size={16} weight="bold" />, onClick: onDone }}
      secondary={{ label: "Sweep again", icon: <ArrowCounterClockwise size={14} />, onClick: sweepAgain }}
    >
      {!scoped && tiles.length > 0 && (
        <div className={s.tiles}>
          {tiles.map((e) => (
            <button
              key={e.id}
              type="button"
              className={s.tile}
              aria-pressed={playing === e.id}
              onClick={() => onPick(e.id)}
            >
              <Thumb id={e.id} size="sm" label={e.name} live />
              <span className={s.tileText}>
                <span className={s.tileName}>{e.name}</span>
                <span className={s.tileState}>{playing === e.id ? "Playing on your PC" : "Press to play"}</span>
              </span>
            </button>
          ))}
        </div>
      )}
      {saveError && <p className="error-line">Not saved: {saveError}</p>}
    </StepFrame>
  );
}
