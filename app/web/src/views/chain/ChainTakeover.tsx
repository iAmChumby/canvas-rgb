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
// answered is pinned once, so a poll or a commit cannot swap it out from under the flow.
//
// A request from the panel is a new visit, and it is spent by the visit that serves it: the visit
// reads the request when it starts and clears it when it is over. A later visit is a visit of its
// own and is asked for nothing, so the setup does not open by itself on every later mount.

import { useCallback, useEffect, useState } from "react";

import { getSnap, refreshChains, useSnap } from "@/lib/store";
import type { ChainView } from "@/proto/ChainView";
import { ChainSetup } from "@/views/chain/ChainSetup";
import { chainKey, markChainOffered, offeredChains } from "@/views/chain/offered";
import { chainSetupRequest, consumeChainSetupRequest, useChainSetupVisit } from "@/views/chain/request";
import s from "@/views/chain/wiring.module.css";
import { chainLaterThisLaunch, putOffChainsUntilNextLaunch } from "@/views/editor/flow";
import { setTakeover } from "@/views/editor/takeover";

/** One header's identity in the daemon's list. */
function idOf(chain: ChainView): string {
  return chainKey(chain.device_id, chain.zone_index);
}

/**
 * The headers a visit may show: the ones it has not shown yet and that are not set up, and that
 * the owner asked for (or that the app may offer by itself).
 *
 * The header on screen is among them: it is not in `shownBefore` until the visit leaves it.
 */
function offerableFor(
  chains: ChainView[] | null,
  shownBefore: readonly string[],
  asked: boolean,
  wanted: string | null,
  offered: ReadonlySet<string>,
): ChainView[] {
  if (!chains) return [];
  return chains.filter((chain) => {
    if (chain.state !== "not_set_up") return false;
    if (shownBefore.includes(idOf(chain))) return false;
    // The header the owner named: that one, and nothing else.
    if (wanted !== null) return idOf(chain) === wanted;
    // The owner asked for the headers: every one that is not set up.
    if (asked) return true;
    // The app offers a header by itself: only one it has not sized (the flow's first step
    // is finding the length) and only one it has never offered.
    return chain.leds === 0 && !offered.has(idOf(chain));
  });
}

/** One visit: the headers this visit has shown, and the one on screen. */
function ChainVisit({
  visit,
  asked: askedAt,
  wanted: wantedAt,
}: {
  visit: number;
  asked: boolean;
  wanted: string | null;
}) {
  const status = useSnap((state) => state.status);
  const chains = useSnap((state) => state.chains);
  /** The headers this visit has answered or left, by identity. */
  const [shownBefore, setShownBefore] = useState<readonly string[]>([]);
  /** The header the flow is on, pinned so nothing the daemon says can swap it out. */
  const [pinned, setPinned] = useState<string | null>(wantedAt);

  // The request belongs to the visit that serves it, so it is read once, when the visit starts.
  // A later visit is a visit of its own: it is asked for nothing.
  const [asked] = useState(askedAt);
  const [wanted] = useState(wantedAt);

  // The list is what the takeover offers, so it is read when the takeover appears.
  useEffect(() => {
    void refreshChains();
  }, []);

  // The request is spent when this visit is over.
  useEffect(() => consumeChainSetupRequest(visit), [visit]);

  // The headers the app has already offered by itself, or the owner has put off.
  const offered = offeredChains();

  const offerable = offerableFor(chains, shownBefore, asked, wanted, offered);
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

  /**
   * Closes the header on screen and asks for the next one, unless the owner is leaving.
   *
   * The headers this visit has not shown yet are worked out before they are recorded, so the visit
   * can still move on to the next one after the close key.
   */
  const close = useCallback(() => {
    const at = shown;
    if (!at) return;
    const now = getSnap().chains?.find((chain) => idOf(chain) === idOf(at))?.state;
    const rest = offerableFor(getSnap().chains, shownBefore, asked, wanted, offered).filter(
      (chain) => idOf(chain) !== idOf(at),
    );
    // The header is remembered whatever closed it: the app does not offer it by itself again.
    markChainOffered(idOf(at));
    setShownBefore((seen) => [...seen, idOf(at)]);
    if (now === "not_set_up") {
      // Later, Back from the first step, or Escape: the owner said "not now", and the question is
      // the same for every header this visit has not shown yet. They are put off for good as well,
      // so none of them is offered by itself again, in this launch or a later one.
      for (const chain of rest) markChainOffered(idOf(chain));
      setPinned(null);
      // The header setup stays shut for the rest of the launch.
      putOffChainsUntilNextLaunch();
      return;
    }
    if (now === "set_up") {
      // The close key: the header is answered, and the headers this visit has not shown are put
      // off too, so the app does not offer them by itself later.
      for (const chain of rest) markChainOffered(idOf(chain));
    }
    // The visit moves on to the next header, which is pinned so that the record above cannot take
    // it away. A header the owner marked empty is not a put-off: the others still need asking.
    setPinned(rest[0] ? idOf(rest[0]) : null);
    void refreshChains();
  }, [asked, offered, shown, shownBefore, wanted]);

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
  // The request this visit serves. It is read while rendering, not taken: a render that happens
  // twice (React's strict mode) must see the same request both times.
  const request = chainSetupRequest(visit);
  return <ChainVisit key={visit} visit={visit} asked={request !== null} wanted={request?.target ?? null} />;
}
