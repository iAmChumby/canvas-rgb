"use client";

// The builder's draft on the lights (#257 phase 4): Try sends the whole stack to the daemon's
// draft engine and keeps it alive with a heartbeat; Stop restores the previous effect; a lost
// lease is watched down to zero. Moved out of StackBuilder.tsx unchanged.

import { useEffect, useRef, useState } from "react";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import type { StackSession } from "@/views/effects/stackDraft";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { Stack } from "@/proto/Stack";

export type DraftState = "off" | "starting" | "running" | "expiring" | "restoring" | "restore-error";

const HEARTBEAT_FALLBACK_MS = 5_000;
const LEASE_FALLBACK_MS = 15_000;

export function useDraftOnLights({
  count,
  session,
  mounted,
  stackForServer,
}: {
  count: number;
  session: { current: StackSession | null };
  mounted: { current: boolean };
  stackForServer: () => Stack;
}) {
  const [draftState, setDraftState] = useState<DraftState>("off");
  const [draftError, setDraftError] = useState<string | null>(null);
  const [draftNote, setDraftNote] = useState<string | null>(null);
  const [expiresIn, setExpiresIn] = useState(0);
  const draftOn = useRef(false);
  const heartbeatMs = useRef(HEARTBEAT_FALLBACK_MS);
  const leaseMs = useRef(LEASE_FALLBACK_MS);
  const lastBeatAt = useRef(0);
  const restoreStarted = useRef(false);

  function applyStatus(answer: DraftStatus) {
    heartbeatMs.current = answer.heartbeat_ms || HEARTBEAT_FALLBACK_MS;
    leaseMs.current = answer.lease_ms || LEASE_FALLBACK_MS;
    lastBeatAt.current = Date.now();
  }

  async function beat() {
    const current = session.current;
    if (!draftOn.current || !current) return;
    const { epoch, seq } = current.next();
    try {
      const answer = await api.putDraft({ html: "", stack: stackForServer(), epoch, seq, props: {} });
      if (!mounted.current || !draftOn.current) return;
      current.observe(answer);
      if (!answer.active) {
        draftOn.current = false;
        setDraftState("off");
        return;
      }
      applyStatus(answer);
      setDraftState("running");
      setDraftError(null);
    } catch (e) {
      if (!mounted.current || !draftOn.current) return;
      setDraftError(messageOf(e));
      setDraftState("expiring");
    }
  }

  // The 5 s beat while a draft runs (D95.4.5). The interval reads the answer's own timing and
  // the draft from refs, so a render never resets the beat.
  useEffect(() => {
    if (draftState !== "running" && draftState !== "expiring") return;
    const timer = setInterval(() => void beat(), heartbeatMs.current);
    return () => clearInterval(timer);
    // `beat` reads the draft, the session and the engine from refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftState]);

  const startDraft = async () => {
    if (count === 0) return;
    setDraftState("starting");
    setDraftError(null);
    setDraftNote(null);
    restoreStarted.current = false;
    const current = session.current;
    if (!current) return;
    const { epoch, seq } = current.next();
    try {
      const answer = await api.putDraft({ html: "", stack: stackForServer(), epoch, seq, props: {} });
      if (!mounted.current) return;
      current.observe(answer);
      if (!answer.active) {
        setDraftState("off");
        return;
      }
      draftOn.current = true;
      applyStatus(answer);
      setDraftState("running");
    } catch (e) {
      if (!mounted.current) return;
      setDraftState("off");
      setDraftError(messageOf(e));
    }
  };

  const stopDraft = async () => {
    draftOn.current = false;
    setDraftState("restoring");
    setDraftError(null);
    setDraftNote(null);
    const epoch = session.current?.epoch;
    try {
      const answer = await api.deleteDraft(epoch);
      if (!mounted.current) return;
      setDraftState("off");
      if (answer.applied === false) {
        // Another page owns the lights now: this Stop ended nothing, and saying it did would be
        // a lie (D95.4.4, item 4).
        setDraftNote("Another window has the lights now; this window's Stop ended nothing.");
      } else if (answer.note) setDraftNote(answer.note);
    } catch (e) {
      if (!mounted.current) return;
      setDraftState("restore-error");
      setDraftError(messageOf(e));
    }
  };

  /** A save ends the draft without restoring the old effect: the activation replaces it. */
  const endDraft = () => {
    draftOn.current = false;
    setDraftState("off");
  };

  // Leaving the builder stops the draft: nothing is left on the lights without a page to stop it.
  // The page's epoch goes with it, so a builder that lost the lights leaves the newer page's
  // draft alone.
  useEffect(
    () => () => {
      if (draftOn.current) {
        draftOn.current = false;
        void api.deleteDraft(session.current?.epoch).catch(() => undefined);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  // The lease countdown, as #2's editor counts it (D95.5.8).
  useEffect(() => {
    if (draftState !== "expiring") return;
    const tick = () => {
      const left = Math.max(0, lastBeatAt.current + leaseMs.current - Date.now());
      setExpiresIn(Math.ceil(left / 1000));
      if (left > 0 || restoreStarted.current) return;
      restoreStarted.current = true;
      setDraftState("restoring");
      let tries = 0;
      const ask = async () => {
        tries += 1;
        try {
          const answer = await api.draft();
          if (!mounted.current) return;
          if (!answer.active) {
            draftOn.current = false;
            setDraftState("off");
            if (answer.note) setDraftNote(answer.note);
            return;
          }
        } catch {
          // A read that fails is not an ending: the next one may say it is gone.
        }
        if (tries >= 5) {
          setDraftState("restore-error");
          setDraftError("the draft is still on the lights; its ending could not be seen");
          restoreStarted.current = false;
          return;
        }
        setTimeout(() => void ask(), 1_000);
      };
      void ask();
    };
    tick();
    const timer = setInterval(tick, 1_000);
    return () => clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftState]);

  return { draftState, draftError, draftNote, expiresIn, startDraft, stopDraft, endDraft };
}
