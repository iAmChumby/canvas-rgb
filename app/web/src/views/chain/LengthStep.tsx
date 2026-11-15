"use client";

// Step 1: drag until the coloured light is the last one on the chain.
//
// The figure counts as the fader moves. The line under it is always there, so the screen keeps its
// height when the header's ceiling is reached; then it names the ceiling and why the chain stops.

import { Button } from "@/components/Button";
import { LengthSlider } from "@/views/chain/LengthSlider";
import s from "@/views/chain/chain.module.css";

export function LengthStep({
  value,
  max,
  capped,
  onChange,
  onNothing,
}: {
  value: number;
  /** The header's own longest length (`max_leds`). */
  max: number;
  /** The length has reached the header's maximum and the chain goes on past it. */
  capped: boolean;
  onChange: (n: number) => void;
  onNothing: () => void;
}) {
  return (
    <div className={s.stepWrap}>
      <h3 className={s.prompt}>Drag until the coloured light is the last one on the chain.</h3>
      <LengthSlider value={value} max={max} onChange={onChange} onCommit={onChange} />
      <p className={s.note} aria-live="polite">
        {capped ? (
          <>
            <span className={s.noteDot} aria-hidden="true" />
            This header takes at most {max} LEDs.
          </>
        ) : null}
      </p>
      <div className={s.aside}>
        <Button variant="ghost" size="sm" onClick={onNothing}>
          Nothing lit up
        </Button>
      </div>
    </div>
  );
}
