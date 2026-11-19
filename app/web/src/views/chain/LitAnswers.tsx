"use client";

// The two answers to "how many orange lights?", drawn. Both pictures show the same four devices of
// four LEDs; only the lit LEDs differ. In a row, the app lights LED 1 of the whole run, so one
// orange light shows, on the first device. On a shared signal every device repeats LED 1, so every
// device shows its own orange light. Reading the pictures side by side is the question's answer,
// so nobody has to know the words "series" or "mirror".
//
// The orange is the LED's own colour (the first of `PART_COLOURS`), the one the daemon is told to
// light; every other colour is a token.

import type { CSSProperties } from "react";

import { PART_COLOURS } from "@/views/chain/partColours";
import m from "@/views/chain/lit.module.css";

/** Devices drawn, and the LEDs each one shows. */
export const DRAWN_DEVICES = 4;
export const DRAWN_LEDS = 4;

/** `row`: one orange LED on the first device. `shared`: LED 1 of every device is orange. */
export type LitWay = "row" | "shared";

function Devices({ way }: { way: LitWay }) {
  return (
    <div className={m.devices} data-testid={`lit-drawing-${way}`} data-way={way} aria-hidden="true">
      {Array.from({ length: DRAWN_DEVICES }, (_, d) => (
        <span key={d} className={m.device} data-device="true">
          {Array.from({ length: DRAWN_LEDS }, (_, i) => (
            <span
              key={i}
              className={m.led}
              data-lit={i === 0 && (way === "shared" || d === 0) ? "true" : "false"}
            />
          ))}
        </span>
      ))}
    </div>
  );
}

export function LitAnswers() {
  const style = { "--orange": PART_COLOURS[0] ?? "#ff4d00" } as CSSProperties;
  return (
    <div className={m.answers} style={style}>
      <figure className={m.answer}>
        <Devices way="row" />
        <figcaption className={m.caption}>
          <span className={m.count}>1</span> orange light in all: a row
        </figcaption>
      </figure>
      <figure className={m.answer}>
        <Devices way="shared" />
        <figcaption className={m.caption}>
          <span className={m.count}>1</span> on every device: one shared signal
        </figcaption>
      </figure>
    </div>
  );
}
