"use client";

import { useEffect } from "react";
import { api } from "@/lib/endpoints";
import { runningOf, setShown, useRunningPreset } from "@/lib/runningPreset";

/**
 * Keeps the running preset known for the effect on the lights. The Effects view publishes it
 * while it is open (it sees every slider move); this asks the daemon whenever the running effect
 * changes, so the strip and the deck name the preset on every page.
 */
export function useTrackRunningPreset(activeId: string | null) {
  useEffect(() => {
    if (!activeId) return;
    let stop = false;
    api
      .effect(activeId)
      .then((detail) => {
        if (!stop) setShown(activeId, runningOf(detail));
      })
      .catch(() => {
        // The name is a nicety: a daemon that does not answer shows the effect without it.
      });
    return () => {
      stop = true;
    };
  }, [activeId]);
}

/** " · Storm", with an amber LED when the live values differ from the preset's. Nothing when none. */
export function RunningPresetName({ effectId }: { effectId: string | null }) {
  const running = useRunningPreset(effectId);
  if (!running) return null;
  return (
    <span className="running-preset" data-modified={running.modified ? "true" : "false"}>
      <span className="running-preset-sep" aria-hidden="true">
        {" · "}
      </span>
      <span className="running-preset-name">{running.name}</span>
      {running.modified && (
        <>
          <span className="running-preset-led" aria-hidden="true" />
          <span className="sr-only"> (modified)</span>
        </>
      )}
    </span>
  );
}
