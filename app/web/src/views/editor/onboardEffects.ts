"use client";

import { useCallback, useEffect, useRef } from "react";
import { api } from "@/lib/endpoints";
import { refreshLayouts, refreshStatus } from "@/lib/store";
import type { Layout } from "@/proto/Layout";
import type { Props } from "@/proto/props";

/** The two built-in effects the flow plays on the real lights (hidden from the Effects page). */
export const HELLO_EFFECT = "onboard-hello";
export const SWEEP_EFFECT = "onboard-sweep";

/**
 * The effects the onboarding plays. The first one it plays remembers what was running, so
 * leaving the flow (Set up later, or Done without picking one) puts the owner's own effect
 * back. Every play is the daemon's ordinary activate route, so the live preview mirrors it on
 * the drawing with no code of its own.
 */
export function useOnboardEffects(current: Layout["effect"] | undefined) {
  /** `undefined`: nothing remembered yet. `null`: nothing was running. */
  const before = useRef<{ id: string; props: Props } | null | undefined>(undefined);
  const currentRef = useRef(current);
  useEffect(() => {
    currentRef.current = current;
  });

  const play = useCallback(async (id: string, props: Props = {}) => {
    if (before.current === undefined) {
      const cur = currentRef.current;
      before.current = cur ? { id: cur.id, props: cur.props } : null;
    }
    try {
      await api.activateEffect(id, { props, preset: null });
      await Promise.all([refreshLayouts(), refreshStatus()]);
    } catch {
      // The lights are the daemon's: a refused effect leaves the drawing as it was, and the
      // flow goes on without it.
    }
  }, []);

  /** Puts the owner's own effect back. A no-op when the flow never played one. */
  const restore = useCallback(async () => {
    const was = before.current;
    before.current = undefined;
    if (!was) return;
    try {
      await api.activateEffect(was.id, { props: was.props, preset: null });
      await Promise.all([refreshLayouts(), refreshStatus()]);
    } catch {
      // Left on whatever is running.
    }
  }, []);

  /** The owner picked an effect of their own: nothing to put back. */
  const keep = useCallback(() => {
    before.current = undefined;
  }, []);

  return { play, restore, keep };
}
