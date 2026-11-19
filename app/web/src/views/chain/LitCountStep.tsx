"use client";

// Counting the orange lights. One LED is on, in orange. The owner looks across the WHOLE case and
// says how many orange lights they can see in total: one for devices in a row (each takes its own
// LEDs), one per device for devices that share one signal and show the same lights (a splitter, a
// hub). That number is `copies`. The prompt says "in total" and the two pictures show both answers,
// because #470's owner read the question as "how many lights on each device".
//
// The ghost keys are the two honest ways out of "I can't answer": nothing lit up at all, and a
// case with no way to look inside.

import { useState } from "react";

import { Button } from "@/components/Button";
import { NumberField } from "@/components/NumberField";
import { COPIES_MAX } from "@/views/chain/limits";
import { LitAnswers } from "@/views/chain/LitAnswers";
import s from "@/views/chain/chain.module.css";

/**
 * The most the entry itself takes, so a count past the limit reaches this step's check, which can
 * say why, instead of being silently clamped by the entry.
 */
const ENTRY_MOST = 65535;

/** What each answer means, in plain words. Beside the entry until a refusal takes the line. */
export const HINT =
  "One orange light means your devices are in a row: each takes its own LEDs. An orange light on every device means they share one signal and show the same lights: type how many devices.";

export function LitCountStep({
  copies,
  onCopies,
  onNothing,
  onBlind,
}: {
  copies: number;
  onCopies: (n: number) => void;
  onNothing: () => void;
  onBlind: () => void;
}) {
  const [refusal, setRefusal] = useState<string | null>(null);

  const commit = (n: number) => {
    if (n > COPIES_MAX) {
      setRefusal(`The app counts up to ${COPIES_MAX}. Count again, or choose Later.`);
      return;
    }
    setRefusal(null);
    onCopies(n);
  };

  return (
    <div className={s.stepWrap}>
      <h3 className={s.prompt}>
        Look across your whole case. How many orange lights can you see in total, adding up every fan and strip?
      </h3>
      <div className={s.countRow}>
        <NumberField value={copies} min={1} max={ENTRY_MOST} label="Orange lights" onCommit={commit} />
        <p className={s.hint} data-bad={refusal ? "true" : undefined} aria-live="polite">
          {refusal ?? HINT}
        </p>
      </div>
      <LitAnswers />
      <div className={`${s.aside} ${s.escapes}`}>
        <Button variant="ghost" size="sm" onClick={onNothing}>
          Nothing lit up
        </Button>
        <Button variant="ghost" size="sm" onClick={onBlind}>
          I can&rsquo;t see inside
        </Button>
      </div>
    </div>
  );
}
