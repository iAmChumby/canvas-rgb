"use client";

// The chain setup flow's data: one session on one ARGB header, lit through the identify path, and
// what the owner says about it in the steps the design names (#381, #382): count the orange lights,
// slide to the last LED, count the devices (a series only), check.
//
// The session's lifetime is this hook's whole job. It opens when the owner presses Start (or at
// once for a flow opened from a device's row) and is ended exactly once: by a commit, by marking
// the header empty, by Later, by Back from the first question, by the page going away (a
// `keepalive` fetch, so a closed window still puts the length back) or by unmount. The daemon ends
// a session of its own accord 6 s after the last light call, so a light call is sent at most every
// 60 ms while the slider moves and again every 2 s as the heartbeat, which keeps a session open
// across a think, a step change and a failed commit's retry.
//
// No resize happens here. The daemon raised the header when the session opened and resizes once
// more at the commit; the flow only says which LEDs to light and what the devices are.
//
// The bookkeeping lives in refs, because the timers read it between renders; everything a render
// shows is state. `live` is the one copy the timers read, written by `apply` in the same breath as
// the state, so a trailing light call can never carry a value older than the last keystroke.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api } from "@/lib/endpoints";
import { ApiFailure, errorText } from "@/lib/errors";
import { refreshChains, refreshLights } from "@/lib/store";
import type { ChainPartKind } from "@/proto/ChainPartKind";
import type { ChainView } from "@/proto/ChainView";
import type { LightView } from "@/proto/LightView";
import type { SetupRun } from "@/proto/SetupRun";
import { COPIES_MAX } from "@/views/chain/limits";
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

/** The colour of the trail behind the orange end, and of every LED in the mirror check. */
export const TRAIL_COLOUR = "#ffffff";

/**
 * The screens of the flow. `intro` and `starting` come before the session exists; `lit` counts the
 * orange lights; `length` slides; `count` counts the devices (a series only); `check` is the series
 * rows or the mirror's one row; `nothing` and `blind` are the two ways out of "I saw no light".
 */
export type ChainStep =
  | "intro"
  | "starting"
  | "lit"
  | "length"
  | "count"
  | "check"
  | "nothing"
  | "blind"
  | "done"
  | "error";

/** What the flow opens on: the full intro, the card for a later header, or no intro at all. */
export type IntroKind = "full" | "next" | "none";

/** What a press of Start (or of "Nothing is plugged in here") is for. */
export type ChainIntent = "setup" | "empty";

/** One device's row in the series check. */
export interface ChainPartRow {
  name: string;
  kind: ChainPartKind;
  leds: number;
  /** True once the owner typed the name: a name that is the app's own follows a change of kind. */
  named: boolean;
}

export interface ChainSetup {
  step: ChainStep;
  /** What the starting screen is for. */
  intent: ChainIntent;
  /** The LEDs on the header, as the slider has them. Never a resize: the slider only lights. */
  length: number;
  setLength(n: number): void;
  /** How many orange lights the owner counts. 2 or more is a header whose devices mirror. */
  copies: number;
  setCopies(n: number): void;
  /** True when the devices on the header mirror each other (`copies` of 2 or more). */
  mirror: boolean;
  /** The mirror's one name, as the owner has it. */
  mirrorName: string;
  setMirrorName(s: string): void;
  count: number;
  setCount(n: number): void;
  parts: ChainPartRow[];
  setPartLeds(i: number, n: number): void;
  setPartKind(i: number, k: ChainPartKind): void;
  setPartName(i: number, s: string): void;
  addPart(): void;
  removePart(i: number): void;
  /** Start: opens the session. */
  start(): void;
  /** "Nothing is plugged in here": opens the session and marks the header empty. */
  startEmpty(): void;
  back(): void;
  next(): void;
  /** Back to counting the orange lights (the miscount guard's key). */
  countAgain(): void;
  confirm(): Promise<void>;
  /** "Nothing lit up": the screen with the checklist. Writes nothing. */
  noLight(): void;
  /** "I can't see inside". Writes nothing. */
  cantSee(): void;
  /** Try again: back to counting the orange lights. Lights LED 1 again and writes nothing. */
  tryAgain(): void;
  /** Marks the header empty. True when the daemon took it. */
  markEmpty(): Promise<boolean>;
  later(): void;
  error: string | null;
  /** Sends again what failed: a commit, the empty call, or a whole new session. One call at a time. */
  retry(): Promise<void>;
  /** The slider reached the header's own maximum and the header goes on past it. */
  capped: boolean;
  /** `saved` after a commit, `empty` after the header was marked empty. */
  doneKind: "saved" | "empty";
  /** The first line of the done screen. */
  doneText: string | null;
  /** The known limits that apply to what was saved, or null. */
  doneNote: string | null;
}

interface Live {
  step: ChainStep;
  length: number;
  copies: number;
  rows: ChainPartRow[];
  /** The mirror's name when the owner typed one; null means the app's own. */
  mirrorName: string | null;
}

/** What a part is, as the light says it: the kind it was given, else its arrangement's guess. */
function kindOf(view: LightView | null): ChainPartKind {
  if (!view) return "strip";
  return view.light.kind ?? (view.light.arrangement === "ring" ? "fan" : "strip");
}

/** The name a mirrored header's one light gets before the owner types one. */
export function mirrorDefaultName(copies: number): string {
  return `Same lights, shown ${copies} times`;
}

/** The times a stored light is shown: 1 for a light that is shown once. */
function copiesOf(view: LightView | null | undefined): number {
  const n = view?.light.copies;
  return typeof n === "number" && n >= 1 ? Math.min(n, COPIES_MAX) : 1;
}

/**
 * The rows the series check starts from. A re-setup at the same count shows the devices the owner
 * last confirmed: their names, their kinds and their lengths (which add up to the length on
 * screen, so an uneven header stays uneven). A new length, or a count the header does not have,
 * splits the LEDs evenly; names the owner typed stay where they were. A stored mirror is one light
 * that stands for many devices, so it is never kept as a row of a series.
 */
function seedRows(chain: ChainView, count: number, length: number): ChainPartRow[] {
  const keepable =
    chain.state === "set_up" &&
    chain.parts.length === count &&
    chain.parts.every((view) => copiesOf(view) === 1);
  if (keepable) {
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
  const first = chain.parts[0] ?? null;
  const kind = copiesOf(first) > 1 ? "fan" : kindOf(first);
  return evenSplit(length, count).map((leds, i) => ({
    name: defaultName(kind, i + 1),
    kind,
    leds,
    named: false,
  }));
}

/** The same devices at a new length: the first `count` keep their names and kinds. */
function reflow(rows: ChainPartRow[], length: number): ChainPartRow[] {
  if (rows.reduce((n, row) => n + row.leds, 0) === length) return rows;
  const count = Math.max(1, Math.min(rows.length, length));
  const split = evenSplit(length, count);
  return rows.slice(0, count).map((row, i) => ({ ...row, leds: split[i] ?? 1 }));
}

const noun = (count: number, one: string, many: string): string =>
  `${count} ${count === 1 ? one : many}`;

/**
 * One ARGB header's setup as the editor drives it. Call it with the header the flow was opened
 * for; the flow's own values are read once, when it opens. With `intro` of `full` or `next` the
 * flow waits on the intro and opens its session on Start; with `none` it opens it at once.
 */
export function useChainSetup(chain: ChainView, intro: IntroKind = "none"): ChainSetup {
  // The header as the flow found it. A parent that polls the store passes a new object each
  // render; the flow must not restart for it.
  const [opening] = useState(chain);
  const device = opening.device_id;
  const zone = opening.zone_index;
  const zoneName = opening.zone_name;
  const ceiling = Math.max(1, opening.max_leds);
  const storedSetUp = opening.state === "set_up" && opening.parts.length > 0;
  /** The orange lights counted before: a re-setup starts there. */
  const startCopies = storedSetUp ? copiesOf(opening.parts[0]) : 1;
  /** The LEDs the header has: a re-setup slides from there, a header never set up from 1. */
  const startLength = storedSetUp ? Math.min(Math.max(1, opening.leds), ceiling) : 1;
  const startCount = Math.min(
    Math.max(1, storedSetUp && startCopies === 1 ? opening.parts.length : 1),
    startLength,
  );
  const [autoStart] = useState(intro === "none");

  const [step, setStep] = useState<ChainStep>(autoStart ? "starting" : "intro");
  const [length, setLengthState] = useState(startLength);
  const [copies, setCopiesState] = useState(startCopies);
  const [rows, setRows] = useState<ChainPartRow[]>(() => seedRows(opening, startCount, startLength));
  const [mirrorName, setMirrorNameState] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [doneKind, setDoneKind] = useState<"saved" | "empty">("saved");
  const [intent, setIntent] = useState<ChainIntent>("setup");

  /** The one copy the timers read. */
  const live = useRef<Live>({
    step: autoStart ? "starting" : "intro",
    length: startLength,
    copies: startCopies,
    rows,
    mirrorName: null,
  });
  /** The session the daemon opened, and whether it has ended. */
  const session = useRef<string | null>(null);
  const finished = useRef(false);
  /** True once a flow without an introduction has opened its session at mount. React's strict
   * mode runs that mount effect twice, and the replay must not open a second session. */
  const opened = useRef(false);
  /** True while the empty call is out: a second press sends nothing. */
  const emptying = useRef(false);
  /** True while the commit is out: the Save key and the error banner's Retry cannot both land. */
  const confirming = useRef(false);
  /** True while a Retry is out, whatever it sends: a second press of the same key is dropped. */
  const retrying = useRef(false);
  /** What a Start is for, as `begin` reads it after the daemon has answered. */
  const intentRef = useRef<ChainIntent>("setup");
  /** When the last light call went, and the trailing call waiting for its window. */
  const sentAt = useRef(0);
  const trailing = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** What the last failure was, so Retry knows what to send again. */
  const failure = useRef<"start" | "commit" | "empty" | "light" | "stale" | null>(null);

  const stopTrailing = useCallback(() => {
    if (trailing.current !== null) {
      clearTimeout(trailing.current);
      trailing.current = null;
    }
  }, []);

  /** The runs the flow shows now, by screen. */
  const runsNow = useCallback((): SetupRun[] => {
    const at = live.current;
    const orange = PART_COLOURS[0] ?? "#ff4d00";
    switch (at.step) {
      case "lit":
      case "nothing":
      case "blind":
        // One LED, orange: the owner counts the orange lights.
        return [{ start: 0, count: 1, color: orange }];
      case "length": {
        // A white trail with an orange end: the length is the last LED where the orange shows.
        const n = Math.max(1, at.length);
        if (n === 1) return [{ start: 0, count: 1, color: orange }];
        return [
          { start: 0, count: n - 1, color: TRAIL_COLOUR },
          { start: n - 1, count: 1, color: orange },
        ];
      }
      case "count":
        return runsFor(at.rows.map((row) => row.leds));
      case "check":
        // A mirror check lights every LED white; a series check lights each device in its colour.
        if (at.copies >= 2) return [{ start: 0, count: Math.max(1, at.length), color: TRAIL_COLOUR }];
        return runsFor(at.rows.map((row) => row.leds));
      default:
        return [];
    }
  }, []);

  /** Sends one light call now. A stale session (409) ends the flow: the daemon let it go. */
  const sendNow = useCallback(() => {
    const id = session.current;
    if (id === null || finished.current) return;
    const runs = runsNow();
    if (runs.length === 0) return;
    sentAt.current = Date.now();
    api.chains
      .light(device, zone, { session: id, runs })
      .catch((e: unknown) => {
        if (finished.current) return;
        failure.current = "light";
        setError(errorText(e));
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
      if ("copies" in next) setCopiesState(merged.copies);
      if ("rows" in next) setRows(merged.rows);
      if ("mirrorName" in next) setMirrorNameState(merged.mirrorName);
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

  /** Marks the header empty. True when the daemon took it. One call at a time. */
  const markEmpty = useCallback(async (): Promise<boolean> => {
    const id = session.current;
    if (id === null || finished.current || emptying.current) return false;
    emptying.current = true;
    /** Where the owner pressed from: the checklist, or the introduction's own empty key. */
    const from = live.current.step;
    failure.current = null;
    setError(null);
    try {
      await api.chains.empty(device, zone, { session: id });
      finished.current = true;
      stopTrailing();
      await refreshChains();
      setDoneKind("empty");
      apply({ step: "done" }, { send: false });
      return true;
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 409) {
        // The daemon has let the session go (its 6 s timeout, or another window): the empty write
        // cannot be sent on it again. The flow ends on the error screen and Retry opens a new
        // session, as a stale commit does. The introduction holds a session it never lights (its
        // screen is the same one orange LED, drawn by the previous step), so nothing here can keep
        // a timeout from happening; this branch is the way out of it.
        finished.current = true;
        session.current = null;
        stopTrailing();
        failure.current = "stale";
        // Retry starts a new session, and that session is a setup: the empty write the owner asked
        // for was never accepted, and repeating a write that keeps them dark is theirs to ask again.
        intentRef.current = "setup";
        setIntent("setup");
        setError(errorText(e));
        apply({ step: "error" }, { send: false });
        return false;
      }
      // The header stays as it was: the screen the key was pressed on shows the daemon's words and
      // Retry, and the session stays open. "Nothing is plugged in here" was never shown a light, so
      // it goes back to the introduction; the checklist would advise about a light nobody saw.
      failure.current = "empty";
      setError(errorText(e));
      apply({ step: from === "starting" ? "intro" : "nothing" });
      return false;
    } finally {
      emptying.current = false;
    }
  }, [apply, device, stopTrailing, zone]);

  /** Opens the session. What comes next is the intent: count the lights, or mark the header empty. */
  const begin = useCallback(async () => {
    failure.current = null;
    if (session.current !== null) {
      // A session is already open for this flow: the empty call failed and the screen went back to
      // the introduction, so the header is still raised. Starting a second one would leave the
      // first open until the daemon's own timeout.
      if (intentRef.current === "empty") {
        await markEmpty();
        return;
      }
      apply({ step: "lit" });
      return;
    }
    try {
      const started = await api.chains.start(device, zone);
      if (finished.current) {
        // Later was pressed while the daemon was still raising the header: the session that just
        // opened is ended at once so the header goes back to its length.
        void api.chains.cancel(device, zone, { session: started.session }).catch(() => undefined);
        return;
      }
      session.current = started.session;
      sentAt.current = 0;
      setError(null);
      if (intentRef.current === "empty") {
        await markEmpty();
        return;
      }
      apply({ step: "lit" });
    } catch (e) {
      failure.current = "start";
      setError(errorText(e));
      apply({ step: "error" }, { send: false });
    }
  }, [apply, device, markEmpty, zone]);

  const start = useCallback(() => {
    if (live.current.step !== "intro") return;
    intentRef.current = "setup";
    setIntent("setup");
    apply({ step: "starting" }, { send: false });
    void begin();
  }, [apply, begin]);

  const startEmpty = useCallback(() => {
    if (live.current.step !== "intro") return;
    intentRef.current = "empty";
    setIntent("empty");
    apply({ step: "starting" }, { send: false });
    void begin();
  }, [apply, begin]);

  const setLength = useCallback(
    (n: number) => {
      const next = Math.min(ceiling, Math.max(1, Math.round(n)));
      apply({ length: next });
    },
    [apply, ceiling],
  );

  const setCopies = useCallback(
    (n: number) => {
      const next = Math.min(COPIES_MAX, Math.max(1, Math.round(n)));
      apply({ copies: next }, { send: false });
    },
    [apply],
  );

  const setMirrorName = useCallback(
    (name: string) => {
      apply({ mirrorName: name }, { send: false });
    },
    [apply],
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

  /** A row is removed and its neighbour takes its LEDs; the header keeps its length. */
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

  const later = useCallback(() => {
    endSession(false);
    // Later while the header is still being raised: the answer that lands later ends its session.
    finished.current = true;
  }, [endSession]);

  const back = useCallback(() => {
    const at = live.current;
    if (at.step === "check") apply({ step: at.copies >= 2 ? "length" : "count" });
    else if (at.step === "count") apply({ step: "length" });
    else if (at.step === "length") apply({ step: "lit" });
    else if (at.step === "lit") later();
  }, [apply, later]);

  const next = useCallback(() => {
    const at = live.current;
    if (at.step === "lit") apply({ step: "length" });
    else if (at.step === "length") {
      if (at.copies >= 2) apply({ step: "check" });
      else apply({ step: "count", rows: reflow(at.rows, at.length) });
    } else if (at.step === "count") apply({ step: "check" });
  }, [apply]);

  const countAgain = useCallback(() => {
    apply({ step: "lit" });
  }, [apply]);

  const noLight = useCallback(() => {
    setError(null);
    apply({ step: "nothing" });
  }, [apply]);

  const cantSee = useCallback(() => {
    setError(null);
    apply({ step: "blind" });
  }, [apply]);

  const tryAgain = useCallback(() => {
    setError(null);
    failure.current = null;
    apply({ step: "lit" });
  }, [apply]);

  const confirm = useCallback(async () => {
    const id = session.current;
    // A ref, not state: the Save key and the error banner's Retry can both be pressed inside one
    // tick, where neither has rendered its busy state yet, and the daemon must see one commit.
    if (id === null || finished.current || confirming.current) return;
    confirming.current = true;
    failure.current = null;
    setError(null);
    const at = live.current;
    const parts =
      at.copies >= 2
        ? [
            {
              name: at.mirrorName?.trim() || mirrorDefaultName(at.copies),
              kind: "other" as const,
              leds: Math.max(1, at.length),
              copies: at.copies,
            },
          ]
        : at.rows.map((row, i) => ({
            name: row.name.trim() || defaultName(row.kind, i + 1),
            kind: row.kind,
            leds: Math.max(1, row.leds),
          }));
    try {
      await api.chains.commit(device, zone, { session: id, parts });
      finished.current = true;
      stopTrailing();
      await Promise.all([refreshChains(), refreshLights()]);
      setDoneKind("saved");
      apply({ step: "done" }, { send: false });
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 409) {
        // The daemon let the session go (its timeout, or another window): the save cannot be
        // retried on it. The flow ends on the error screen, and Retry opens a new session, as a
        // stale light call does.
        finished.current = true;
        session.current = null;
        stopTrailing();
        failure.current = "stale";
        setError(errorText(e));
        apply({ step: "error" }, { send: false });
        return;
      }
      // The step stays, and so does the session: the daemon keeps it open for a retry.
      failure.current = "commit";
      setError(errorText(e));
    } finally {
      confirming.current = false;
    }
  }, [apply, device, stopTrailing, zone]);

  const retry = useCallback(async () => {
    // One retry at a time: the banner and the error screen both offer the key, and two presses in
    // one tick would otherwise open two sessions or send two commits.
    if (retrying.current) return;
    retrying.current = true;
    try {
      if (failure.current === "commit") await confirm();
      else if (failure.current === "empty") await markEmpty();
      else {
        // A new session: the old one is gone (a failed start, or the daemon let it go).
        finished.current = false;
        session.current = null;
        apply({ step: "starting" }, { send: false });
        await begin();
      }
    } finally {
      retrying.current = false;
    }
  }, [apply, begin, confirm, markEmpty]);

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
    // A flow without an intro opens its session one microtask after this effect: `begin` answers
    // with state, and a setState in the effect's own body is the cascading render the lint rule
    // forbids. A flow with an intro waits for Start.
    //
    // React's strict mode runs this effect twice (mount, cleanup, mount). The cleanup above has
    // just set `finished`, and no session exists yet, so the second run only takes that flag back:
    // one mount opens the session, and its answer lands rather than cancelling itself.
    if (autoStart) {
      if (opened.current) {
        // The replay: nothing has been started yet and the screen has not moved on, so the start
        // answer still in flight is this flow's and must land.
        if (session.current === null && live.current.step === "starting") finished.current = false;
      } else {
        opened.current = true;
        queueMicrotask(() => void begin());
      }
    }
    return () => {
      clearInterval(beat);
      window.removeEventListener("pagehide", onHide);
      window.removeEventListener("beforeunload", onHide);
      endSession(false);
      // Nothing is sent for a tree that is gone, even when the start answer has not landed yet.
      finished.current = true;
    };
  }, [autoStart, begin, endSession, send]);

  const mirror = copies >= 2;
  const total = useMemo(() => rows.reduce((n, row) => n + row.leds, 0), [rows]);

  let doneText: string | null = null;
  let doneNote: string | null = null;
  if (step === "done") {
    if (doneKind === "empty") {
      doneText = "Marked empty. Set it up again from Devices, then add its lights to your scenes.";
    } else if (mirror) {
      doneText = `Saved. The same ${length} ${length === 1 ? "LED shows" : "LEDs show"} ${copies} times on ${zoneName}.`;
      doneNote = "Devices of different lengths on this header are stored at the length you chose.";
    } else {
      doneText = `Saved. ${noun(rows.length, "device", "devices")}, ${noun(total, "LED", "LEDs")} on ${zoneName}.`;
      doneNote =
        "A device hidden behind a radiator can end the orange early. If one looks short, set up again from Devices, then add its lights to your scenes.";
    }
  }

  return {
    step,
    intent,
    length,
    setLength,
    copies,
    setCopies,
    mirror,
    mirrorName: mirrorName ?? mirrorDefaultName(copies),
    setMirrorName,
    count: rows.length,
    setCount,
    parts: rows,
    setPartLeds,
    setPartKind,
    setPartName,
    addPart,
    removePart,
    start,
    startEmpty,
    back,
    next,
    countAgain,
    confirm,
    noLight,
    cantSee,
    tryAgain,
    markEmpty,
    later,
    error,
    retry,
    capped: length >= ceiling,
    doneKind,
    doneText,
    doneNote,
  };
}
