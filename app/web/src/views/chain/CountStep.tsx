"use client";

// The devices step, for a header whose devices run in series: how many are plugged in.
//
// The entry holds 1 or more, and a number is only taken when the header can hold that many
// devices; a count that cannot is refused with the reason on its own line, and the rows stay as
// they were. The line beside the entry says what the count makes, generated from the numbers.
//
// The miscount guard catches #382's mistake: an owner who counted one orange light on a header
// whose devices mirror each other arrives here and types the number of devices over the number of
// LEDs, which makes devices of one LED each. When the devices equal the LEDs and are more than 2,
// the screen says that is unusual and offers the way back to counting the orange lights. Saving
// stays possible.

import { useState } from "react";

import { AnimatedNumber } from "@/components/AnimatedNumber";
import { Button } from "@/components/Button";
import { NumberField } from "@/components/NumberField";
import type { ChainPartRow } from "@/views/chain/useChainSetup";
import { PartChip } from "@/views/chain/PartRow";
import s from "@/views/chain/chain.module.css";

/**
 * The most LEDs one controller can report (`MAX_LEDS_PER_CONTROLLER`, a wire limit). The entry
 * needs a ceiling of its own so a count past the header reaches this step's check, which can say
 * why, instead of being silently clamped by the entry.
 */
const WIRE_MOST_LEDS = 65535;

const WORDS = [
  "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
  "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

/** A count in words from zero to ninety-nine; a larger one stays in digits. */
export function numberWord(n: number): string {
  if (!Number.isInteger(n) || n < 0 || n > 99) return String(n);
  if (n < 20) return WORDS[n] ?? String(n);
  const ones = n % 10;
  return `${TENS[Math.floor(n / 10)] ?? ""}${ones === 0 ? "" : `-${WORDS[ones] ?? ""}`}`;
}

/** The line that says what a count makes: even, or uneven with who gets the extra LED. */
export function splitLine(length: number, count: number): string {
  const q = Math.floor(length / count);
  const r = length % count;
  const devices = count === 1 ? "device" : "devices";
  if (r === 0) return `${count} ${devices}, ${length} LEDs: ${q} each.`;
  return `${length} LEDs do not split evenly. The first ${r} ${r === 1 ? "device gets" : "devices get"} ${q + 1}, the rest get ${q}.`;
}

export function CountStep({
  zone,
  length,
  count,
  parts,
  onCount,
  onCountAgain,
}: {
  /** The header's name as the app shows it. */
  zone: string;
  /** The header's length, as the slide left it. */
  length: number;
  count: number;
  parts: ChainPartRow[];
  onCount: (n: number) => void;
  onCountAgain: () => void;
}) {
  const [refusal, setRefusal] = useState<string | null>(null);

  const commit = (n: number) => {
    if (n > length) {
      setRefusal(`${length} LEDs cannot be split into more than ${length} devices.`);
      return;
    }
    setRefusal(null);
    onCount(n);
  };

  const unusual = count === length && count > 2;
  const said = numberWord(count);

  return (
    <div className={s.stepWrap}>
      <h3 className={s.prompt}>How many devices are plugged into {zone}?</h3>
      <div className={s.countRow}>
        <NumberField
          value={count}
          min={1}
          max={WIRE_MOST_LEDS}
          label={`Devices on ${zone}`}
          onCommit={commit}
        />
        <p className={s.hint} data-bad={refusal ? "true" : undefined} aria-live="polite">
          {refusal ?? splitLine(length, count)}
        </p>
      </div>
      <p className={s.second}>Count each device, not each LED: 7 fans and 1 strip is 8.</p>
      {unusual ? (
        <div className={s.panel}>
          <p className={s.panelText}>
            {said.charAt(0).toUpperCase() + said.slice(1)} 1-LED devices is unusual. If every device shows the
            same colours, one per LED, they share one signal. Count again, and count the orange lights on
            every device together.
          </p>
          <div className={s.aside}>
            <Button size="sm" onClick={onCountAgain}>
              Count again
            </Button>
          </div>
        </div>
      ) : null}
      <ul className={s.list} aria-label={`Devices on ${zone}`}>
        {parts.map((part, i) => (
          <li key={i} className={s.line}>
            <PartChip index={i} />
            <span className={s.lineName}>Device {i + 1}</span>
            <span className={s.lineLeds}>
              <span className={s.sep} aria-hidden="true">
                ·
              </span>
              <AnimatedNumber value={part.leds} />
              <span className={s.unit}>{part.leds === 1 ? "LED" : "LEDs"}</span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
