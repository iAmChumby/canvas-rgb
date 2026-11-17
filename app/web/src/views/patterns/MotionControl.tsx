"use client";

// The pattern's motion (#468): off, or Spin at a speed in laps a second, either way round. The
// editor saves it as `Motion::spin` and the daemon turns the pattern per tick from the effect's
// clock; the preview above turns it with the same maths. The slider keeps its room when Spin is
// off (it is dimmed, never removed), so turning Spin on moves nothing.

import { useState } from "react";
import { ArrowClockwise, ArrowCounterClockwise, ArrowsClockwise, ProhibitInset } from "@phosphor-icons/react";
import { Segmented } from "@/components/Segmented";
import { Slider } from "@/components/Slider";
import { SPIN_TURNS_DEFAULT, SPIN_TURNS_MAX, spinMotion, spinOf } from "@/lib/spin";
import type { Motion } from "@/proto/Motion";
import s from "@/views/patterns/patterns.module.css";

type Mode = "off" | "spin";

export function MotionControl({ motion, onMotion }: { motion: Motion; onMotion: (motion: Motion) => void }) {
  const speed = spinOf(motion);
  // Off remembers the speed the owner had, so Spin comes back to it and not to the default.
  const [last, setLast] = useState(speed !== null && speed !== 0 ? speed : SPIN_TURNS_DEFAULT);
  // The daemon reads a speed of 0 as no motion (`scene_engine.rs` filters `speed != 0`). Show Off
  // for it instead of a Spin that plays nothing; the slider still spans both directions, so a
  // minimum above zero would take reverse spin away (#481).
  const mode: Mode = speed === null || speed === 0 ? "off" : "spin";
  const shown = speed !== null && speed !== 0 ? speed : last;
  const spin = (value: number) => {
    if (value !== 0) setLast(value);
    onMotion(spinMotion(value));
  };
  return (
    <div className={s.motion} role="group" aria-label="Motion">
      <span className={s.motionKey}>Motion</span>
      <Segmented<Mode>
        label="Motion"
        value={mode}
        options={[
          { value: "off", label: "Off", icon: <ProhibitInset size={14} weight="bold" aria-hidden="true" /> },
          { value: "spin", label: "Spin", icon: <ArrowsClockwise size={14} weight="bold" aria-hidden="true" /> },
        ]}
        onChange={(next) => {
          if (next === mode) return;
          if (next === "spin") spin(last);
          else onMotion({ type: "none" });
        }}
      />
      <span className={s.motionSpeed} data-off={mode === "off" ? "true" : "false"}>
        <ArrowCounterClockwise size={14} weight="bold" aria-hidden="true" />
        <span className={s.motionSlider}>
          <Slider
            label="Spin speed, turns per second"
            value={shown}
            min={-SPIN_TURNS_MAX}
            max={SPIN_TURNS_MAX}
            step={0.05}
            disabled={mode === "off"}
            onChange={spin}
          />
        </span>
        <ArrowClockwise size={14} weight="bold" aria-hidden="true" />
        <span className={s.motionFigure}>{shown.toFixed(2)}</span>
        <span className={s.motionUnit}>turns/s</span>
      </span>
    </div>
  );
}
