"use client";

import { CloudSlash } from "@phosphor-icons/react";
import { StatePage } from "@/components/StatePage";
import { RecoveryKeys } from "@/components/RecoveryKeys";

/**
 * The state the whole editor shows when it cannot reach canvas-rgb.
 *
 * The service relaunches a crashed daemon by itself (#576 R8), so the page says that in one line
 * and offers the recovery keys. It makes no claim about the lights' last frame: the editor cannot
 * read that state with the daemon down, so it does not guess.
 */
export function DaemonDown({
  onRetry,
  lastTryAt,
  nextTryInMs,
}: {
  onRetry: () => Promise<void>;
  lastTryAt?: number;
  nextTryInMs?: number;
}) {
  return (
    <StatePage
      icon={<CloudSlash size={26} />}
      title="canvas-rgb is not answering"
      lead="canvas-rgb draws the lights on 127.0.0.1:9430. It may still be starting, or it may have stopped."
      keeps={[
        "canvas-rgb restarts it by itself; if it keeps stopping, save diagnostics and report it.",
        "The fans stay under BIOS control.",
      ]}
      onRetry={onRetry}
      lastTryAt={lastTryAt}
      nextTryInMs={nextTryInMs}
      keys={
        <RecoveryKeys
          title="canvas-rgb is not answering"
          detail="The daemon did not answer. It may be stopping or restarting."
        />
      }
    />
  );
}
