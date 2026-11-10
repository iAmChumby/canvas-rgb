"use client";

// The builder's preview (#257 phase 4): every change is PUT to the daemon's stack preview, after a
// trailing debounce and never later than one debounce after the first of a run (D95.4.4); the feed
// says when a second builder owns the socket. Moved out of StackBuilder.tsx unchanged.

import { useEffect, useRef, useState } from "react";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { onStackPreviewFeed } from "@/lib/stackPreview";
import type { StackSession } from "@/views/effects/stackDraft";
import type { LoadState } from "@/components/Loading";
import type { Stack } from "@/proto/Stack";
import type { StackLayerStatus } from "@/proto/StackLayerStatus";

/** Sent this long after the last change, and no later than this after the first. */
const PREVIEW_DEBOUNCE_MS = 400;
/**
 * The longest the builder waits for the daemon's own "try again later" before it tries again
 * (issue #97). The daemon's contract is one second, and a render takes at most thirty: a cap
 * that keeps the retry inside one render cycle. Without it a wrong, an abusive or a future
 * `Retry-After` header would leave the preview on its last frame for an hour.
 */
const MAX_PREVIEW_RETRY_MS = 30_000;

export function usePreview({
  phase,
  draft,
  session,
  mounted,
  stackForServer,
  setStatuses,
}: {
  phase: LoadState;
  draft: Stack;
  session: { current: StackSession | null };
  mounted: { current: boolean };
  stackForServer: () => Stack;
  setStatuses: (next: Record<string, StackLayerStatus>) => void;
}) {
  const [previewError, setPreviewError] = useState<string | null>(null);
  /** What the preview feed says is wrong with it: the daemon refuses a second stack socket. */
  const [feedProblem, setFeedProblem] = useState<string | null>(null);
  const previewTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** The max wait's own timer: a change never moves it, so a steady stream of changes cannot put
   *  a send off for ever. */
  const previewWait = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** When the last change arrived: a change that follows another within one debounce is part of
   *  the same drag, and only then does the max wait apply. */
  const previewChangedAt = useRef(0);
  const previewSentAt = useRef(0);
  const appliedSeq = useRef(0);
  const previewRetry = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastDraftKey = useRef("");

  async function sendPreview(retried = false) {
    // One retry waits at a time (issue #97): a send that starts now takes the place of the retry
    // that was waiting, instead of leaving it to fire as well. A change that sends again while a
    // retry waits would otherwise cost two requests for one busy answer.
    if (previewRetry.current) {
      clearTimeout(previewRetry.current);
      previewRetry.current = null;
    }
    previewSentAt.current = Date.now();
    const current = session.current;
    if (!current) return;
    const { epoch, seq } = current.next();
    try {
      const answer = await api.putStackPreview({ stack: stackForServer(), epoch, seq });
      if (!mounted.current) return;
      current.observe(answer);
      // A superseded PUT is never resent (D95.4.4): only a new change sends again.
      if (!answer.applied) return;
      if (seq > appliedSeq.current) {
        appliedSeq.current = seq;
        setStatuses(Object.fromEntries(answer.layers.map((layer) => [layer.id, layer])));
        setPreviewError(null);
      }
    } catch (e) {
      if (!mounted.current) return;
      // A busy daemon says when to ask again. Anything else is a transport failure or a server
      // error, and one more try 400 ms later is the difference between a preview that catches up
      // and one that stays on the last frame until the next change. The daemon's wait has a
      // ceiling, so a wrong header cannot strand the preview (issue #97).
      const wait =
        e instanceof ApiFailure
          ? e.status === 503
            ? Math.min(e.retryAfterMs ?? 1_000, MAX_PREVIEW_RETRY_MS)
            : null
          : PREVIEW_DEBOUNCE_MS;
      if (!retried && wait !== null) {
        previewRetry.current = setTimeout(() => void sendPreview(true), wait);
        return;
      }
      setPreviewError(messageOf(e));
    }
  }

  /** Sends now, and forgets both timers: a send is the end of the wait either way. */
  function flushPreview() {
    if (previewTimer.current) clearTimeout(previewTimer.current);
    if (previewWait.current) clearTimeout(previewWait.current);
    previewTimer.current = null;
    previewWait.current = null;
    void sendPreview();
  }

  function queuePreview() {
    const now = Date.now();
    // A change waits 400 ms after itself. A change that follows another within one debounce is
    // part of a drag, and the drag gets a wait of its own that a change does not move, due 400 ms
    // after the last send: a change every 300 or 390 ms then sends every 400 ms, not every 600
    // or 780 (D95.4.4). A change after a quiet spell is alone, and waits out the debounce.
    const inStream = previewChangedAt.current > 0 && now - previewChangedAt.current <= PREVIEW_DEBOUNCE_MS;
    previewChangedAt.current = now;
    if (previewTimer.current) clearTimeout(previewTimer.current);
    previewTimer.current = setTimeout(() => {
      previewTimer.current = null;
      flushPreview();
    }, PREVIEW_DEBOUNCE_MS);
    if (inStream && !previewWait.current) {
      const due = Math.max(0, previewSentAt.current + PREVIEW_DEBOUNCE_MS - now);
      previewWait.current = setTimeout(() => {
        previewWait.current = null;
        flushPreview();
      }, due);
    }
  }

  // The first draft on screen opens the preview at once; every later change waits out the
  // trailing debounce, and never longer than the debounce after the first of a run (D95.4.4).
  useEffect(() => {
    if (phase !== "ready") return;
    const key = JSON.stringify(draft);
    if (key === lastDraftKey.current) return;
    const first = lastDraftKey.current === "";
    lastDraftKey.current = key;
    if (first) void sendPreview();
    else queuePreview();
    // The two senders read the draft from refs and the session's own epoch; only a change to the
    // draft or the phase asks for a new PUT.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft, phase]);

  useEffect(
    () => () => {
      if (previewTimer.current) clearTimeout(previewTimer.current);
      if (previewWait.current) clearTimeout(previewWait.current);
      if (previewRetry.current) clearTimeout(previewRetry.current);
      void api.deleteStackPreview(session.current?.epoch).catch(() => undefined);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  // The daemon opens one stack preview at a time: a second builder's socket is refused with a
  // reason (`api/preview.rs`), and the page says so instead of showing a still picture.
  useEffect(() => onStackPreviewFeed(setFeedProblem), []);

  return { previewError, feedProblem };
}
