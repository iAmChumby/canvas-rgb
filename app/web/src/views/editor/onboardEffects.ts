"use client";

import { useCallback } from "react";
import { api } from "@/lib/endpoints";
import { refreshScenes, refreshStatus } from "@/lib/store";

/** The two built-in effects the flow plays on the real lights (hidden from the Effects page). */
export const HELLO_EFFECT = "onboard-hello";
export const SWEEP_EFFECT = "onboard-sweep";

/**
 * The effects the onboarding plays. The daemon plays one as a takeover: it holds the lights for
 * a moment and its timer puts the active scene back, so nothing here remembers what ran before
 * (#310: the scene is the only thing that can be "before").
 */
export function useOnboardEffects() {
  const play = useCallback(async (id: string) => {
    try {
      await api.activateEffect(id, { props: {}, preset: null });
      await Promise.all([refreshScenes(), refreshStatus()]);
    } catch {
      // The lights are the daemon's: a refused effect leaves them as they were, and the flow
      // goes on without it.
    }
  }, []);

  return { play };
}
