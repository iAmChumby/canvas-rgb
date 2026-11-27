"use client";

import { Pulse } from "@phosphor-icons/react";
import { StatePage } from "@/components/StatePage";
import { RecoveryKeys } from "@/components/RecoveryKeys";

/**
 * The state the whole editor shows when canvas-rgb is up and its render engine is not
 * answering (#241). The daemon's message arrives scrubbed through the response layer (R2).
 */
export function EngineDown({
  onRetry,
  lastTryAt,
  nextTryInMs,
  message,
}: {
  onRetry: () => Promise<void>;
  lastTryAt?: number;
  nextTryInMs?: number;
  /** The daemon's own words: why it could not answer. */
  message?: string | null;
}) {
  return (
    <StatePage
      icon={<Pulse size={26} />}
      title="canvas-rgb is up, but its render engine is not answering"
      lead="canvas-rgb is running and answering requests. It says:"
      keeps={["The fans stay under BIOS control."]}
      message={message}
      onRetry={onRetry}
      lastTryAt={lastTryAt}
      nextTryInMs={nextTryInMs}
      keys={
        <RecoveryKeys
          title="canvas-rgb is up, but its render engine is not answering"
          detail={message ?? null}
        />
      }
    />
  );
}
