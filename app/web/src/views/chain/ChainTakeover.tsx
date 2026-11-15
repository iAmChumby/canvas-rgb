"use client";

// The chain takeover: the first run's answer to "what is plugged into a header", offered by itself.
//
// It opens when the hello is behind the owner and a header has never been set up, and it offers
// one header at a time. Finish a header and the next one follows; mark one empty and it is never
// offered again; when every header is set up or empty, nothing opens.
//
// What the owner says about a header is remembered per header, and it survives the window
// (#380): the record is the header's own, in this viewer's storage, so a header is offered by
// itself at most once, ever. A window opened from the tray (a new webview, a new launch) finds
// the header already answered and stays shut.
//
// A header that already has a size is never offered by itself: the flow's first step is finding
// how long the header is, and a header the app has already sized does not need that. Those
// headers are reached from the device row, from the notice for a new light, or from the panel
// that stands on every page.
//
// Escape leaves the header the flow is on. Leaving a header unfinished is the same as Later: the
// owner said "not now".
//
// What is on screen is worked out from the daemon's list on every render: the header being
// answered, else the first one never set up that this visit has not shown. The header being
// answered is pinned once, so a poll or a commit cannot swap it out from under the flow. A request
// from the panel is a new visit: the whole offer starts over.

import { useCallback, useEffect, useState } from "react";

import { getSnap, refreshChains, useSnap } from "@/lib/store";
import type { ChainView } from "@/proto/ChainView";
import { ChainSetup } from "@/views/chain/ChainSetup";
import { chainKey, markChainOffered, offeredChains } from "@/views/chain/offered";
import { useChainSetupVisit, wantedChain } from "@/views/chain/request";
import s from "@/views/chain/wiring.module.css";
import { chainLaterThisLaunch, putOffChainsUntilNextLaunch } from "@/views/editor/flow";
import { setTakeover } from "@/views/editor/takeover";

/** One header's identity in the daemon's list. */
function idOf(chain: ChainView): string {
  return chainKey(chain.device_id, chain.zone_index);
}

/** One visit: the headers this visit has shown, and the one on screen. */
function ChainVisit({ asked, wanted }: { asked: boolean; wanted: string | null }) {
  const status = useSnap((state) => state.status);
  const chains = useSnap((state) => state.chains);
  /** The headers this visit has answered or left, by identity. */
  const [shownBefore, setShownBefore] = useState<readonly string[]>([]);
  /** The header the flow is on, pinned so nothing the daemon says can swap it out. */
  const [pinned, setPinned] = useState<string | null>(wanted);

  // The list is what the takeover offers, so it is read when the takeover appears.
  useEffect(() => {
    void refreshChains();
  }, []);

  // The headers the app has already offered by itself, or the owner has put off.
  const offered = offeredChains();

  const offerable =
    status?.onboarding_complete && !chainLaterThisLaunch() && chains
      ? chains.filter((chain) => {
          if (chain.state !== "not_set_up") return false;
          if (shownBefore.includes(idOf(chain))) return false;
          // The header the owner named: that one, and nothing else.
          if (wanted !== null) return idOf(chain) === wanted;
          // The owner asked for the headers: every one that is not set up.
          if (asked) return true;
          // The app offers a header by itself: only one it has not sized (the flow's first step
          // is finding the length) and only one it has never offered.
          return chain.leds === 0 && !offered.has(idOf(chain));
        })
      : [];
  const next = offerable[0] ?? null;
  const shown = pinned !== null ? (chains?.find((chain) => idOf(chain) === pinned) ?? null) : next;

  // The first header offered becomes the header on screen, so the flow keeps its place while the
  // list is read again. This is React's own "adjust the state during a render" case: the render
  // that follows draws the pinned header at once, before anything is painted.
  if (pinned === null && next !== null) {
    setPinned(idOf(next));
  }

  // The header on screen is remembered: the app offers a header by itself at most once, ever, and
  // "Later", the close key and Escape all mean the same thing to that record.
  useEffect(() => {
    if (shown === null) return;
    markChainOffered(idOf(shown));
  }, [shown]);

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
    // The header is remembered whatever closed it: the app does not offer it by itself again.
    markChainOffered(idOf(at));
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
  return <ChainVisit key={visit} asked={visit > 0} wanted={wantedChain()} />;
}
