"use client";

// The chain setup flow's data: one session on one header, lit through the identify path, and the
// parts the owner describes in three steps (length, count, confirm).
//
// The session's lifetime is this hook's whole job. It opens on mount and is ended exactly once:
// by a commit, by `nothingLitUp`, by Later, by Back from the length step, by the page going away
// (a `keepalive` fetch, so a closed window still puts the length back) or by unmount. The daemon
// ends a session of its own accord 6 s after the last light call, so a light call is sent at most
// every 60 ms while the slider moves and again every 2 s as the heartbeat, which keeps a session
// open across a think, a step change and a failed commit's retry.
//
// No resize happens here. The daemon raised the header when the session opened and resizes once
// more at the commit; the flow only says which LEDs to light and what the parts are.
//
// The bookkeeping lives in refs, because the timers read it between renders; everything a render
// shows is state. `live` is the one copy the timers read, written by `apply` in the same breath as
// the state, so a trailing light call can never carry a value older than the last keystroke.

import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { refreshChains, refreshLights, useSnap } from "@/lib/store";
import type { ChainPartKind } from "@/proto/ChainPartKind";
import type { ChainView } from "@/proto/ChainView";
import type { LightView } from "@/proto/LightView";
import type { SetupRun } from "@/proto/SetupRun";
import { defaultName, PART_COLOURS, renameForKind } from "@/views/chain/partColours";
import { evenSplit, moveBoundary, runsFor } from "@/views/chain/split";

/** The shortest time between two light calls (ms), so a drag cannot flood the daemon. */
export const LIGHT_THROTTLE_MS = 60;
/**
 * How often the session is kept alive (ms). The daemon ends a setup 6 s after the last light
 * call (`SETUP_TIMEOUT_MS`, `crates/canvas-proto/src/scene.rs`), so two seconds leaves room for a
 * slow answer.
 */
export const HEARTBEAT_MS = 2000;

export type ChainStep = "length" | "count" | "confirm" | "done" | "error";

/** One part's row in the confirm step. */
export interface ChainPartRow {
  name: string;
  kind: ChainPartKind;
  leds: number;
  /** True once the owner typed the name: a name that is the app's own follows a change of kind. */
  named: boolean;
}

export interface ChainSetup {
  step: ChainStep;
  length: number;
  /** Never a resize: the slider only lights LEDs through the session. */
  setLength(n: number): void;
  count: number;
  setCount(n: number): void;
  parts: ChainPartRow[];
  setPartLeds(i: number, n: number): void;
  setPartKind(i: number, k: ChainPartKind): void;
  setPartName(i: number, s: string): void;
  addPart(): void;
  removePart(i: number): void;
  back(): void;
  next(): void;
  confirm(): Promise<void>;
  nothingLitUp(): Promise<void>;
  later(): void;
  error: string | null;
  retry(): void;
  /** The slider reached the header's own maximum and the chain goes on past it. */
  capped: boolean;
  /** What the daemon lists once the flow is done: "7 fans and 1 strip on 2 chains." */
  foundSummary: string | null;
}

interface Live {
  step: ChainStep;
  length: number;
  rows: ChainPartRow[];
}

/** What a part is, as the light says it: the kind it was given, else its arrangement's guess. */
function kindOf(view: LightView | null): ChainPartKind {
  if (!view) return "strip";
  return view.light.kind ?? (view.light.arrangement === "ring" ? "fan" : "strip");
}

/**
 * The rows the confirm step starts from. A re-setup at the same count shows the parts the owner
 * last confirmed: their names, their kinds and their lengths (which add up to the length on
 * screen, so an uneven chain stays uneven). A new length, or a count the chain does not have,
 * splits the LEDs evenly; names the owner typed stay where they were.
 */
function seedRows(chain: ChainView, count: number, length: number): ChainPartRow[] {
  if (chain.state === "set_up" && chain.parts.length === count) {
    const kept = chain.parts.map((view, i) => {
      const kind = kindOf(view);
      const name = view.light.name ?? defaultName(kind, i + 1);
      return { name, kind, leds: view.led_count, named: name !== defaultName(kind, i + 1) };
    });
    const total = kept.reduce((n, part) => n + part.leds, 0);
    if (total === length) return kept;
    const split = evenSplit(length, count);
    return kept.map((part, i) => ({ ...part, leds: split[i] ?? 1 }));
  }
  const kind = kindOf(chain.parts[0] ?? null);
  return evenSplit(length, count).map((leds, i) => ({
    name: defaultName(kind, i + 1),
    kind,
    leds,
    named: false,
  }));
}

const noun = (count: number, one: string, many: string): string =>
  `${count} ${count === 1 ? one : many}`;

/** "7 fans and 1 strip on 2 chains." across every chain that is set up. */
function foundOn(chains: ChainView[] | null): string | null {
  if (!chains) return null;
  const set = chains.filter((chain) => chain.state === "set_up");
  let fans = 0;
  let others = 0;
  for (const chain of set) {
    for (const view of chain.parts) {
      if (kindOf(view) === "fan") fans += 1;
      else others += 1;
    }
  }
  const what = [
    fans > 0 ? noun(fans, "fan", "fans") : null,
    others > 0 ? noun(others, "strip", "strips") : null,
  ]
    .filter(Boolean)
    .join(" and ");
  return `${what || "no parts"} on ${noun(set.length, "chain", "chains")}.`;
}

/**
 * One header's setup as the editor drives it. Call it with the chain the flow was opened for;
 * the flow's own values are read once, when it opens.
 */
export function useChainSetup(chain: ChainView): ChainSetup {
  // The header as the flow found it. A parent that polls the store passes a new object each
  // render; the flow must not restart for it.
  const [opening] = useState(chain);
  const device = opening.device_id;
  const zone = opening.zone_index;
  const ceiling = Math.max(1, opening.max_leds);
  const startLength = Math.min(Math.max(1, opening.leds), ceiling);
  const startCount = Math.min(
    Math.max(1, opening.state === "set_up" && opening.parts.length > 0 ? opening.parts.length : 1),
    startLength,
  );

  const [step, setStep] = useState<ChainStep>("length");
  const [length, setLengthState] = useState(startLength);
  const [rows, setRows] = useState<ChainPartRow[]>(() => seedRows(opening, startCount, startLength));
  const [error, setError] = useState<string | null>(null);

  /** The one copy the timers read. */
  const live = useRef<Live>({ step: "length", length: startLength, rows });
  /** The session the daemon opened, and whether it has ended. */
  const session = useRef<string | null>(null);
  const finished = useRef(false);
  /** When the last light call went, and the trailing call waiting for its window. */
  const sentAt = useRef(0);
  const trailing = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** What the last failure was, so Retry knows what to send again. */
  const failure = useRef<"start" | "commit" | "empty" | "light" | null>(null);

  const chains = useSnap((state) => state.chains);

  const stopTrailing = useCallback(() => {
    if (trailing.current !== null) {
      clearTimeout(trailing.current);
      trailing.current = null;
    }
  }, []);

  /** The runs the flow shows now: the length step is one run, every later step is the parts. */
  const runsNow = useCallback((): SetupRun[] => {
    const at = live.current;
    if (at.step === "length") {
      return [{ start: 0, count: Math.max(1, at.length), color: PART_COLOURS[0] ?? "#ffffff" }];
    }
    return runsFor(at.rows.map((row) => row.leds));
  }, []);

  /** Sends one light call now. A stale session (409) ends the flow: the daemon let it go. */
  const sendNow = useCallback(() => {
    const id = session.current;
    if (id === null || finished.current) return;
    sentAt.current = Date.now();
    api.chains
      .light(device, zone, { session: id, runs: runsNow() })
      .catch((e: unknown) => {
        if (finished.current) return;
        failure.current = "light";
        setError(messageOf(e));
        if (e instanceof ApiFailure && e.status === 409) {
          // The daemon ended the session (its timeout, or another window): only a new one goes on.
          finished.current = true;
          session.current = null;
          stopTrailing();
          live.current = { ...live.current, step: "error" };
          setStep("error");
        }
      });
  }, [device, zone, runsNow, stopTrailing]);

  /** Asks for a light call: at most one every 60 ms, and always with the newest value. */
  const send = useCallback(() => {
    if (session.current === null || finished.current) return;
    if (trailing.current !== null) return;
    const wait = Math.max(0, sentAt.current + LIGHT_THROTTLE_MS - Date.now());
    if (wait > 0) {
      // The window has passed when this runs, and it carries whatever the value is by then: the
      // call after the last one is the one that goes, so a drag never ends on a stale LED.
      trailing.current = setTimeout(() => {
        trailing.current = null;
        sendNow();
      }, wait);
      return;
    }
    sendNow();
  }, [sendNow]);

  /** Puts the values in the state and in the copy the timers read, then lights what they make. */
  const apply = useCallback(
    (next: Partial<Live>, opts?: { send?: boolean }) => {
      const merged = { ...live.current, ...next };
      live.current = merged;
      if ("step" in next) setStep(merged.step);
      if ("length" in next) setLengthState(merged.length);
      if ("rows" in next) setRows(merged.rows);
      if (opts?.send !== false) send();
    },
    [send],
  );

  /** Ends the session once. `keepalive` is for the page going away with it. */
  const endSession = useCallback(
    (keepalive: boolean) => {
      const id = session.current;
      if (id === null || finished.current) return;
      finished.current = true;
      stopTrailing();
      const body = { session: id };
      const answer = keepalive
        ? api.chains.cancel(device, zone, body, { keepalive: true })
        : api.chains.cancel(device, zone, body);
      void answer.catch(() => undefined);
    },
    [device, zone, stopTrailing],
  );

  const begin = useCallback(async () => {
    failure.current = null;
    try {
      const started = await api.chains.start(device, zone);
      if (finished.current) return;
      finished.current = false;
      session.current = started.session;
      sentAt.current = 0;
      setError(null);
      apply({ step: "length" });
    } catch (e) {
      failure.current = "start";
      setError(messageOf(e));
      apply({ step: "error" }, { send: false });
    }
  }, [apply, device, zone]);

  const setLength = useCallback(
    (n: number) => {
      const next = Math.min(ceiling, Math.max(1, Math.round(n)));
      apply({ length: next });
    },
    [apply, ceiling],
  );

  const setCount = useCallback(
    (n: number) => {
      const at = live.current;
      const count = Math.min(at.length, Math.max(1, Math.round(n)));
      apply({ rows: seedRows(opening, count, at.length) });
    },
    [apply, opening],
  );

  const setPartLeds = useCallback(
    (i: number, n: number) => {
      const at = live.current;
      if (i < 0 || i >= at.rows.length) return;
      const moved = moveBoundary(
        at.rows.map((row) => row.leds),
        i,
        n,
      );
      let total = moved.reduce((sum, leds) => sum + (leds ?? 0), 0);
      // The header drives what it drives: a last row grown past the maximum gives the excess back.
      if (total > ceiling) {
        const last = moved.length - 1;
        const trimmed = (moved[last] ?? 1) - (total - ceiling);
        if (trimmed < 1) return;
        moved[last] = trimmed;
        total = ceiling;
      }
      apply({ rows: at.rows.map((row, j) => ({ ...row, leds: moved[j] ?? row.leds })), length: total });
    },
    [apply, ceiling],
  );

  const setPartKind = useCallback(
    (i: number, kind: ChainPartKind) => {
      apply({
        rows: live.current.rows.map((row, j) =>
          j === i ? { ...row, kind, name: renameForKind(row.name, row.named, kind, j + 1) } : row,
        ),
      });
    },
    [apply],
  );

  const setPartName = useCallback(
    (i: number, name: string) => {
      apply(
        {
          rows: live.current.rows.map((row, j) =>
            j === i ? { ...row, name, named: name.trim() !== defaultName(row.kind, j + 1) } : row,
          ),
        },
        { send: false },
      );
    },
    [apply],
  );

  /** A row is added at the end, taking half of the last row's LEDs: the split is not redone. */
  const addPart = useCallback(() => {
    const at = live.current;
    const last = at.rows.at(-1);
    if (!last || last.leds < 2 || at.rows.length >= at.length) return;
    const take = Math.max(1, Math.floor(last.leds / 2));
    apply({
      rows: [
        ...at.rows.slice(0, -1),
        { ...last, leds: last.leds - take },
        { name: defaultName(last.kind, at.rows.length + 1), kind: last.kind, leds: take, named: false },
      ],
    });
  }, [apply]);

  /** A row is removed and its neighbour takes its LEDs; the chain keeps its length. */
  const removePart = useCallback(
    (i: number) => {
      const at = live.current;
      if (at.rows.length <= 1 || i < 0 || i >= at.rows.length) return;
      const next = at.rows.slice();
      const [gone] = next.splice(i, 1);
      const into = Math.min(i, next.length - 1);
      const keeper = next[into];
      if (keeper && gone) next[into] = { ...keeper, leds: keeper.leds + gone.leds };
      apply({ rows: next });
    },
    [apply],
  );

  const back = useCallback(() => {
    const at = live.current.step;
    if (at === "confirm") apply({ step: "count" });
    else if (at === "count") apply({ step: "length" });
    else if (at === "length") endSession(false);
  }, [apply, endSession]);

  const next = useCallback(() => {
    const at = live.current.step;
    if (at === "length") apply({ step: "count" });
    else if (at === "count") apply({ step: "confirm" });
  }, [apply]);

  const confirm = useCallback(async () => {
    const id = session.current;
    if (id === null || finished.current) return;
    failure.current = null;
    setError(null);
    const parts = live.current.rows.map((row, i) => ({
      name: row.name.trim() || defaultName(row.kind, i + 1),
      kind: row.kind,
      leds: Math.max(1, row.leds),
    }));
    try {
      await api.chains.commit(device, zone, { session: id, parts });
      finished.current = true;
      stopTrailing();
      await Promise.all([refreshChains(), refreshLights()]);
      apply({ step: "done" }, { send: false });
    } catch (e) {
      // The step stays, and so does the session: the daemon keeps it open for a retry.
      failure.current = "commit";
      setError(messageOf(e));
    }
  }, [apply, device, stopTrailing, zone]);

  const nothingLitUp = useCallback(async () => {
    const id = session.current;
    if (id === null || finished.current) return;
    failure.current = null;
    setError(null);
    try {
      await api.chains.empty(device, zone, { session: id });
      finished.current = true;
      stopTrailing();
      await refreshChains();
      apply({ step: "done" }, { send: false });
    } catch (e) {
      failure.current = "empty";
      setError(messageOf(e));
    }
  }, [apply, device, stopTrailing, zone]);

  const retry = useCallback(() => {
    if (failure.current === "commit") void confirm();
    else if (failure.current === "empty") void nothingLitUp();
    else void begin();
  }, [begin, confirm, nothingLitUp]);

  const later = useCallback(() => {
    endSession(false);
  }, [endSession]);

  // The session, its heartbeat, and the two ways a page going away ends it.
  useEffect(() => {
    const onHide = () => endSession(true);
    window.addEventListener("pagehide", onHide);
    window.addEventListener("beforeunload", onHide);
    const beat = setInterval(() => {
      if (session.current === null || finished.current) return;
      // The heartbeat is an ordinary ask: it takes the throttle's window like a drag, and the
      // trailing call always goes, so the session never waits more than a window for its next one.
      send();
    }, HEARTBEAT_MS);
    // The session opens one microtask after this effect: `begin` answers with state, and a
    // setState in the effect's own body is the cascading render the lint rule forbids.
    queueMicrotask(begin);
    return () => {
      clearInterval(beat);
      window.removeEventListener("pagehide", onHide);
      window.removeEventListener("beforeunload", onHide);
      endSession(false);
      // Nothing is sent for a tree that is gone, even when the start answer has not landed yet.
      finished.current = true;
    };
  }, [begin, endSession, send]);

  return {
    step,
    length,
    setLength,
    count: rows.length,
    setCount,
    parts: rows,
    setPartLeds,
    setPartKind,
    setPartName,
    addPart,
    removePart,
    back,
    next,
    confirm,
    nothingLitUp,
    later,
    error,
    retry,
    capped: length >= ceiling,
    foundSummary: step === "done" ? foundOn(chains) : null,
  };
}
