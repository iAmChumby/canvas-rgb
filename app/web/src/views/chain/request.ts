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

import { useSyncExternalStore } from "react";

import { offerChainsAgain } from "@/views/editor/flow";

let visits = 0;
let wanted: string | null = null;
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
  wanted = target ?? null;
  visits += 1;
  tell();
}

/** How many times the takeover has been asked to open. A new number is a new visit. */
export function chainSetupVisit(): number {
  return visits;
}

/** The header the last request named, if it named one. */
export function wantedChain(): string | null {
  return wanted;
}

/** The takeover's view of the request. */
export function useChainSetupVisit(): number {
  return useSyncExternalStore(subscribe, chainSetupVisit, () => 0);
}
