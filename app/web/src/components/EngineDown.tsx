"use client";

import { Pulse } from "@phosphor-icons/react";
import { StatePage } from "@/components/StatePage";

/**
 * The state the whole editor shows when canvas-rgb is up and its render engine is not
 * answering (#241).
 *
 * It is not the daemon-down page: the daemon is running and answering requests, so
 * starting it again would not help, and the daemon's own words are shown as they are.
 */
export function EngineDown({
  onRetry,
  lastTryAt,
  nextTryInMs,
  message,
}: {
  onRetry: () => Promise<void>;
  /** When the daemon was last tried, 0 when it has not been tried yet. */
  lastTryAt?: number;
  /** How long until the next try, in milliseconds. */
  nextTryInMs?: number;
  /** The daemon's own words: why it could not answer. */
  message?: string | null;
}) {
  return (
    <StatePage
      icon={<Pulse size={26} />}
      title="canvas-rgb is up, but its render engine is not answering"
      lead="canvas-rgb is running and answering requests. It says:"
      keeps={["The lights keep the frame they had.", "The fans stay under BIOS control."]}
      message={message}
      onRetry={onRetry}
      lastTryAt={lastTryAt}
      nextTryInMs={nextTryInMs}
    />
  );
}
