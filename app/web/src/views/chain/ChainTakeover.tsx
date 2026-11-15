"use client";

// The chain takeover: the first run's answer to "what is plugged into a header", offered by itself.
//
// It opens when the hello is behind the owner and a header has never been set up, and it offers
// one header at a time. Finish a header and the next one follows; mark one empty and it is never
// offered again; when every header is set up or empty, nothing opens. "Later" is remembered for
// the rest of the launch (the panel on every page is the way back in), and Escape leaves the header
// the flow is on. Leaving a header unfinished is the same as Later: the owner said "not now".
//
// What is on screen is worked out from the daemon's list on every render: the header being
// answered, else the first one never set up that this visit has not shown. The header being
// answered is pinned once, so a poll or a commit cannot swap it out from under the flow. A request
// from the panel is a new visit: the whole offer starts over.
//
// It draws the flow (`ChainSetup`) over Scenes and takes the window while it is open, the way the
// hello does, so the rail, the strip and the banners stay out of the way.

import { useCallback, useEffect, useState } from "react";

import { getSnap, refreshChains, useSnap } from "@/lib/store";
import type { ChainView } from "@/proto/ChainView";
import { ChainSetup } from "@/views/chain/ChainSetup";
import { useChainSetupVisit } from "@/views/chain/request";
import s from "@/views/chain/wiring.module.css";
import { chainLaterThisLaunch, putOffChainsUntilNextLaunch } from "@/views/editor/flow";
import { setTakeover } from "@/views/editor/takeover";

/** One header's identity in the daemon's list. */
function idOf(chain: ChainView): string {
  return `${chain.device_id}#z${chain.zone_index}`;
}

/** One visit: the headers this visit has shown, and the one on screen. */
function ChainVisit() {
  const status = useSnap((state) => state.status);
  const chains = useSnap((state) => state.chains);
  /** The headers this visit has answered or left, by identity. */
  const [shownBefore, setShownBefore] = useState<readonly string[]>([]);
  /** The header the flow is on, pinned so nothing the daemon says can swap it out. */
  const [pinned, setPinned] = useState<string | null>(null);

  // The list is what the takeover offers, so it is read when the takeover appears.
  useEffect(() => {
    void refreshChains();
  }, []);

  const offerable =
    status?.onboarding_complete && !chainLaterThisLaunch() && chains
      ? chains.filter((chain) => chain.state === "not_set_up" && !shownBefore.includes(idOf(chain)))
      : [];
  const next = offerable[0] ?? null;
  const shown = pinned !== null ? (chains?.find((chain) => idOf(chain) === pinned) ?? null) : next;

  // The first header offered becomes the header on screen, so the flow keeps its place while the
  // list is read again. This is React's own "adjust the state during a render" case: the render
  // that follows draws the pinned header at once, before anything is painted.
  if (pinned === null && next !== null) {
    setPinned(idOf(next));
  }

  // The takeover owns the window while a header is on screen.
  useEffect(() => {
    setTakeover(shown !== null);
  }, [shown]);
  useEffect(() => () => setTakeover(false), []);

  /** Closes the header on screen and asks for the next one, unless the owner is leaving. */
  const close = useCallback(() => {
    const at = shown;
    if (!at) return;
    setPinned(null);
    setShownBefore((seen) => [...seen, idOf(at)]);
    const now = getSnap().chains?.find((chain) => idOf(chain) === idOf(at))?.state;
    // Still not set up: Later, Back from the first step, or Escape. All three mean "not now",
    // so the header setup stays shut for the rest of the launch.
    if (now === "not_set_up") putOffChainsUntilNextLaunch();
    else void refreshChains();
  }, [shown]);

  // Escape leaves the flow: the header on screen goes away, which unmounts `ChainSetup` and so
  // ends its session and puts the length back. A name field stops Escape for itself.
  useEffect(() => {
    if (shown === null) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [close, shown]);

  if (shown === null) return null;

  return (
    <div className={s.takeover}>
      <ChainSetup key={idOf(shown)} chain={shown} onClose={close} />
    </div>
  );
}

export function ChainTakeover() {
  // A request from the panel starts a new visit: the state above goes and the offer starts over.
  const visit = useChainSetupVisit();
  return <ChainVisit key={visit} />;
}
