"use client";

// The check for a header whose devices mirror each other (#382). There is no yes/no question: the
// count and the length were both answered by sight, so this screen shows what will be saved and
// what the app cannot do with it. Every LED on the header is lit white, and the screen says so,
// so the owner can see the whole run in one look before saving.
//
// One row: the name (the app's own, "Same lights, shown 8 times", until the owner types one) and
// the LEDs, read only. The kind is not asked: the app cannot know what the devices are, so it
// stores them as other and never calls them fans.

import type { CSSProperties } from "react";

import { NameField } from "@/components/NameField";
import { TRAIL_COLOUR } from "@/views/chain/useChainSetup";
import s from "@/views/chain/chain.module.css";
import m from "@/views/chain/mirror.module.css";

export function MirrorCheck({
  zone,
  length,
  copies,
  name,
  onName,
}: {
  zone: string;
  /** The LEDs of the one run. */
  length: number;
  /** How many times the header shows it. */
  copies: number;
  name: string;
  onName: (name: string) => void;
}) {
  const endEditing = () => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  };
  return (
    <div className={s.stepWrap}>
      <h3 className={s.prompt}>
        The same {length} {length === 1 ? "LED shows" : "LEDs show"} {copies} times on {zone}.
      </h3>
      <p className={s.body}>You can colour them together. You cannot set one on its own.</p>
      <p className={m.lit} style={{ "--white": TRAIL_COLOUR } as CSSProperties}>
        <span className={m.litDot} aria-hidden="true" />
        {length === 1 ? "The LED is lit white now." : `All ${length} LEDs are lit white now.`}
      </p>
      <div className={m.row}>
        <span className={m.name}>
          <NameField
            value={name}
            label="Name of the light"
            autoFocus={false}
            onChange={onName}
            onEnter={endEditing}
            onEscape={endEditing}
          />
        </span>
        <span className={m.leds}>
          {length} {length === 1 ? "LED" : "LEDs"}
        </span>
      </div>
    </div>
  );
}
