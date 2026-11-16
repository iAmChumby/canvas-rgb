"use client";

// The slide step: drag until the orange light is on the last LED of the last device.
//
// The hardware shows a white trail and an orange end; the length is the last LED where the orange
// still shows. The figure counts as the fader moves. The line under it is always there, so the
// screen keeps its height when the header's ceiling is reached; then it names the ceiling and why
// the header stops.
//
// The two exits are for the headers where the rule does not hold. "Some went dark" is for devices
// of different lengths on a header that mirrors (the shortest one runs out first); "It never goes
// out" is for a hub that wraps or pads past the last LED, so the orange never leaves: the owner
// says how many LEDs one device has. Both are a mirror's exits: on a series a device at the last
// LED ends the run, so neither is offered. Both open in the slot the keys are in, so nothing moves.

import { useState } from "react";

import { Button } from "@/components/Button";
import { NumberField } from "@/components/NumberField";
import { LengthSlider } from "@/views/chain/LengthSlider";
import { RunDrawing } from "@/views/chain/RunDrawing";
import s from "@/views/chain/chain.module.css";

type Panel = "dark" | "never" | null;

export function LengthStep({
  value,
  max,
  capped,
  copies,
  onChange,
}: {
  value: number;
  /** The header's own longest length (`max_leds`). */
  max: number;
  /** The length has reached the header's maximum and the header goes on past it. */
  capped: boolean;
  /** The orange lights the owner counted. */
  copies: number;
  onChange: (n: number) => void;
}) {
  const [panel, setPanel] = useState<Panel>(null);
  /** The LEDs on one device, as typed in the second panel. */
  const [perDevice, setPerDevice] = useState(Math.max(1, value));

  const close = () => setPanel(null);

  return (
    <div className={s.stepWrap}>
      <h3 className={s.prompt}>
        {copies > 1
          ? "Slide until the orange light is on the last LED of each device."
          : "Slide until the orange light is on the last LED of the last device along the cable."}
      </h3>
      <LengthSlider value={value} max={max} onChange={onChange} onCommit={onChange} />
      <RunDrawing length={value} />
      <p className={s.note} aria-live="polite">
        {capped ? (
          <>
            <span className={s.noteDot} aria-hidden="true" />
            This header takes at most {max} LEDs.
          </>
        ) : null}
      </p>
      <div className={s.slot}>
        {panel === null && copies > 1 ? (
          <div className={`${s.aside} ${s.escapes}`}>
            {value >= 2 ? (
              <Button variant="ghost" size="sm" onClick={() => setPanel("dark")}>
                Some went dark
              </Button>
            ) : null}
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setPerDevice(Math.max(1, value));
                setPanel("never");
              }}
            >
              It never goes out
            </Button>
          </div>
        ) : null}
        {panel === "dark" ? (
          <div className={s.panel}>
            <p className={s.panelText}>Some devices have gone dark. Others still show orange.</p>
            <div className={s.aside}>
              <Button
                size="sm"
                variant="primary"
                onClick={() => {
                  onChange(value - 1);
                  close();
                }}
              >
                Use {value - 1} {value - 1 === 1 ? "LED" : "LEDs"}
              </Button>
              <Button size="sm" variant="ghost" onClick={close}>
                Keep sliding
              </Button>
            </div>
          </div>
        ) : null}
        {panel === "never" ? (
          <div className={s.panel}>
            <p className={s.panelText}>
              Some hubs repeat the lights past the last LED, so the orange never goes out. How many LEDs does
              one device have?
            </p>
            <div className={s.aside}>
              <NumberField
                value={perDevice}
                min={1}
                max={max}
                label="LEDs on one device"
                onCommit={setPerDevice}
              />
              <Button
                size="sm"
                variant="primary"
                onClick={() => {
                  onChange(perDevice);
                  close();
                }}
              >
                Use {perDevice} {perDevice === 1 ? "LED" : "LEDs"}
              </Button>
              <Button size="sm" variant="ghost" onClick={close}>
                Keep sliding
              </Button>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
