"use client";

// Step 2: how many things are plugged into the chain.
//
// The entry holds 1 or more, and a number is only taken when the chain can hold that many parts;
// a count that cannot is refused with the reason on its own line, and the rows stay as they were.
// The rows show the colour each part's light will be, so the next step is recognised at once.

import { useState } from "react";

import { AnimatedNumber } from "@/components/AnimatedNumber";
import { NumberField } from "@/components/NumberField";
import type { ChainPartRow } from "@/views/chain/useChainSetup";
import { PartChip } from "@/views/chain/PartRow";
import s from "@/views/chain/chain.module.css";

/**
 * The most LEDs one controller can report (`MAX_LEDS_PER_CONTROLLER`, a wire limit). The entry
 * needs a ceiling of its own so a count past the chain reaches this step's check, which can say
 * why, instead of being silently clamped by the entry.
 */
const WIRE_MOST_LEDS = 65535;

export function CountStep({
  length,
  count,
  parts,
  onCount,
}: {
  /** The chain's length, as step 1 left it. */
  length: number;
  count: number;
  parts: ChainPartRow[];
  onCount: (n: number) => void;
}) {
  const [refusal, setRefusal] = useState<string | null>(null);

  const commit = (n: number) => {
    if (n > length) {
      setRefusal(`A chain of ${length} LEDs cannot have more than ${length} parts.`);
      return;
    }
    setRefusal(null);
    onCount(n);
  };

  return (
    <div className={s.stepWrap}>
      <h3 className={s.prompt}>How many things are plugged into this chain?</h3>
      <div className={s.countRow}>
        <NumberField
          value={count}
          min={1}
          max={WIRE_MOST_LEDS}
          label="Parts on this chain"
          onCommit={commit}
        />
        <p className={s.hint} data-bad={refusal ? "true" : undefined} aria-live="polite">
          {refusal ?? `Between 1 and ${length} parts.`}
        </p>
      </div>
      <ul className={s.list} aria-label="Parts on this chain">
        {parts.map((part, i) => (
          <li key={i} className={s.line}>
            <PartChip index={i} />
            <span className={s.lineName}>Part {i + 1}</span>
            <span className={s.lineLeds}>
              <AnimatedNumber value={part.leds} />
              <span className={s.unit}>LEDs</span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
