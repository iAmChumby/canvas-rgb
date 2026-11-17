"use client";

import { CloudSlash } from "@phosphor-icons/react";
import { StatePage } from "@/components/StatePage";

/**
 * The state the whole editor shows when it cannot reach canvas-rgb.
 *
 * It says what is wrong in one line, what keeps working in two, and offers a retry that
 * says it is retrying.
 */
export function DaemonDown({
  onRetry,
  lastTryAt,
  nextTryInMs,
}: {
  onRetry: () => Promise<void>;
  /** When the daemon was last tried, 0 when it has not been tried yet. */
  lastTryAt?: number;
  /** How long until the next try, in milliseconds. */
  nextTryInMs?: number;
}) {
  return (
    <StatePage
      icon={<CloudSlash size={26} />}
      title="canvas-rgb is not answering"
      lead="canvas-rgb draws the lights on 127.0.0.1:9430. It may still be starting, or it may have stopped."
      keeps={["The lights keep the frame they had.", "The fans stay under BIOS control."]}
      onRetry={onRetry}
      lastTryAt={lastTryAt}
      nextTryInMs={nextTryInMs}
    />
  );
}
