"use client";

import { useEffect, useRef, useState } from "react";
import { ArrowRight, ArrowsClockwise, Check, Circle } from "@phosphor-icons/react";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { useMotionOff } from "@/components/Motion";
import { Greeting } from "@/views/editor/Greeting";
import { Fold, StepFrame } from "@/views/editor/OnboardStep";
import { HELLO_EFFECT } from "@/views/editor/onboardEffects";
import { api } from "@/lib/endpoints";
import { patchSnap, refreshStatus, useSnap } from "@/lib/store";
import s from "@/views/editor/onboarding.module.css";

/**
 * The first screen. It is five screens in one, chosen by what is true right now:
 *
 *  - SignalRGB is running: the gate. It looks again every 5 s and goes by itself.
 *  - OpenRGB is still starting: "Waking up your lights". The count is a dash, never a zero.
 *  - No lights, or "Nothing lit up?": the ladder of things to try.
 *  - Otherwise the greeting, with the count, the one button and the quiet ones.
 */
export function HelloStep({
  onStart,
  onLater,
  play,
}: {
  onStart: () => void;
  onLater: () => void;
  play: (id: string) => Promise<void>;
}) {
  const status = useSnap((st) => st.status);
  const devices = useSnap((st) => st.devices);
  const [ladder, setLadder] = useState(false);

  const conflict = status?.conflicts.find((c) => c.kind === "signal_rgb") ?? null;
  const waking = status?.openrgb === "connecting";
  const found = (devices ?? []).filter((d) => d.connected).length;
  const mode: "gate" | "waking" | "ladder" | "happy" = conflict
    ? "gate"
    : waking
      ? "waking"
      : found === 0 || ladder
        ? "ladder"
        : "happy";

  // The real lights do one slow wave together, mirrored on the drawing by the live preview.
  const waved = useRef(false);
  useEffect(() => {
    if (mode !== "happy" || waved.current) return;
    waved.current = true;
    void play(HELLO_EFFECT);
  }, [mode, play]);

  if (mode === "gate") return <Gate onLater={onLater} onStart={onStart} />;
  if (mode === "waking") return <Waking onLater={onLater} />;
  if (mode === "ladder") {
    return (
      <Ladder
        volunteer={found > 0}
        onBack={found > 0 ? () => setLadder(false) : null}
        onStart={onStart}
        onLater={onLater}
      />
    );
  }
  return (
    <StepFrame
      center
      title={<Greeting />}
      primary={{ label: "Map my PC", icon: <ArrowRight size={16} weight="bold" />, onClick: onStart }}
      secondary={{ label: "Set up later", onClick: onLater }}
      tertiary={{ label: "Nothing lit up?", onClick: () => setLadder(true) }}
    >
      <Count value={found} />
    </StepFrame>
  );
}

/** "11 lights found", in mono. A dash while the count is not known. */
function Count({ value }: { value: number | null }) {
  return (
    <p className={s.count} aria-live="polite">
      <span className={s.countNumber} data-known={value !== null}>
        {value === null ? "—" : <AnimatedNumber value={value} />}
      </span>
      <span className={s.countWord}>{value === 1 ? "light found" : "lights found"}</span>
    </p>
  );
}

/** A pulse of LED dots: something is going on and nothing is wrong. */
function Waiting() {
  const off = useMotionOff();
  return (
    <span className={s.waiting} data-still={off} aria-hidden="true">
      <i />
      <i />
      <i />
      <i />
      <i />
    </span>
  );
}

function Gate({ onLater, onStart }: { onLater: () => void; onStart: () => void }) {
  // The daemon is asked again every 5 s; the screen goes by itself when SignalRGB is gone.
  useEffect(() => {
    const t = setInterval(() => void refreshStatus(), 5000);
    return () => clearInterval(t);
  }, []);
  return (
    <StepFrame
      kicker="SignalRGB"
      title="SignalRGB has your lights."
      lead="Quit it from the tray."
      primary={{ label: "Check again", icon: <ArrowsClockwise size={16} />, onClick: () => void refreshStatus() }}
      secondary={{ label: "Continue without lights", onClick: onStart }}
      tertiary={{ label: "Set up later", onClick: onLater }}
    >
      <p className={s.watching}>
        <Waiting />
        Watching for it to quit
      </p>
    </StepFrame>
  );
}

function Waking({ onLater }: { onLater: () => void }) {
  return (
    <StepFrame
      center
      title="Waking up your lights…"
      lead="This takes a moment."
      primary={{ label: "Map my PC", icon: <ArrowRight size={16} weight="bold" />, disabled: true }}
      tertiary={{ label: "Set up later", onClick: onLater }}
    >
      <Waiting />
      <Count value={null} />
    </StepFrame>
  );
}

/** One rung of the ladder: a mark that is filled when it is settled, and the words. */
function Rung({ done, children, tag }: { done?: boolean; children: string; tag?: string }) {
  return (
    <li className={s.rung} data-done={done ? "true" : "false"}>
      <span className={s.rungMark} aria-hidden="true">
        {done ? <Check size={12} weight="bold" /> : <Circle size={8} weight="fill" />}
      </span>
      <span className={s.rungText}>{children}</span>
      {tag && <span className={s.rungTag}>{tag}</span>}
    </li>
  );
}

function Ladder({
  volunteer,
  onBack,
  onStart,
  onLater,
}: {
  /** The owner opened it with "Nothing lit up?", so lights were found. */
  volunteer: boolean;
  onBack: (() => void) | null;
  onStart: () => void;
  onLater: () => void;
}) {
  const status = useSnap((st) => st.status);
  const [busy, setBusy] = useState(false);
  const [asked, setAsked] = useState(false);
  const noOtherApps = (status?.conflicts.length ?? 0) === 0;

  const rescan = async () => {
    setBusy(true);
    try {
      const res = await api.rescanDevices();
      patchSnap({ devices: res.devices });
      await refreshStatus();
    } catch {
      // The daemon did not answer: the rescan is asked again on the next press.
    } finally {
      setAsked(true);
      setBusy(false);
    }
  };

  return (
    <StepFrame
      title={volunteer ? "Nothing lit up?" : "No lights found."}
      lead="Try these, top to bottom."
      onBack={onBack}
      primary={{
        label: busy ? "Looking…" : "Rescan",
        icon: <ArrowsClockwise size={16} />,
        disabled: busy,
        onClick: () => void rescan(),
      }}
      secondary={{ label: "Continue without lights", onClick: onStart }}
      tertiary={onBack ? null : { label: "Set up later", onClick: onLater }}
    >
      <ol className={s.ladder}>
        <Rung done={noOtherApps} tag={noOtherApps ? "SignalRGB not seen" : undefined}>
          Quit other RGB apps.
        </Rung>
        <Rung>Replug it, or try another port.</Rung>
        <Rung>Turn off Fast Startup.</Rung>
        <Rung>Rescan.</Rung>
      </ol>
      {asked && !busy && !volunteer && <p className={s.hint}>Still no lights.</p>}
      <Fold label="More help">
        <p className={s.hint}>Close iCUE, Armoury Crate and Razer Synapse.</p>
        <p className={s.hint}>Plug USB lights straight into the PC.</p>
        <p className={s.hint}>Restart Windows, not just shut down.</p>
      </Fold>
    </StepFrame>
  );
}
