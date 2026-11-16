"use client";

// The first screen of a header's setup: what an ARGB header is, and the one honest way out for a
// header with nothing on it. Nothing on the hardware has changed yet; the session opens on Start.
//
// The first header of a visit gets the full introduction. The headers after it get a short card,
// because the owner has read the introduction already: "Next: JARGB 2. Is anything plugged into
// it?". When the header before was marked empty, the card says so above itself, with the way back.

import { Button } from "@/components/Button";
import s from "@/views/chain/chain.module.css";

export function IntroStep({
  kind,
  zone,
  previousEmpty,
  onEmpty,
}: {
  kind: "full" | "next";
  /** The header's name as the app shows it. */
  zone: string;
  /** The header before this one, if the owner marked it empty a moment ago. */
  previousEmpty: string | null;
  /** "Nothing is plugged in here": marks the header empty. */
  onEmpty: () => void;
}) {
  return (
    <div className={s.stepWrap}>
      {kind === "next" && previousEmpty !== null ? (
        <p className={s.notice} role="status">
          {previousEmpty} is marked empty. Set it up again from Devices, then add its lights to your scenes.
        </p>
      ) : null}
      {kind === "full" ? (
        <>
          <h3 className={s.prompt}>Tell the app about your lights.</h3>
          <p className={s.body}>
            Your motherboard has ARGB headers: small plugs for colour-changing lights. Each fan or strip on
            one is a device. Your manual may call them connectors. This one is called {zone}. Look at your
            case and answer a few questions.
          </p>
        </>
      ) : (
        <>
          <h3 className={s.prompt}>Next: {zone}.</h3>
          <p className={s.body}>Is anything plugged into it?</p>
        </>
      )}
      <div className={s.emptyKey}>
        <Button variant="default" onClick={onEmpty}>
          Nothing is plugged in here
        </Button>
        <p className={s.note}>Anything plugged in here stays dark and leaves your scenes.</p>
      </div>
      <p className={s.caption}>
        Later leaves every light as it is. You can set up this header any time from Devices.
      </p>
    </div>
  );
}
