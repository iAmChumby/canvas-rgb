"use client";

// Counting the orange lights. One LED is on, in orange. The owner looks at the case and says how
// many orange lights they can see: one for a plain series of devices, more for a header whose
// devices all show the same LEDs (a splitter, a hub). That number is `copies`.
//
// The ghost keys are the two honest ways out of "I can't answer": nothing lit up at all, and a
// case with no way to look inside.

import { useState } from "react";

import { Button } from "@/components/Button";
import { NumberField } from "@/components/NumberField";
import { COPIES_MAX } from "@/views/chain/limits";
import { RunDrawing } from "@/views/chain/RunDrawing";
import s from "@/views/chain/chain.module.css";

/**
 * The most the entry itself takes, so a count past the limit reaches this step's check, which can
 * say why, instead of being silently clamped by the entry.
 */
const ENTRY_MOST = 65535;

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
      <h3 className={s.prompt}>Look at your case. How many orange lights can you see?</h3>
      <div className={s.countRow}>
        <NumberField value={copies} min={1} max={ENTRY_MOST} label="Orange lights" onCommit={commit} />
        <p className={s.hint} data-bad={refusal ? "true" : undefined} aria-live="polite">
          {refusal ?? "Only one LED (one tiny light) is on. It can show on more than one fan or strip."}
        </p>
      </div>
      <RunDrawing length={1} />
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
