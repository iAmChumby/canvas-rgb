"use client";

// A request, from somewhere else in the app, for the chain takeover to open.
//
// The panel that stands on every page ("Set up your headers") is a `StandingNotice` in the shell,
// and the takeover lives on Scenes, where the lights are visible behind it. The panel cannot mount
// it and cannot rely on navigation alone: an owner already on Scenes has the takeover mounted and
// shut after "Later". So the panel leaves a request here, the takeover subscribes to it, and every
// request is a new visit: the takeover's state is thrown away and the headers are offered again.
//
// A visit is a number rather than a flag: the takeover keys its own content by it, so nothing has
// to be reset inside an effect.
//
// A request may name the header it is for (the notice for a new light offers that header's
// setup). A request without a name asks for the headers in general, and the takeover offers them
// one after another.
//
// The request is spent by the visit that serves it (#380). A flag that lived here would outlive
// that visit, and the setup would open by itself on every later mount of the takeover: the visit
// reads the request while it renders and clears it when it is over, so a later visit is a visit
// of its own and is asked for nothing.

import { useSyncExternalStore } from "react";

import { offerChainsAgain } from "@/views/editor/flow";

/** A request for the takeover to open: the visit it was made for, and the header it names. */
export interface ChainSetupRequest {
  /** The visit the request was made for. */
  visit: number;
  /** The header the request names, if it names one. */
  target: string | null;
}

let visits = 0;
let pending: ChainSetupRequest | null = null;
const listeners = new Set<() => void>();

function tell(): void {
  for (const l of listeners) l();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The panel's "Set up": forget this launch's Later, and ask the takeover to offer the headers. */
export function requestChainSetup(target?: string): void {
  offerChainsAgain();
  visits += 1;
  pending = { visit: visits, target: target ?? null };
  tell();
}

/** How many times the takeover has been asked to open. A new number is a new visit. */
export function chainSetupVisit(): number {
  return visits;
}

/**
 * The request this visit serves, if it is the one the request was made for.
 *
 * It is read, not taken: a render that happens twice (React's strict mode) must see the same
 * request both times, and the request is only spent when the visit that served it is over.
 */
export function chainSetupRequest(visit: number): ChainSetupRequest | null {
  return pending !== null && pending.visit === visit ? pending : null;
}

/** The visit that served the request is over: the request is not served again. */
export function consumeChainSetupRequest(visit: number): void {
  if (pending !== null && pending.visit === visit) pending = null;
}

/** The takeover's view of the request. */
export function useChainSetupVisit(): number {
  return useSyncExternalStore(subscribe, chainSetupVisit, () => 0);
}
