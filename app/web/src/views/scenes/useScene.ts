"use client";

// The Scenes page's data: the active scene, the lights no group holds, and every way to change
// them.
//
// A change reaches the screen at once and goes to the daemon as the WHOLE scene with the `rev`
// it was made from (`PUT /scenes/{id}`). Writes go one at a time, newest state last, so each
// one carries the rev the one before it returned. A 409 means another window saved first: this
// window takes the daemon's scene and says so. An edit made while the daemon was answering is
// not lost: it goes again on the daemon's scene, so the two windows' changes both survive.
// Undo and redo are a history of whole scenes for the active scene; an undo is an ordinary edit
// that puts the older scene back.
//
// `setProps` and `setBrightness` are the live paths for a slider drag: they go to the engine
// (`PATCH .../props`, `PUT .../brightness`) at most once per 60 ms per kind, and the last value
// always goes. One live write of the scene is on its way at a time, across the groups, so each
// answer can be checked: it must carry the rev its write was sent on plus one. Any other rev
// means another window saved in between, which is handled as a 409 is. The daemon answers those
// writes with the whole scene and its new `rev`, so a whole-scene save after a live write carries
// that rev and never reverts the value: the live values go with the save. Live values are keyed
// by scene and group, since two scenes can share a group id. A write the daemon refuses is tried
// again, three times in a row at most; then its value leaves the queue and the screen. A scene
// that goes, or a page that goes away, sends its waiting values through the same queue.
//
// The bookkeeping lives in refs, because the write loop and the timers read it between renders.
// Everything a render shows is copied into one state object (`publish`), so a render never
// reads a ref.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { notify } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { getSnap, patchSnap, refreshLights, refreshScenes, useSnap } from "@/lib/store";
import { moveEmptiedMessage } from "@/views/scenes/groupEdit";
import type { Group } from "@/proto/Group";
import type { GroupState } from "@/proto/GroupState";
import type { GroupMoved } from "@/proto/GroupMoved";
import type { LightView } from "@/proto/LightView";
import type { Plays } from "@/proto/Plays";
import type { Scene } from "@/proto/Scene";
import type { SceneWrite } from "@/proto/SceneWrite";
import type { ScenesResponse } from "@/proto/ScenesResponse";
import type { Props } from "@/proto/props";
import type { PropValue } from "@/proto/PropValue";

/** How many whole scenes undo remembers. */
export const HISTORY_DEPTH = 100;
/** The shortest time between two live writes of one kind and one group. */
export const LIVE_THROTTLE_MS = 60;
/** How long a live write waits when a whole-scene save holds the daemon. */
export const LIVE_RETRY_MS = 20;
/**
 * How many tries a live write of one group gets in a row. A write the daemon refuses is tried
 * again after the throttle; after the last refusal the value leaves the queue and the screen, and
 * the error stays. A daemon that is down is not asked again and again.
 */
export const LIVE_FAILURES = 3;
/**
 * How long the read that follows a live write whose outcome is unknown waits for its answer. A
 * write's slot is held until the read settles, so the read gives up on its own: a daemon that
 * stopped mid-answer must not hold the live writes of the page.
 */
export const LIVE_READ_TIMEOUT_MS = 5000;
/** What the page says after a 409. */
export const CONFLICT_MESSAGE = "Changed in another window";
/** What the page says when another window changed the scenes before a move could land. */
export const NOTHING_MOVED = "Changed in another window. Nothing moved.";
/** What the page says when another window changed the scenes before an undo could land. */
export const NOTHING_UNDONE = "Changed in another window. Nothing undone.";
/**
 * How long a burst of key presses waits before the order goes to the daemon. A burst is one
 * order and one write: the screen shows every press at once, the daemon hears one order.
 */
export const ORDER_COALESCE_MS = 250;
/** What the undo step of a move is called. */
export const MOVE_STEP_LABEL = "Move the group to another scene";
/** What the undo step of a copy is called. */
export const COPY_STEP_LABEL = "Copy the group to another scene";

/**
 * What the daemon says about a move that has not been made yet (#368).
 *
 * The daemon's own dry run is the answer: the page does not work the rules out a second time,
 * so a move can never be allowed here and refused there.
 */
export type MoveCheck =
  /** The answer is on its way. */
  | { state: "pending" }
  /** The daemon's answer: what the move would do. */
  | { state: "answered"; answer: GroupMoved }
  /** The daemon refused it, in its own words. */
  | { state: "refused"; message: string }
  /** The check could not be made: the daemon was not there, or it failed. */
  | { state: "failed" };

const EMPTY_SCENES: Scene[] = [];
const EMPTY_LIGHTS: LightView[] = [];
const EMPTY_STATES: GroupState[] = [];

const clone = <T,>(value: T): T => structuredClone(value);

/** The key of one group's live values. Two scenes can name the same group id, so the scene is part of it. */
const liveKey = (sceneId: string, groupId: string): string => JSON.stringify([sceneId, groupId]);

/** One group's live values the daemon has not confirmed yet. */
interface Live {
  props?: Props;
  brightness?: number;
}

/** The two kinds of live write a group has. */
type LiveKind = "props" | "brightness";

/** What a live write's lost answer left behind, read from the daemon's own scene. */
type LostAnswer =
  /** The daemon is one rev on and holds what was sent: the write landed, only its answer was lost. */
  | { landed: Scene }
  /** The rev moved on without what was sent: another window saved in between. */
  | { conflict: Scene }
  /** The daemon is still on the rev the write was sent on: the write did not land. */
  | null;

/**
 * One prop value as text, so two of them compare by what they say and not by their key order.
 *
 * The `switch` names every kind of `PropValue`: a new kind fails the typecheck here, instead of
 * falling into a `String()` that would make two different values of it compare equal.
 */
function propText(value: PropValue): string {
  switch (value.type) {
    // An `Rgb8` is its `#rrggbb` string on the wire (`crates/canvas-proto/src/color.rs`).
    case "color":
      return `color:${value.value}`;
    case "choice":
      return `choice:${value.value}`;
    case "bool":
      return `bool:${value.value ? "true" : "false"}`;
    // A number, a hue and an angle are numbers: two of them are the same only when they are.
    case "number":
    case "hue":
    case "angle":
      return `${value.type}:${value.value}`;
    default: {
      const unknown: never = value;
      return unknown;
    }
  }
}

/** True when two groups play the same thing with the same values. */
function samePlays(a: Plays, b: Plays): boolean {
  if (a.type !== b.type) return false;
  if (a.type === "off") return true;
  if (a.type !== "effect" || b.type !== "effect") return false;
  if (a.effect !== b.effect) return false;
  // The props are compared by what they say, not by their key order: the daemon's own map has no
  // order of its own.
  for (const key of Object.keys(a.props)) {
    const mine = a.props[key];
    const theirs = b.props[key];
    if (!mine || !theirs) return false;
    if (propText(mine) !== propText(theirs)) return false;
  }
  return Object.keys(a.props).length === Object.keys(b.props).length;
}

/**
 * True when two scenes hold the same groups, in the same order, with the same lights and the
 * same effect values. The undo compares the scene the daemon has now with the one the move left:
 * the same groups mean nothing else touched it, so the undo may write it back.
 */
function sameGroups(a: Group[], b: Group[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((group, i) => {
    const other = b[i];
    if (!other) return false;
    if (group.id !== other.id || group.name !== other.name) return false;
    if (group.mode !== other.mode || group.brightness !== other.brightness) return false;
    if (group.members.length !== other.members.length) return false;
    if (!group.members.every((m, j) => m === other.members[j])) return false;
    return samePlays(group.plays, other.plays);
  });
}

/**
 * True when `group` holds exactly the values a live write sent. A key that is there with another
 * value is not this write's: another window may have saved, or the daemon fitted the patch to the
 * effect's schema. Either way the answer is ambiguous, and it is taken as a conflict.
 */
function holdsValue(group: Group, kind: LiveKind, written: Props | number): boolean {
  if (kind === "brightness") return group.brightness === written;
  const plays = group.plays;
  if (plays.type !== "effect") return false;
  const sent = written as Props;
  return Object.keys(sent).every((key) => {
    const got = plays.props[key];
    const want = sent[key];
    return got !== undefined && want !== undefined && propText(got) === propText(want);
  });
}

/**
 * True when a live write's failure does not say whether the daemon saved it: no answer at all (a
 * transport failure, a deadline) or a 5xx (the daemon may have saved before it failed). A 4xx is
 * the daemon's own refusal, so nothing was saved and there is nothing to read back.
 */
function outcomeUnknown(e: unknown): boolean {
  if (!(e instanceof ApiFailure)) return true;
  return e.status === 0 || e.status >= 500;
}

/** One change this window made that the daemon has not confirmed yet. */
interface Change {
  /** Numbers the change, so a failed write can take its own off the undo stack. */
  id: number;
  /** What the change does to a copy of the scene. */
  apply: (draft: Scene) => void;
  /** What the change is called, for the page. */
  label: string;
}

/** One write that has not gone out yet: the scene to save and the changes it carries. */
interface Pending {
  /** The scene to send, with the `rev` it was made from. */
  scene: Scene;
  /** The changes it carries, so a conflict can put them on the daemon's scene. */
  changes: Change[];
}

/** One group's live values: what is on screen, what waits to go, and the throttle. */
interface GroupLive {
  /** The scene the group belongs to. */
  sceneId: string;
  /** The group the values belong to. */
  groupId: string;
  /** The values on screen that the daemon has not confirmed. */
  shown: Live;
  /** The newest value of each kind that waits to go. */
  queued: Live;
  /** How many times the user moved each kind. */
  version: Record<LiveKind, number>;
  /** When each kind was last sent, so a kind waits its 60 ms. */
  sentAt: Record<LiveKind, number>;
  /** Live writes of each kind that failed in a row. One kind's failures never refuse the other. */
  failures: Record<LiveKind, number>;
  /** A write for this group is on its way. */
  sending: boolean;
  /** The timer that sends the next value. */
  timer: ReturnType<typeof setTimeout> | null;
  /** The values that wait go now, without the 60 ms wait: the page or the scene is going away. */
  hurry: boolean;
}

/**
 * A change to the scenes themselves rather than to the scene that plays: a scene was renamed,
 * made, duplicated or deleted. It is put back by its own command, because the daemon has no
 * route that undoes it.
 */
export interface Command {
  /** What the change was called. */
  label: string;
  /**
   * Puts the world back the way it was. `false` when it could not: the step is dropped and the
   * page says why, so the undo stack never keeps a step that does nothing.
   */
  undo: () => Promise<boolean | void>;
  /** Does it again. `false` when it could not. */
  redo: () => Promise<boolean | void>;
}

/** One step of the undo history. */
interface Entry {
  /** The scene before the change, when the change was inside a scene. */
  scene?: Scene;
  /** The change to put back, when the change was to the scenes themselves. */
  command?: Command;
  /** What the change was called. */
  label: string;
  /** The change this step undoes, when it was one this window made. */
  changeId?: number;
}

/** The undo history: the changes of the scene that plays, and the changes to the scenes. */
interface History {
  /** The scene the content changes belong to. */
  sceneId: string;
  past: Entry[];
  future: Entry[];
  /**
   * The next change of the active scene is one this window made (a scene was deleted, made or
   * duplicated): the history keeps its steps, so the change can be put back.
   */
  keep?: boolean;
}

/** What a render shows of this window's own bookkeeping. */
interface View {
  /** What this window changed and the daemon has not answered yet. */
  draft: Scene | null;
  /** The newest scene a write was answered with. */
  saved: Scene | null;
  error: string | null;
  canUndo: boolean;
  canRedo: boolean;
  /**
   * What the Undo would take back: the label of the top step of the history, or null when there
   * is none. A toast that offers an Undo asks for this: the Undo is only the answer it means
   * while the step it belongs to is still the top one.
   */
  undoLabel: string | null;
  /** The scene the history belongs to. */
  historyScene: string;
  live: Record<string, Live>;
}

const BLANK_VIEW: View = {
  draft: null,
  saved: null,
  error: null,
  canUndo: false,
  canRedo: false,
  undoLabel: null,
  historyScene: "",
  live: {},
};

/**
 * How one change is taken. A change the owner made is on the undo stack; a change the page made
 * to show something (the effect picker's preview) is not: it is put back when the preview ends,
 * and an Undo must not land on it.
 */
export interface EditOptions {
  /** The change is a preview: it goes to the daemon and on the screen, but not on the undo stack. */
  transient?: boolean;
}

export interface UseScene {
  scene: Scene | null;
  scenes: Scene[];
  /** Lights no group of the active scene holds. */
  unassigned: LightView[];
  /** How each group of the active scene is doing. */
  states: GroupState[];
  /** Applies `change` to a copy of the scene at once, then saves the whole scene. `label` names the change for the page. */
  edit(change: (draft: Scene) => void, label: string, options?: EditOptions): void;
  setProps(groupId: string, props: Props): void;
  setBrightness(groupId: string, n: number): void;
  setActive(id: string): void;
  create(name: string, copyOf?: string): void;
  remove(id: string): void;
  /** A scene's own name, whether or not it is the one playing. */
  renameScene(id: string, name: string): void;
  /**
   * Asks the daemon what a move would do, without writing anything (#368). The daemon's own
   * dry run is the answer: the page does not work the rules out a second time.
   */
  dryRunMove(groupId: string, toSceneId: string, copy: boolean): Promise<MoveCheck>;
  /**
   * Moves or copies one group to another scene (#368). The daemon writes both scenes in one save
   * and answers what it did; an Undo puts both scenes back. The answer is what the page says.
   *
   * `expected` names the revisions the move is made from: the ones a dry run answered with, so
   * the move that follows a dry run carries the same ones.
   */
  moveGroup(
    groupId: string,
    toSceneId: string,
    copy: boolean,
    expected?: { from: number; to: number },
  ): Promise<GroupMoved | null>;
  /**
   * Puts the scenes in another order (#371). The order shows at once; the daemon is told in one
   * write. A burst of key presses is one order and one write (`coalesce`).
   */
  reorderScenes(order: string[], options?: { coalesce?: boolean }): void;
  undo(): void;
  redo(): void;
  canUndo: boolean;
  canRedo: boolean;
  /** What the Undo would take back: the label of the top step of the history, or null. */
  undoLabel: string | null;
  error: string | null;
}

/** The scene with each group's unconfirmed live values laid over it. */
function overlayOn(scene: Scene, values: Record<string, Live>): Scene {
  if (Object.keys(values).length === 0) return scene;
  return {
    ...scene,
    groups: scene.groups.map((g) => {
      const l = values[liveKey(scene.id, g.id)];
      if (!l) return g;
      return {
        ...g,
        brightness: l.brightness ?? g.brightness,
        plays:
          l.props && g.plays.type === "effect"
            ? { ...g.plays, props: { ...g.plays.props, ...l.props } }
            : g.plays,
      };
    }),
  };
}

/** `scene` with each group's unconfirmed live values laid over it. */
function overlay(scene: Scene | null, values: Record<string, Live>): Scene | null {
  if (!scene) return null;
  return overlayOn(scene, values);
}

/** The scene with the changes applied to it. */
function withChanges(scene: Scene, changes: Change[]): Scene {
  const next = clone(scene);
  for (const c of changes) c.apply(next);
  return next;
}

/** One undo step per change, each restoring the scene as it stood just before that change. */
function undoStepsFor(base: Scene, changes: Change[]): Entry[] {
  const steps: Entry[] = [];
  const state = clone(base);
  for (const c of changes) {
    steps.push({ scene: clone(state), label: c.label, changeId: c.id });
    c.apply(state);
  }
  return steps;
}

/** The newer of what the daemon says and what a write of this window returned, for one scene. */
function newest(server: Scene | null, saved: Scene | null, id: string | null): Scene | null {
  if (saved && saved.id === id && (!server || saved.rev > server.rev)) return saved;
  return server;
}

/**
 * The draft with the daemon's own answer for one group laid over it: the rev the answer
 * carries, and the value the live write put there.
 *
 * The rest of the group stays as the user left it. The answer is the daemon's whole scene,
 * and taking the group from it would drop an edit the user made while the write was out.
 */
function adoptGroup(
  draft: Scene,
  answer: Scene,
  groupId: string,
  kind: LiveKind,
  written: Props | number,
): Scene {
  const group = answer.groups.find((g) => g.id === groupId);
  const rev = Math.max(draft.rev, answer.rev);
  if (!group) return { ...draft, rev };
  return {
    ...draft,
    rev,
    groups: draft.groups.map((g) => {
      if (g.id !== groupId) return g;
      if (kind === "brightness") return { ...g, brightness: group.brightness };
      if (g.plays.type !== "effect" || group.plays.type !== "effect") return g;
      // The values the write put there: the daemon's own, where it has them.
      const sent = written as Props;
      const props = { ...g.plays.props };
      for (const key of Object.keys(sent)) {
        props[key] = group.plays.props[key] ?? sent[key]!;
      }
      return { ...g, plays: { ...g.plays, props } };
    }),
  };
}

export function useScene(): UseScene {
  const scenesAnswer = useSnap((s) => s.scenes);
  const lights = useSnap((s) => s.lights);
  const [view, setView] = useState<View>(BLANK_VIEW);

  const activeId = scenesAnswer?.active ?? null;
  const server = useMemo(
    () => scenesAnswer?.scenes.find((s) => s.id === scenesAnswer.active) ?? null,
    [scenesAnswer],
  );

  /** The writes that have not gone out yet, one per scene. */
  const outgoing = useRef(new Map<string, Pending>());
  /** The newest scene a write was answered with. */
  const saved = useRef<Scene | null>(null);
  /** Numbers the changes, so a failed write can take its own off the undo stack. */
  const changeSeq = useRef(0);
  const history = useRef<History>({ sceneId: "", past: [], future: [] });
  /** The write loop is on its way. */
  const flushing = useRef(false);
  /** A whole-scene write is on its way, so no live write may start. */
  const writing = useRef(false);
  const error = useRef<string | null>(null);
  const live = useRef(new Map<string, GroupLive>());
  /** The live writes that are on their way. */
  const liveBusy = useRef(0);
  /** The whole-scene writes that wait for the live writes to finish. */
  const liveWaiters = useRef<Array<() => void>>([]);
  /** The sends of one group that are on their way: a move waits for them. */
  const liveIdle = useRef(new Map<string, Array<() => void>>());
  /** The writes that wait for one scene to be quiet (a move waits for the edit before it). */
  const quiet = useRef(new Map<string, Array<() => void>>());
  /** The order the page shows while a reorder of this window is on its way. */
  const [localOrder, setLocalOrder] = useState<string[] | null>(null);
  /** The same order, for the callbacks that run between renders. */
  const localOrderRef = useRef<string[] | null>(null);
  /** The order the daemon last confirmed: what a write names as the order it was made from. */
  const knownOrder = useRef<string[]>([]);
  /** The order writes go one after the other: the next one names the order the last one sent. */
  const orderChain = useRef<Promise<void>>(Promise.resolve());
  /**
   * The message the last move left in the toast lane, and the undo step it belongs to. The toast
   * is said again when the Undo it offers is no longer the one: an Undo takes the top step of
   * the history back, so it may only be offered while the move is still on top.
   */
  const lastMove = useRef<{ text: string; tone: "ok" | "warn"; label: string } | null>(null);
  /**
   * Whether the toast of the last move offers the Undo now. The toast is said again only when
   * that changes, so a move is said once and a later change does not bring it back.
   */
  const undoOffered = useRef(false);
  /** The timer that sends one order for a burst of key presses. */
  const orderTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mounted = useRef(false);
  // What the daemon last said, for the callbacks that run between renders.
  const known = useRef<{ server: Scene | null; activeId: string | null }>({ server: null, activeId: null });
  // The live write calls itself; the ref keeps the newest copy without making every callback
  // below change on every render.
  const sendLiveRef = useRef<(groupId: string) => Promise<void>>(async () => undefined);

  /** Copies the bookkeeping into state, so the next render shows it. */
  const publish = useCallback(() => {
    if (!mounted.current) return;
    const h = history.current;
    const active = known.current.activeId;
    const pending = active ? outgoing.current.get(active) : undefined;
    const shown: Record<string, Live> = {};
    for (const [id, g] of live.current) shown[id] = g.shown;
    setView({
      draft: pending?.scene ?? null,
      saved: saved.current,
      error: error.current,
      canUndo: h.past.length > 0,
      canRedo: h.future.length > 0,
      undoLabel: h.past.length > 0 ? (h.past[h.past.length - 1]?.label ?? null) : null,
      historyScene: h.sceneId,
      live: shown,
    });
  }, []);

  const fail = useCallback(
    (message: string | null) => {
      error.current = message;
      publish();
    },
    [publish],
  );

  /** The newest scene known without a pending change: the daemon's, or the one a write returned. */
  const base = useCallback(
    (): Scene | null => newest(known.current.server, saved.current, known.current.activeId),
    [],
  );

  /** Puts the daemon's answer into the snapshot, so a later poll cannot bring an older scene back. */
  const adoptScene = useCallback((answer: Scene) => {
    saved.current = answer;
    const current = getSnap().scenes;
    if (!current) return;
    patchSnap({
      scenes: {
        ...current,
        scenes: current.scenes.map((s) => (s.id === answer.id && answer.rev >= s.rev ? answer : s)),
      },
    });
  }, []);

  /** Waits until no live write is on its way. */
  const waitForLive = useCallback(
    () =>
      new Promise<void>((resolve) => {
        liveWaiters.current.push(resolve);
      }),
    [],
  );

  /** Waits until one group has nothing left to send. */
  const waitLiveGroup = useCallback(
    (key: string) =>
      new Promise<void>((resolve) => {
        const list = liveIdle.current.get(key) ?? [];
        list.push(resolve);
        liveIdle.current.set(key, list);
      }),
    [],
  );

  /** Says that one group has nothing left to send: the waiters for it may go on. */
  const settleLiveGroup = useCallback((key: string) => {
    const list = liveIdle.current.get(key);
    if (!list) return;
    liveIdle.current.delete(key);
    for (const resolve of list) resolve();
  }, []);

  /** Resolves when the scene has no whole-scene write on its way or waiting. */
  const waitQuiet = useCallback(
    (sceneId: string) =>
      new Promise<void>((resolve) => {
        if (!outgoing.current.has(sceneId)) {
          resolve();
          return;
        }
        const list = quiet.current.get(sceneId) ?? [];
        list.push(resolve);
        quiet.current.set(sceneId, list);
      }),
    [],
  );

  /** Says that a scene is quiet again: the write that was on its way has landed or failed. */
  const settleQuiet = useCallback((sceneId: string) => {
    const list = quiet.current.get(sceneId);
    if (!list) return;
    quiet.current.delete(sceneId);
    for (const resolve of list) resolve();
  }, []);

  /**
   * Puts the daemon's answer for several scenes into the snapshot, so the page shows it at once.
   * A scene the answer is older than is left alone: a poll that landed in between must not take
   * a newer scene back.
   */
  const adoptScenes = useCallback((scenes: Scene[]) => {
    const current = getSnap().scenes;
    if (!current) return;
    const byId = new Map(current.scenes.map((s) => [s.id, s]));
    for (const scene of scenes) {
      const old = byId.get(scene.id);
      if (!old || scene.rev >= old.rev) byId.set(scene.id, scene);
    }
    patchSnap({ scenes: { ...current, scenes: [...byId.values()] } });
  }, []);

  /** The scene a whole-scene write sends: the draft with the live values on top. */
  const body = useCallback((scene: Scene): Scene => {
    const shown: Record<string, Live> = {};
    for (const [id, g] of live.current) shown[id] = g.shown;
    return overlayOn(scene, shown);
  }, []);

  /**
   * Sends the live values that are waiting now, without the 60 ms wait: the page is going away or
   * the scene is. They go through the same queue as every live write, one at a time, and their
   * answers are kept. The entry leaves the queue once it has nothing left to send.
   */
  const flushLive = useCallback((g: GroupLive) => {
    g.hurry = true;
    if (g.timer) {
      clearTimeout(g.timer);
      g.timer = null;
    }
    void sendLiveRef.current(liveKey(g.sceneId, g.groupId));
  }, []);

  /**
   * Sends the live values of one scene now, without the 60 ms wait, and waits until they have
   * landed.
   *
   * A move carries the revision these writes leave, so it may not go before them: an edit made
   * inside the throttle window would otherwise be sent to a scene that no longer holds the
   * group, and the daemon would answer 404 and the edit would be lost.
   */
  const flushLiveScene = useCallback(
    async (sceneId: string) => {
      for (;;) {
        const groups = [...live.current.values()].filter((g) => g.sceneId === sceneId);
        const waiting = groups.filter(
          (g) =>
            g.sending ||
            g.timer !== null ||
            g.queued.props !== undefined ||
            g.queued.brightness !== undefined,
        );
        if (waiting.length === 0) return;
        // The sends that are on their way are waited for. The wait is booked before the values
        // are sent, so an answer that lands at once is not missed.
        const landed = waiting.map((g) => waitLiveGroup(liveKey(g.sceneId, g.groupId)));
        // The values that wait for their turn go now, without the 60 ms wait.
        for (const g of groups) flushLive(g);
        await Promise.all(landed);
      }
    },
    [flushLive, waitLiveGroup],
  );

  useEffect(() => {
    mounted.current = true;
    // The page has to hold the answers once, even where nothing started the store's polling.
    void refreshScenes();
    void refreshLights();
    const groups = live.current;
    return () => {
      mounted.current = false;
      // The last live value goes out even when the page goes away.
      for (const g of [...groups.values()]) flushLive(g);
    };
  }, [flushLive]);

  useEffect(() => {
    known.current = { server, activeId };
    if (history.current.sceneId !== (activeId ?? "")) {
      // The live values of the scene that goes go to that scene, not to the new one.
      // The entries stay until their queue has drained: they are keyed by their scene, so they
      // never show on or ride on the scene that comes.
      for (const g of [...live.current.values()]) flushLive(g);
      const h = history.current;
      if (h.keep) {
        // The change that brought this scene about is on the undo stack: it is put back with an
        // Undo, so the stack keeps its steps. The steps of the scene that is gone cannot be
        // applied to the new one, so they go.
        h.past = h.past.filter((e) => e.command);
        h.future = h.future.filter((e) => e.command);
        h.sceneId = activeId ?? "";
        h.keep = false;
      } else {
        // One scene's history never outlives a switch to another scene.
        history.current = { sceneId: activeId ?? "", past: [], future: [] };
      }
      publish();
    }
  }, [server, activeId, publish, flushLive]);

  /** Sends the writes that have not gone out yet, oldest first. */
  const flush = useCallback(async () => {
    if (flushing.current) return;
    flushing.current = true;
    try {
      for (;;) {
        const entry = outgoing.current.entries().next();
        if (entry.done) return;
        const sceneId = entry.value[0];
        // The whole-scene write holds the daemon: no live write starts while it is on its
        // way, so the rev cannot move under the save.
        writing.current = true;
        // The live writes go first: they answer the whole scene, and the save carries their rev.
        if (liveBusy.current > 0) await waitForLive();
        // The pending write may have changed while we waited: a live write answered and gave
        // it the answered rev, or an edit landed. The newest one goes.
        const pending = outgoing.current.get(sceneId);
        if (!pending) {
          writing.current = false;
          continue;
        }
        const sent = pending.scene;
        const sentChanges = pending.changes;
        let answer: Scene | null = null;
        let failure: unknown = null;
        try {
          answer = await api.scenes.put(sent.id, { scene: body(sent), expected_rev: sent.rev });
        } catch (e) {
          failure = e;
        }
        writing.current = false;
        const now = outgoing.current.get(sceneId);
        if (!failure && answer) {
          adoptScene(answer);
          if (now && now !== pending) {
            // An edit came while the write was out: it goes again, on the answer.
            const later = now.changes.slice(sentChanges.length);
            outgoing.current.set(sceneId, { scene: withChanges(answer, later), changes: now.changes });
            publish();
            continue;
          }
          outgoing.current.delete(sceneId);
          settleQuiet(sceneId);
          error.current = null;
          publish();
          return;
        }
        if (failure instanceof ApiFailure && failure.status === 409) {
          // Another window saved first. The changes that were on their way are dropped; the ones
          // that came while the daemon was answering stay and go again on the daemon's scene.
          history.current = { ...history.current, past: [], future: [] };
          await refreshScenes();
          // The changes that came in the meantime are read after the read: an edit made while
          // the daemon was answering is not lost.
          const after = outgoing.current.get(sceneId);
          const later = after && after !== pending ? after.changes.slice(sentChanges.length) : [];
          const serverScene = getSnap().scenes?.scenes.find((s) => s.id === sceneId) ?? null;
          if (later.length > 0 && serverScene) {
            outgoing.current.set(sceneId, { scene: withChanges(serverScene, later), changes: later });
            // The kept edits get undo steps of their own, from the daemon's scene. The steps that
            // the edits made while the answer was read pushed are dropped: they start from the old
            // scene, and undoing one would take back the other window's change.
            history.current = { ...history.current, past: undoStepsFor(serverScene, later), future: [] };
            error.current = CONFLICT_MESSAGE;
            publish();
            continue;
          }
          outgoing.current.delete(sceneId);
          settleQuiet(sceneId);
          error.current = CONFLICT_MESSAGE;
          publish();
          return;
        }
        // Another failure: the write is dropped, and the undo stack forgets it, so the first
        // undo is not a no-op.
        const dropped = (now?.changes ?? sentChanges).map((c) => c.id);
        outgoing.current.delete(sceneId);
        settleQuiet(sceneId);
        history.current = {
          ...history.current,
          past: history.current.past.filter((e) => e.changeId === undefined || !dropped.includes(e.changeId)),
        };
        await refreshScenes();
        fail(messageOf(failure));
        return;
      }
    } finally {
      flushing.current = false;
      // A write that came while this loop was on its way goes now.
      if (outgoing.current.size > 0) void flush();
    }
  }, [adoptScene, body, fail, publish, settleQuiet, waitForLive]);

  /** Puts `next` on screen and on its way to the daemon. */
  const edit = useCallback(
    (change: (draft: Scene) => void, label: string, options?: EditOptions) => {
      const active = known.current.activeId;
      if (!active) return;
      const pending = outgoing.current.get(active);
      const current = pending?.scene ?? base();
      if (!current) return;
      const next = clone(current);
      change(next);
      const id = ++changeSeq.current;
      const entry: Change = { id, apply: change, label };
      outgoing.current.set(active, { scene: next, changes: [...(pending?.changes ?? []), entry] });
      // A preview is not the owner's decision: it goes to the lights, but the Undo stack keeps
      // the step before it, so an Undo never lands on a picture that was only a preview.
      if (!options?.transient) {
        const h = history.current;
        h.past.push({ scene: clone(current), label, changeId: id });
        if (h.past.length > HISTORY_DEPTH) h.past.shift();
        h.future = [];
      }
      publish();
      void flush();
    },
    [base, flush, publish],
  );

  /**
   * Puts a change to the scenes themselves on the undo stack: a scene was renamed, made,
   * duplicated or deleted. The change is put back by its own command, because the daemon has no
   * route that undoes it.
   */
  const pushCommand = useCallback(
    (command: Command, changesTheActive: boolean) => {
      const h = history.current;
      h.past.push({ label: command.label, command });
      if (h.past.length > HISTORY_DEPTH) h.past.shift();
      h.future = [];
      // The change may make another scene play: the history keeps its steps, so the change can be
      // put back.
      if (changesTheActive) h.keep = true;
      publish();
    },
    [publish],
  );

  const undo = useCallback(() => {
    const h = history.current;
    const active = known.current.activeId;
    if (!active) return;
    const previous = h.past.pop();
    if (!previous) return;
    h.future.push(previous);
    if (previous.command) {
      // A change to the scenes themselves: it is put back by its own command. The active scene
      // may change under it, so the history keeps its steps.
      h.keep = true;
      const command = previous.command;
      // The command is waited for: a command that could not put the world back says so, and its
      // step is dropped (the future forgets it), so the next Undo is the one before it.
      void (async () => {
        const done = await command.undo();
        if (done === false) {
          const now = history.current;
          history.current = { ...now, future: now.future.filter((e) => e !== previous) };
        }
        publish();
      })();
      publish();
      return;
    }
    const pending = outgoing.current.get(active);
    const current = pending?.scene ?? base();
    if (!current || !previous.scene) return;
    // The undo is a change of its own: it puts the older scene back.
    const id = ++changeSeq.current;
    const entry: Change = {
      id,
      label: previous.label,
      apply: (d) => {
        Object.assign(d, clone(previous.scene!), { rev: d.rev });
      },
    };
    outgoing.current.set(active, {
      scene: withChanges(current, [entry]),
      changes: [...(pending?.changes ?? []), entry],
    });
    publish();
    void flush();
  }, [base, flush, publish]);

  const redo = useCallback(() => {
    const h = history.current;
    const active = known.current.activeId;
    if (!active) return;
    const next = h.future.pop();
    if (!next) return;
    h.past.push(next);
    if (next.command) {
      // A change to the scenes themselves: it is done again by its own command.
      h.keep = true;
      const command = next.command;
      void (async () => {
        const done = await command.redo();
        if (done === false) {
          const now = history.current;
          history.current = { ...now, past: now.past.filter((e) => e !== next) };
        }
        publish();
      })();
      publish();
      return;
    }
    const pending = outgoing.current.get(active);
    const current = pending?.scene ?? base();
    if (!current || !next.scene) return;
    const id = ++changeSeq.current;
    const entry: Change = {
      id,
      label: next.label,
      apply: (d) => {
        Object.assign(d, clone(next.scene!), { rev: d.rev });
      },
    };
    outgoing.current.set(active, {
      scene: withChanges(current, [entry]),
      changes: [...(pending?.changes ?? []), entry],
    });
    publish();
    void flush();
  }, [base, flush, publish]);

  /** The newest rev of one scene this window knows: the snapshot's, or the one a write answered with. */
  const knownRev = useCallback((sceneId: string): number | null => {
    const fromSnap = getSnap().scenes?.scenes.find((s) => s.id === sceneId)?.rev ?? null;
    const fromSaved = saved.current && saved.current.id === sceneId ? saved.current.rev : null;
    if (fromSnap === null) return fromSaved;
    if (fromSaved === null) return fromSnap;
    return Math.max(fromSnap, fromSaved);
  }, []);

  /** The values on screen are the daemon's once its answer is in, unless the user moved on since. */
  const settleShown = useCallback((sceneId: string, groupId: string, kind: LiveKind, version: number) => {
    const g = live.current.get(liveKey(sceneId, groupId));
    if (g && g.version[kind] === version) {
      if (kind === "props") delete g.shown.props;
      else delete g.shown.brightness;
    }
  }, []);

  /**
   * A live write answered at a rev other than the one after the rev it was sent on: another window
   * saved in between. It is a 409 on a whole-scene save. The answer is the daemon's scene, taken as
   * local state; the unsent draft of that scene was made from the older scene, so it is dropped and
   * not sent on top; the undo history starts over.
   */
  const liveConflict = useCallback(
    (answer: Scene, groupId: string, kind: LiveKind, version: number) => {
      outgoing.current.delete(answer.id);
      const h = history.current;
      if (h.sceneId === answer.id) history.current = { ...h, past: [], future: [] };
      adoptScene(answer);
      settleShown(answer.id, groupId, kind, version);
      error.current = CONFLICT_MESSAGE;
      publish();
    },
    [adoptScene, publish, settleShown],
  );

  /** Takes the daemon's answer for a live write: the scene, its rev, and the group's values. */
  const adoptLive = useCallback(
    (answer: Scene, groupId: string, kind: LiveKind, version: number, written: Props | number) => {
      adoptScene(answer);
      // The pending write carries the answered rev and the answered values, so the next
      // whole-scene save neither 409s against this write nor reverts it.
      const pending = outgoing.current.get(answer.id);
      if (pending) {
        outgoing.current.set(answer.id, {
          ...pending,
          scene: adoptGroup(pending.scene, answer, groupId, kind, written),
        });
      }
      settleShown(answer.id, groupId, kind, version);
    },
    [adoptScene, settleShown],
  );

  /** Takes a group's entry out of the queue once nothing of it is on its way or waiting. */
  const retireIfIdle = useCallback(
    (key: string, g: GroupLive) => {
      if (g.sending || g.timer || g.queued.props !== undefined || g.queued.brightness !== undefined) return;
      g.hurry = false;
      if (live.current.get(key) === g) {
        live.current.delete(key);
        publish();
      }
    },
    [publish],
  );

  /** Starts the next live write of every group that has one waiting. */
  const kickLive = useCallback(() => {
    for (const groupId of [...live.current.keys()]) void sendLiveRef.current(groupId);
  }, []);

  /**
   * Reads one scene straight from its route, for a live write whose outcome is unknown. It is not
   * the store's read: a newer poll cannot drop its answer, a daemon that answered 404 for the
   * scene routes cannot skip it, and the page's own snapshot is never taken for the daemon's
   * word. It gives up on its own after [`LIVE_READ_TIMEOUT_MS`], and answers `null` when it did
   * not read the scene.
   */
  const readScene = useCallback(async (sceneId: string): Promise<Scene | null> => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const giveUp = new Promise<null>((resolve) => {
      timer = setTimeout(() => resolve(null), LIVE_READ_TIMEOUT_MS);
    });
    try {
      const answer = await Promise.race([api.scenes.list(), giveUp]);
      return answer?.scenes.find((s) => s.id === sceneId) ?? null;
    } catch {
      return null;
    } finally {
      if (timer) clearTimeout(timer);
    }
  }, []);

  /**
   * Reads the scene after a live write whose outcome is unknown, and says what became of the
   * write. A write can reach the daemon and be saved while its answer is lost (a dropped
   * connection, a deadline): taking the next answer for another window's save would drop the
   * user's unsent draft, and taking it for this write's own would drop the draft just the same.
   */
  const afterLostAnswer = useCallback(
    async (
      sceneId: string,
      groupId: string,
      kind: LiveKind,
      sentRev: number | null,
      written: Props | number,
    ): Promise<LostAnswer> => {
      if (sentRev === null) return null;
      const scene = await readScene(sceneId);
      // The read did not answer, or answered from the scene the write was sent on: the write did
      // not land, and the retry is an ordinary one.
      if (!scene || scene.rev <= sentRev) return null;
      // The rev moved more than this write would move it, or the group holds another value for
      // what was sent: another window saved in between.
      const group = scene.groups.find((g) => g.id === groupId);
      if (scene.rev > sentRev + 1 || !group || !holdsValue(group, kind, written)) return { conflict: scene };
      return { landed: scene };
    },
    [readScene],
  );

  /** Sends the live values of one group that are waiting. */
  const sendLive = useCallback(
    async (key: string) => {
      const g = live.current.get(key);
      if (!g) return;
      const groupId = g.groupId;
      // One write at a time for this group: a second one would race the first.
      if (g.sending) return;
      // The group's turn is already booked.
      if (g.timer) return;
      if (g.queued.props === undefined && g.queued.brightness === undefined) {
        retireIfIdle(key, g);
        settleLiveGroup(key);
        return;
      }
      if (writing.current) {
        // A whole-scene save is on its way. It carries the rev of the last answer, and a live
        // write now would answer a scene one rev behind it. The value waits.
        g.timer = setTimeout(() => {
          g.timer = null;
          void sendLiveRef.current(key);
        }, LIVE_RETRY_MS);
        return;
      }
      // One live write of the scene at a time, across the groups: each answer is checked against
      // the rev its write was sent on, and that rev is only known once the write before it has
      // answered. The answer that lands starts the waiting groups (`kickLive`).
      if (liveBusy.current > 0) return;
      const now = Date.now();
      const ready = (kind: LiveKind): boolean =>
        g.queued[kind] !== undefined && (g.hurry || now - g.sentAt[kind] >= LIVE_THROTTLE_MS);
      if (!ready("props") && !ready("brightness")) {
        // The window of one kind is not over yet: wait for the earliest one.
        const waits: number[] = [];
        if (g.queued.props !== undefined) waits.push(LIVE_THROTTLE_MS - (now - g.sentAt.props));
        if (g.queued.brightness !== undefined) waits.push(LIVE_THROTTLE_MS - (now - g.sentAt.brightness));
        const wait = Math.max(0, Math.min(...waits));
        g.timer = setTimeout(() => {
          g.timer = null;
          void sendLiveRef.current(key);
        }, wait);
        return;
      }
      const kind: LiveKind = ready("props") ? "props" : "brightness";
      const props = g.queued.props;
      const brightness = g.queued.brightness;
      if (kind === "props") g.queued.props = undefined;
      else g.queued.brightness = undefined;
      // The values that were waiting when the page or the scene went are sent; later ones are paced.
      if (g.queued.props === undefined && g.queued.brightness === undefined) g.hurry = false;
      g.sentAt[kind] = now;
      const version = g.version[kind];
      const written = kind === "props" ? (props as Props) : (brightness as number);
      // The rev this write is sent on: the unsent draft's when there is one (the next save would
      // carry that rev), else the newest rev this window knows.
      const sentRev = outgoing.current.get(g.sceneId)?.scene.rev ?? knownRev(g.sceneId);
      g.sending = true;
      liveBusy.current += 1;
      let failure: unknown = null;
      try {
        const answer =
          kind === "props"
            ? await api.scenes.patchGroupProps(g.sceneId, groupId, props as Props)
            : await api.scenes.putGroupBrightness(g.sceneId, groupId, brightness as number);
        if (sentRev !== null && answer.rev === sentRev + 1) {
          adoptLive(answer, groupId, kind, version, written);
        } else {
          liveConflict(answer, groupId, kind, version);
        }
      } catch (e) {
        failure = e;
      }
      // A write whose outcome is unknown may have landed all the same: no answer at all, or a 5xx.
      // The scene says which, and it is read before the live write is let go (bounded by the
      // read's own deadline), so no whole-scene save can move the rev while it is out.
      const lost =
        failure && outcomeUnknown(failure)
          ? await afterLostAnswer(g.sceneId, groupId, kind, sentRev, written)
          : null;
      g.sending = false;
      liveBusy.current -= 1;
      if (liveBusy.current === 0) {
        const waiters = liveWaiters.current;
        liveWaiters.current = [];
        for (const resolve of waiters) resolve();
      }
      if (!failure) {
        g.failures[kind] = 0;
        publish();
      } else if (lost && "landed" in lost) {
        // The write landed; only its answer was lost. It is taken as any other answer is: the
        // scene, its rev and the value go to the draft, and the value is not sent again.
        g.failures[kind] = 0;
        adoptLive(lost.landed, groupId, kind, version, written);
        error.current = null;
        publish();
      } else if (lost && "conflict" in lost) {
        liveConflict(lost.conflict, groupId, kind, version);
      } else {
        // The write did not land. The value that came while it was out still goes: a failed
        // write must not take the next value with it.
        g.failures[kind] += 1;
        // A page or a scene that is going away asks for the values without the 60 ms wait. A
        // refused write must not answer that by sending its retries back to back: they wait
        // their throttle like any other write.
        g.hurry = false;
        if (g.failures[kind] < LIVE_FAILURES) {
          // Try again after the throttle, with the value that did not land, unless the user has
          // moved on since (then the newer value is the one that goes).
          if (kind === "props") {
            if (g.version.props === version && g.queued.props === undefined) g.queued.props = written as Props;
          } else if (g.version.brightness === version && g.queued.brightness === undefined) {
            g.queued.brightness = written as number;
          }
        } else {
          // Three tries in a row failed. The value that failed leaves the queue and the screen, so
          // nothing shows as saved that is not, and no later save carries it. A value the user
          // moved on to since was never tried, and the other kind was not tried either: both stay,
          // with tries of their own. The error stays.
          g.failures[kind] = 0;
          if (g.version[kind] === version) {
            delete g.queued[kind];
            delete g.shown[kind];
          }
        }
        fail(messageOf(failure));
      }
      // The values that came while this write was out go next, one write at a time.
      kickLive();
      retireIfIdle(key, g);
      // Nothing of this group is left to send: whoever waits for it may go on.
      if (!g.sending && !g.timer && g.queued.props === undefined && g.queued.brightness === undefined) {
        settleLiveGroup(key);
      }
    },
    [
      adoptLive,
      afterLostAnswer,
      fail,
      knownRev,
      kickLive,
      liveConflict,
      publish,
      retireIfIdle,
      settleLiveGroup,
    ],
  );

  useEffect(() => {
    sendLiveRef.current = sendLive;
  }, [sendLive]);

  /** Books the live write of one group: the value goes now, or when its turn comes. */
  const queueLive = useCallback(
    (groupId: string, kind: LiveKind, value: Props | number) => {
      const scene = base();
      if (!scene) return;
      const key = liveKey(scene.id, groupId);
      let g = live.current.get(key);
      if (!g) {
        const now = Date.now();
        g = {
          sceneId: scene.id,
          groupId,
          shown: {},
          queued: {},
          version: { props: 0, brightness: 0 },
          sentAt: { props: now, brightness: now },
          failures: { props: 0, brightness: 0 },
          sending: false,
          timer: null,
          hurry: false,
        };
        live.current.set(key, g);
      }
      // The user moved this kind again: it tries again, whatever came before. The other kind's
      // failures are its own.
      g.failures[kind] = 0;
      g.version[kind] += 1;
      if (kind === "props") {
        const props = value as Props;
        g.shown.props = { ...g.shown.props, ...props };
        g.queued.props = { ...g.queued.props, ...props };
      } else {
        const brightness = value as number;
        g.shown.brightness = brightness;
        g.queued.brightness = brightness;
      }
      publish();
      // The write goes when the group's turn comes; the last value always goes.
      void sendLiveRef.current(key);
    },
    [base, publish],
  );

  const setProps = useCallback(
    (groupId: string, props: Props) => {
      queueLive(groupId, "props", props);
    },
    [queueLive],
  );

  const setBrightness = useCallback(
    (groupId: string, n: number) => {
      const value = Math.max(0, Math.min(100, Math.round(n)));
      queueLive(groupId, "brightness", value);
    },
    [queueLive],
  );

  /** Runs one scene call, says what went wrong, and reads the scenes again either way. */
  const run = useCallback(
    (call: () => Promise<unknown>) => {
      void (async () => {
        try {
          await call();
          fail(null);
        } catch (e) {
          fail(messageOf(e));
        }
        await refreshScenes();
      })();
    },
    [fail],
  );

  /** The same, for a call whose answer is waited for: a command's undo and redo. */
  const runAsync = useCallback(
    async (call: () => Promise<unknown>) => {
      try {
        await call();
        fail(null);
      } catch (e) {
        fail(messageOf(e));
      }
      await refreshScenes();
    },
    [fail],
  );

  const setActive = useCallback((id: string) => run(() => api.scenes.setActive(id)), [run]);

  /** Puts a name on a scene. The scene's revision is read again, so the write is never stale. */
  const putName = useCallback(async (id: string, name: string) => {
    const answer = await api.scenes.list();
    const scene = answer.scenes.find((s) => s.id === id);
    if (!scene) throw new ApiFailure(404, "not_found", `there is no scene with the id ${id}`);
    await api.scenes.put(id, { scene: { ...scene, name }, expected_rev: scene.rev });
  }, []);

  /**
   * A new scene, and it plays: the owner made it to use it, not to keep it on a shelf. The daemon
   * makes the scene but leaves the old one playing, so the new one is set here. The change is on
   * the undo stack: an Undo takes the new scene away.
   */
  const create = useCallback(
    (name: string, copyOf?: string) => {
      let madeId: string | null = null;
      const make = async () => {
        const made = await api.scenes.create(copyOf ? { name, copy_of: copyOf } : { name });
        madeId = made.id;
        await api.scenes.setActive(made.id);
      };
      pushCommand(
        {
          label: copyOf ? "Duplicate the scene" : "Make a scene",
          undo: () =>
            runAsync(async () => {
              if (madeId) await api.scenes.remove(madeId);
            }),
          redo: () => runAsync(make),
        },
        true,
      );
      run(make);
    },
    [pushCommand, run, runAsync],
  );

  /**
   * A scene goes. The change is on the undo stack: an Undo makes the scene again, with its
   * groups, and plays it again if it was the one that played.
   */
  const remove = useCallback(
    (id: string) => {
      const answer = getSnap().scenes;
      const scene = answer?.scenes.find((s) => s.id === id);
      if (!scene) return;
      const wasActive = answer?.active === id;
      let currentId = id;
      const restore = async () => {
        // The daemon has no route that makes a scene with these groups: a new scene is made with
        // the name, and the groups are put into it.
        const made = await api.scenes.create({ name: scene.name });
        const saved = await api.scenes.put(made.id, {
          scene: { ...scene, id: made.id },
          expected_rev: made.rev,
        });
        currentId = saved.id;
        if (wasActive) await api.scenes.setActive(saved.id);
      };
      pushCommand(
        {
          label: "Delete the scene",
          undo: () => runAsync(restore),
          redo: () =>
            runAsync(async () => {
              await api.scenes.remove(currentId);
            }),
        },
        true,
      );
      run(() => api.scenes.remove(id));
    },
    [pushCommand, run, runAsync],
  );
  /**
   * A scene's own name. The scene need not be the active one, so this goes to the scene's own
   * route with the revision it was read at; a scene another window changed in between is refused
   * and the name stays as it was. The change is on the undo stack: an Undo puts the old name back.
   */
  const renameScene = useCallback(
    (id: string, name: string) => {
      const clean = name.trim();
      if (!clean) return;
      const scene = getSnap().scenes?.scenes.find((s) => s.id === id);
      if (!scene || scene.name === clean) return;
      pushCommand(
        {
          label: "Rename the scene",
          undo: () => runAsync(() => putName(id, scene.name)),
          redo: () => runAsync(() => putName(id, clean)),
        },
        false,
      );
      run(() => putName(id, clean));
    },
    [pushCommand, putName, run, runAsync],
  );

  /**
   * Puts the scenes the daemon has back, after checking that nothing else touched them.
   *
   * The scenes are read again first: an undo that wrote a remembered rev would be refused by
   * this window's own later edit, and one that wrote over another window's change would lose it.
   * `write` is what goes back, `expect` is what the scene must hold now; when they differ, the
   * undo says so and drops itself.
   */
  const restoreScenes = useCallback(
    async (write: Scene[], expect: Scene[], ids: string[]): Promise<boolean> => {
      let answer: ScenesResponse;
      try {
        answer = await api.scenes.list();
      } catch (e) {
        fail(messageOf(e));
        return false;
      }
      const writes: SceneWrite[] = [];
      for (const id of ids) {
        const now = answer.scenes.find((s) => s.id === id);
        const wanted = expect.find((s) => s.id === id);
        const back = write.find((s) => s.id === id);
        // A scene that is gone, or that another window changed since, is not written over. The
        // name counts as part of the scene: an undo that wrote the old name back would take a
        // rename another window made away.
        if (!now || !wanted || !back || !sameGroups(now.groups, wanted.groups) || now.name !== wanted.name) {
          fail(NOTHING_UNDONE);
          return false;
        }
        writes.push({ scene: back, expected_rev: now.rev });
      }
      try {
        const saved = await api.scenes.write({ scenes: writes });
        adoptScenes(saved.scenes);
        fail(null);
        return true;
      } catch (e) {
        // A 409 means another window changed the scene; anything else is the daemon's own
        // refusal. Both drop the undo and say what the daemon said.
        if (e instanceof ApiFailure && e.status === 409) fail(NOTHING_UNDONE);
        else fail(messageOf(e));
        return false;
      } finally {
        await refreshScenes();
      }
    },
    [adoptScenes, fail],
  );

  /**
   * Asks the daemon what a move would do, and writes nothing (#368).
   *
   * The daemon's own dry run is the answer: its rules, its refusals and its words. The page does
   * not work them out a second time, so a move can never be allowed here and refused there.
   */
  const dryRunMove = useCallback(
    async (groupId: string, toSceneId: string, copy: boolean): Promise<MoveCheck> => {
      const fromId = known.current.activeId;
      if (!fromId) return { state: "failed" };
      // The live edits of this scene's groups go first: the answer must be about the scene as it
      // will be, and the move that follows carries the revision they leave.
      await flushLiveScene(fromId);
      await waitQuiet(fromId);
      await waitQuiet(toSceneId);
      const fromRev = knownRev(fromId);
      const toRev = knownRev(toSceneId);
      if (fromRev === null || toRev === null) return { state: "failed" };
      try {
        const answer = await api.scenes.moveGroup(fromId, groupId, {
          to: toSceneId,
          copy,
          expected_rev_from: fromRev,
          expected_rev_to: toRev,
          dry_run: true,
        });
        return { state: "answered", answer };
      } catch (e) {
        // The daemon's own refusal (422) is shown on the row, in its own words.
        if (e instanceof ApiFailure && e.status === 422) {
          return { state: "refused", message: e.message };
        }
        // Another window saved first: the scenes are read again, so a retry asks with the
        // revision the daemon has now.
        if (e instanceof ApiFailure && e.status === 409) await refreshScenes();
        return { state: "failed" };
      }
    },
    [flushLiveScene, knownRev, refreshScenes, waitQuiet],
  );

  /**
   * Moves or copies one group to another scene (#368).
   *
   * The daemon's one route writes both scenes in one save and answers what it did: the scenes
   * after, the scenes before, the lights it took and the groups it removed. The undo puts both
   * scenes back through `PUT /api/v1/scenes`, after reading them again.
   *
   * `expected` names the revisions the move is made from: the ones a dry run answered with, so
   * the move that follows a dry run carries the same ones.
   */
  const moveGroup = useCallback(
    async (
      groupId: string,
      toSceneId: string,
      copy: boolean,
      expected?: { from: number; to: number },
    ): Promise<GroupMoved | null> => {
      const fromId = known.current.activeId;
      if (!fromId) return null;
      // The names the toast says: the group that goes and the scene it goes to. They are read
      // before the move, because the move takes the group out of the scene it was in.
      const held = getSnap().scenes?.scenes ?? [];
      const groupName =
        held.find((s) => s.id === fromId)?.groups.find((g) => g.id === groupId)?.name ?? "the group";
      const sceneName = held.find((s) => s.id === toSceneId)?.name ?? "the scene";
      // The live edits of this scene's groups go first: the move carries the revision they
      // leave, and an edit sent after the move would go to a scene that no longer holds the
      // group.
      await flushLiveScene(fromId);
      // A whole-scene write of either scene that is on its way goes first: the move carries the
      // revision that write left.
      await waitQuiet(fromId);
      await waitQuiet(toSceneId);
      const fromRev = expected?.from ?? knownRev(fromId);
      const toRev = expected?.to ?? knownRev(toSceneId);
      if (fromRev === null || toRev === null) {
        fail("The scenes could not be read.");
        return null;
      }
      let answer: GroupMoved;
      try {
        answer = await api.scenes.moveGroup(fromId, groupId, {
          to: toSceneId,
          copy,
          expected_rev_from: fromRev,
          expected_rev_to: toRev,
        });
      } catch (e) {
        // Another window saved first: nothing moved, and the page says so.
        if (e instanceof ApiFailure && e.status === 409) fail(NOTHING_MOVED);
        else fail(messageOf(e));
        await refreshScenes();
        return null;
      }
      adoptScenes(answer.scenes);
      fail(null);
      // The undo: the scenes the move left are read again and compared, and the ones before are
      // written back. A copy leaves the source as it was, so the source is not written.
      const ids = copy ? [toSceneId] : [fromId, toSceneId];
      pushCommand(
        {
          label: copy ? COPY_STEP_LABEL : MOVE_STEP_LABEL,
          undo: () => restoreScenes(answer.before, answer.scenes, ids),
          redo: () => restoreScenes(answer.scenes, answer.before, ids),
        },
        false,
      );
      // The toast: what happened, and the Undo while the move is the top step of the history.
      const emptied = moveEmptiedMessage(answer.emptied);
      const first = `${copy ? "Copied" : "Moved"} ${groupName} to ${sceneName}.`;
      const text = emptied ? `${first} ${emptied}` : first;
      const tone = emptied ? "warn" : "ok";
      const label = copy ? COPY_STEP_LABEL : MOVE_STEP_LABEL;
      lastMove.current = { text, tone, label };
      undoOffered.current = true;
      notify(
        text,
        tone,
        {
          label: "Undo",
          onClick: () => {
            // The toast goes with its answer: it is not said again.
            lastMove.current = null;
            undoOffered.current = false;
            undo();
          },
        },
        // The reader takes the toast away: the move is forgotten, so no later change says it
        // again.
        { onClose: () => { lastMove.current = null; undoOffered.current = false; } },
      );
      await refreshScenes();
      return answer;
    },
    [
      adoptScenes,
      fail,
      flushLiveScene,
      knownRev,
      pushCommand,
      refreshScenes,
      restoreScenes,
      undo,
      waitQuiet,
    ],
  );

  /**
   * The toast of the last move is said again when the Undo it offers is no longer the one: an
   * Undo takes the top step of the history back, so it may only be offered while the move is
   * still on top. When another change came after the move, the toast says what happened and
   * offers nothing.
   *
   * The toast is said once: it is said again only when the Undo it offers changes, so a change
   * that comes later does not bring back a toast that has gone.
   */
  useEffect(() => {
    const move = lastMove.current;
    if (!move) return;
    const top = view.undoLabel === move.label;
    if (top === undoOffered.current) return;
    undoOffered.current = top;
    notify(
      move.text,
      move.tone,
      top
        ? {
            label: "Undo",
            onClick: () => {
              lastMove.current = null;
              undoOffered.current = false;
              undo();
            },
          }
        : undefined,
      // The reader takes the toast away: the move is forgotten, so no later change says it
      // again.
      { onClose: () => { lastMove.current = null; undoOffered.current = false; } },
    );
  }, [undo, view.undoLabel]);

  /**
   * Shows an order of this window while it is on its way, and forgets it when it is gone. The ref
   * keeps the same order for the callbacks that run between renders.
   */
  const showLocalOrder = useCallback((order: string[] | null) => {
    localOrderRef.current = order;
    setLocalOrder(order);
  }, []);

  /**
   * The daemon is told the order of the scenes, with the order it last confirmed. A window that
   * put the scenes in another order in between is refused, and nothing is written.
   */
  const writeOrder = useCallback(
    async (order: string[], expected: string[]): Promise<boolean> => {
      try {
        const answer = await api.scenes.write({ order, expected_order: expected });
        adoptScenes(answer.scenes);
        knownOrder.current = order;
        fail(null);
        await refreshScenes();
        return true;
      } catch (e) {
        if (e instanceof ApiFailure && e.status === 409) fail(NOTHING_MOVED);
        else fail(messageOf(e));
        await refreshScenes();
        return false;
      }
    },
    [adoptScenes, fail],
  );

  /** The undo of an order: the old order goes back, with the new one as the order it was made from. */
  const pushOrderCommand = useCallback(
    (from: string[], to: string[]) => {
      pushCommand(
        {
          label: "Put the scenes in another order",
          undo: async () => {
            showLocalOrder(from);
            const done = await writeOrder(from, to);
            showLocalOrder(null);
            return done;
          },
          redo: async () => {
            showLocalOrder(to);
            const done = await writeOrder(to, from);
            showLocalOrder(null);
            return done;
          },
        },
        false,
      );
    },
    [pushCommand, showLocalOrder, writeOrder],
  );

  /**
   * Puts the scenes in another order (#371).
   *
   * The order shows at once; the daemon is told in one write, with the order it last confirmed.
   * A burst of key presses is one order and one write (`coalesce`), so the screen shows every
   * press at once and the daemon hears one order.
   *
   * Two orders that come one after the other go one after the other as well: the second one is
   * sent after the first has landed and names the order the first one sent. The daemon refuses
   * a stale order, and the order this window sent is the one the daemon has.
   */
  const reorderScenes = useCallback(
    (order: string[], options?: { coalesce?: boolean }) => {
      if (knownOrder.current.length === 0 && (getSnap().scenes?.scenes.length ?? 0) === 0) return;
      showLocalOrder(order);
      const send = () => {
        const run = async () => {
          try {
            const expected = knownOrder.current.length
              ? knownOrder.current
              : (getSnap().scenes?.scenes.map((s) => s.id) ?? []);
            if (expected.length === 0) return;
            const done = await writeOrder(order, expected);
            if (!done) {
              showLocalOrder(null);
              return;
            }
            pushOrderCommand(expected, order);
            // The daemon's answer is the order now. When it is the order on the screen, the strip
            // follows the daemon again, so the order another window makes shows as soon as it
            // lands.
            const landed = (getSnap().scenes?.scenes ?? []).map((s) => s.id);
            const local = localOrderRef.current;
            if (local && landed.join("|") === local.join("|")) showLocalOrder(null);
          } catch {
            showLocalOrder(null);
          }
        };
        orderChain.current = orderChain.current.then(run, run);
      };
      if (options?.coalesce) {
        if (orderTimer.current) clearTimeout(orderTimer.current);
        orderTimer.current = setTimeout(() => {
          orderTimer.current = null;
          send();
        }, ORDER_COALESCE_MS);
        return;
      }
      send();
    },
    [pushOrderCommand, showLocalOrder, writeOrder],
  );

  // The order the daemon last confirmed. While a reorder of this window is on its way the
  // daemon's answer is still the old order: a write names the order it was made from, not the
  // one on the screen.
  useEffect(() => {
    const order = scenesAnswer?.scenes.map((s) => s.id) ?? [];
    if (order.length === 0 || localOrder) return;
    knownOrder.current = order;
  }, [scenesAnswer, localOrder]);

  // The scene on screen: what this window changed, else the newest the daemon has, with the
  // live values laid over the group they belong to.
  const shown = view.draft ?? newest(server, view.saved, activeId);
  const scene = useMemo(() => overlay(shown, view.live), [shown, view.live]);

  /** The scenes as the page shows them: the daemon's order, or the page's own while a reorder is on its way. */
  const scenes = useMemo(() => {
    const all = scenesAnswer?.scenes ?? EMPTY_SCENES;
    if (!localOrder) return all;
    const byId = new Map(all.map((s) => [s.id, s]));
    const ordered: Scene[] = [];
    for (const id of localOrder) {
      const scene = byId.get(id);
      if (scene) ordered.push(scene);
    }
    // A scene this window has not seen yet (made elsewhere) keeps its place at the end.
    for (const other of all) if (!localOrder.includes(other.id)) ordered.push(other);
    return ordered;
  }, [scenesAnswer, localOrder]);

  const unassigned = useMemo(() => {
    if (!lights) return EMPTY_LIGHTS;
    const held = new Set((shown?.groups ?? []).flatMap((g) => g.members));
    return lights.filter((l) => !held.has(l.light.id));
  }, [lights, shown]);

  const ownsHistory = view.historyScene === (activeId ?? "");
  return {
    scene,
    scenes,
    unassigned,
    states: scenesAnswer?.group_states ?? EMPTY_STATES,
    edit,
    setProps,
    setBrightness,
    setActive,
    create,
    remove,
    renameScene,
    dryRunMove,
    moveGroup,
    reorderScenes,
    undo,
    redo,
    canUndo: ownsHistory && view.canUndo,
    canRedo: ownsHistory && view.canRedo,
    undoLabel: ownsHistory ? view.undoLabel : null,
    error: view.error,
  };
}
