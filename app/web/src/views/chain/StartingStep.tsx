"use client";

// The screen between a press of Start and the first question. The daemon raises the header to its
// longest length first, which can take a moment; the question is not drawn until it has answered,
// so the first thing the owner is asked about is already on the hardware. Text only: a decorative
// loop is not part of this app's look.

import s from "@/views/chain/chain.module.css";

export function StartingStep({ zone, intent }: { zone: string; intent: "setup" | "empty" }) {
  return (
    <div className={s.stepWrap} role="status">
      {intent === "empty" ? (
        <h3 className={s.prompt}>Marking {zone} empty.</h3>
      ) : (
        <>
          <h3 className={s.prompt}>Getting {zone} ready.</h3>
          <p className={s.body}>The LEDs on it are about to turn on.</p>
        </>
      )}
    </div>
  );
}
