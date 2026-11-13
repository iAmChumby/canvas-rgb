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
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { getSnap, patchSnap, refreshLights, refreshScenes, useSnap } from "@/lib/store";
import type { Group } from "@/proto/Group";
import type { GroupState } from "@/proto/GroupState";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
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

/** One prop value as text, so two of them compare by what they say and not by their key order. */
function propText(value: PropValue): string {
  return `${value.type}:${String(value.value)}`;
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

/** One step of the undo history. */
interface Entry {
  /** The scene before the change. */
  scene: Scene;
  /** What the change was called. */
  label: string;
  /** The change this step undoes, when it was one this window made. */
  changeId?: number;
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
  historyScene: "",
  live: {},
};

export interface UseScene {
  scene: Scene | null;
  scenes: Scene[];
  /** Lights no group of the active scene holds. */
  unassigned: LightView[];
  /** How each group of the active scene is doing. */
  states: GroupState[];
  /** Applies `change` to a copy of the scene at once, then saves the whole scene. `label` names the change for the page. */
  edit(change: (draft: Scene) => void, label: string): void;
  setProps(groupId: string, props: Props): void;
  setBrightness(groupId: string, n: number): void;
  setActive(id: string): void;
  create(name: string, copyOf?: string): void;
  remove(id: string): void;
  undo(): void;
  redo(): void;
  canUndo: boolean;
  canRedo: boolean;
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
  const history = useRef({ sceneId: "", past: [] as Entry[], future: [] as Entry[] });
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
    // One scene's history never outlives a switch to another scene.
    if (history.current.sceneId !== (activeId ?? "")) {
      // The live values of the scene that goes go to that scene, not to the new one.
      // The entries stay until their queue has drained: they are keyed by their scene, so they
      // never show on or ride on the scene that comes.
      for (const g of [...live.current.values()]) flushLive(g);
      // A write that has not gone out yet stays in the queue: it belongs to the scene that is
      // going, and it goes out there.
      history.current = { sceneId: activeId ?? "", past: [], future: [] };
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
          error.current = CONFLICT_MESSAGE;
          publish();
          return;
        }
        // Another failure: the write is dropped, and the undo stack forgets it, so the first
        // undo is not a no-op.
        const dropped = (now?.changes ?? sentChanges).map((c) => c.id);
        outgoing.current.delete(sceneId);
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
  }, [adoptScene, body, fail, publish, waitForLive]);

  /** Puts `next` on screen and on its way to the daemon. */
  const edit = useCallback(
    (change: (draft: Scene) => void, label: string) => {
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
      const h = history.current;
      h.past.push({ scene: clone(current), label, changeId: id });
      if (h.past.length > HISTORY_DEPTH) h.past.shift();
      h.future = [];
      publish();
      void flush();
    },
    [base, flush, publish],
  );

  const undo = useCallback(() => {
    const h = history.current;
    const active = known.current.activeId;
    if (!active) return;
    const pending = outgoing.current.get(active);
    const current = pending?.scene ?? base();
    const previous = h.past.pop();
    if (!current || !previous) return;
    h.future.push({ scene: clone(current), label: previous.label });
    // The undo is a change of its own: it puts the older scene back.
    const id = ++changeSeq.current;
    const entry: Change = {
      id,
      label: previous.label,
      apply: (d) => {
        Object.assign(d, clone(previous.scene), { rev: d.rev });
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
    const pending = outgoing.current.get(active);
    const current = pending?.scene ?? base();
    const next = h.future.pop();
    if (!current || !next) return;
    const id = ++changeSeq.current;
    const entry: Change = {
      id,
      label: next.label,
      apply: (d) => {
        Object.assign(d, clone(next.scene), { rev: d.rev });
      },
    };
    h.past.push({ scene: clone(current), label: next.label, changeId: id });
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
    },
    [adoptLive, afterLostAnswer, fail, knownRev, kickLive, liveConflict, publish, retireIfIdle],
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

  const setActive = useCallback((id: string) => run(() => api.scenes.setActive(id)), [run]);
  const create = useCallback(
    (name: string, copyOf?: string) => run(() => api.scenes.create(copyOf ? { name, copy_of: copyOf } : { name })),
    [run],
  );
  const remove = useCallback((id: string) => run(() => api.scenes.remove(id)), [run]);

  // The scene on screen: what this window changed, else the newest the daemon has, with the
  // live values laid over the group they belong to.
  const shown = view.draft ?? newest(server, view.saved, activeId);
  const scene = useMemo(() => overlay(shown, view.live), [shown, view.live]);

  const unassigned = useMemo(() => {
    if (!lights) return EMPTY_LIGHTS;
    const held = new Set((shown?.groups ?? []).flatMap((g) => g.members));
    return lights.filter((l) => !held.has(l.light.id));
  }, [lights, shown]);

  const ownsHistory = view.historyScene === (activeId ?? "");
  return {
    scene,
    scenes: scenesAnswer?.scenes ?? EMPTY_SCENES,
    unassigned,
    states: scenesAnswer?.group_states ?? EMPTY_STATES,
    edit,
    setProps,
    setBrightness,
    setActive,
    create,
    remove,
    undo,
    redo,
    canUndo: ownsHistory && view.canUndo,
    canRedo: ownsHistory && view.canRedo,
    error: view.error,
  };
}
