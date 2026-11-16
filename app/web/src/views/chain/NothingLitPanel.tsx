"use client";

// "Nothing lit up", and "I can't see inside": the two screens for an owner who cannot answer the
// first question.
//
// Nothing lit up lists what to check and writes nothing; Try again lights LED 1 again. Only the
// explicit key marks the header empty, and the caption above it says what that does: whatever is
// plugged in stays dark, and the header's lights leave every scene.

import { Button } from "@/components/Button";
import s from "@/views/chain/chain.module.css";

export function NothingLitPanel({
  zone,
  onBlind,
  onEmpty,
}: {
  zone: string;
  onBlind: () => void;
  onEmpty: () => void;
}) {
  return (
    <div className={s.stepWrap}>
      <h3 className={s.prompt}>No light came on.</h3>
      <p className={s.body}>Check these, then try again.</p>
      <ol className={s.checks}>
        <li>Is the cable in the ARGB header called {zone}?</li>
        <li>Is another lighting app running? Close it, such as SignalRGB.</li>
      </ol>
      <div className={s.emptyKey}>
        <p className={s.note}>Anything plugged in here stays dark and leaves your scenes.</p>
        <div className={s.aside}>
          <Button variant="ghost" size="sm" onClick={onBlind}>
            I can&rsquo;t see inside
          </Button>
          <Button variant="default" size="sm" onClick={onEmpty}>
            Mark this header empty
          </Button>
        </div>
      </div>
    </div>
  );
}

export function BlindStep() {
  return (
    <div className={s.stepWrap}>
      <h3 className={s.prompt}>
        Open the side panel or look from the back, then try again. Nothing was changed.
      </h3>
    </div>
  );
}
