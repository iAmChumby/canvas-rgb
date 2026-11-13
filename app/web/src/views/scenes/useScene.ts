"use client";

// The Scenes page's data: the active scene, the lights no group holds, and every way to change
// them.
//
// A change reaches the screen at once and goes to the daemon as the WHOLE scene with the `rev`
// it was made from (`PUT /scenes/{id}`). Writes go one at a time, newest state last, so each
// one carries the rev the one before it returned. A 409 means another window saved first: this
// window drops its pending changes, takes the daemon's scene and says so. Undo and redo are a
// history of whole scenes for the active scene; an undo is an ordinary edit that puts the older
// scene back.
//
// `setProps` and `setBrightness` are the live paths for a slider drag: they go to the engine
// (`PATCH .../props`, `PUT .../brightness`) at most once per 60 ms per group, and the last value
// always goes. They show at once through a small overlay that the daemon's answer replaces.
//
// The bookkeeping lives in refs, because the write loop and the timers read it between renders.
// Everything a render shows is copied into one state object (`publish`), so a render never
// reads a ref.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/endpoints";
import { ApiFailure, messageOf } from "@/lib/errors";
import { getSnap, patchSnap, refreshLights, refreshScenes, useSnap } from "@/lib/store";
import type { GroupState } from "@/proto/GroupState";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
import type { Props } from "@/proto/props";

/** How many whole scenes undo remembers. */
export const HISTORY_DEPTH = 100;
/** The shortest time between two live writes of one group. */
export const LIVE_THROTTLE_MS = 60;
/** What the page says after a 409. */
export const CONFLICT_MESSAGE = "Changed in another window";

const EMPTY_SCENES: Scene[] = [];
const EMPTY_LIGHTS: LightView[] = [];
const EMPTY_STATES: GroupState[] = [];

const clone = <T,>(value: T): T => structuredClone(value);

/** One group's live values the daemon has not confirmed yet. */
interface Live {
  props?: Props;
  brightness?: number;
}

/** One group's pending live write. */
interface Pending {
  timer: ReturnType<typeof setTimeout>;
  props?: Props;
  brightness?: number;
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

/** `scene` with each group's unconfirmed live values laid over it. */
function overlay(scene: Scene | null, values: Record<string, Live>): Scene | null {
  if (!scene || Object.keys(values).length === 0) return scene;
  return {
    ...scene,
    groups: scene.groups.map((g) => {
      const l = values[g.id];
      if (!l) return g;
      return {
        ...g,
        brightness: l.brightness ?? g.brightness,
        plays:
          l.props && g.plays.type === "effect" ? { ...g.plays, props: { ...g.plays.props, ...l.props } } : g.plays,
      };
    }),
  };
}

/** The newer of what the daemon says and what a write of this window returned, for one scene. */
function newest(server: Scene | null, saved: Scene | null, id: string | null): Scene | null {
  if (saved && saved.id === id && (!server || saved.rev > server.rev)) return saved;
  return server;
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

  const draft = useRef<Scene | null>(null);
  const saved = useRef<Scene | null>(null);
  const history = useRef({ sceneId: "", past: [] as Scene[], future: [] as Scene[] });
  const writing = useRef(false);
  const error = useRef<string | null>(null);
  const live = useRef(new Map<string, Live>());
  const pending = useRef(new Map<string, Pending>());
  const mounted = useRef(false);
  // What the daemon last said, for the callbacks that run between renders.
  const known = useRef<{ server: Scene | null; activeId: string | null }>({ server: null, activeId: null });

  /** Copies the bookkeeping into state, so the next render shows it. */
  const publish = useCallback(() => {
    if (!mounted.current) return;
    const h = history.current;
    setView({
      draft: draft.current,
      saved: saved.current,
      error: error.current,
      canUndo: h.past.length > 0,
      canRedo: h.future.length > 0,
      historyScene: h.sceneId,
      live: Object.fromEntries(live.current),
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

  useEffect(() => {
    mounted.current = true;
    const timers = pending.current;
    // The page has to hold the answers once, even where nothing started the store's polling.
    void refreshScenes();
    void refreshLights();
    return () => {
      mounted.current = false;
      for (const p of timers.values()) clearTimeout(p.timer);
      timers.clear();
    };
  }, []);

  useEffect(() => {
    known.current = { server, activeId };
    // One scene's history never outlives a switch to another scene.
    if (history.current.sceneId !== (activeId ?? "")) {
      history.current = { sceneId: activeId ?? "", past: [], future: [] };
      draft.current = null;
      live.current.clear();
      publish();
    }
  }, [server, activeId, publish]);

  /** Sends the newest draft, then the next newest, until the daemon has the last one. */
  const flush = useCallback(async () => {
    if (writing.current) return;
    writing.current = true;
    try {
      for (;;) {
        const sent = draft.current;
        if (!sent) return;
        try {
          const answer = await api.scenes.put(sent.id, { scene: sent, expected_rev: sent.rev });
          saved.current = answer;
          const current = getSnap().scenes;
          if (current) {
            patchSnap({
              scenes: {
                ...current,
                scenes: current.scenes.map((s) => (s.id === answer.id && answer.rev >= s.rev ? answer : s)),
              },
            });
          }
          if (draft.current === sent) {
            draft.current = null;
            error.current = null;
            publish();
            return;
          }
          // Edited while the write was out: the next one carries the rev just returned.
          if (draft.current) draft.current = { ...draft.current, rev: answer.rev };
        } catch (e) {
          draft.current = null;
          saved.current = null;
          if (e instanceof ApiFailure && e.status === 409) {
            // Another window saved first. Nothing here is kept: the daemon's scene replaces it.
            history.current = { ...history.current, past: [], future: [] };
            await refreshScenes();
            fail(CONFLICT_MESSAGE);
          } else {
            await refreshScenes();
            fail(messageOf(e));
          }
          return;
        }
      }
    } finally {
      writing.current = false;
    }
  }, [fail, publish]);

  /** Puts `next` on screen and on its way to the daemon. */
  const apply = useCallback(
    (next: Scene) => {
      draft.current = next;
      publish();
      void flush();
    },
    [flush, publish],
  );

  const edit = useCallback(
    (change: (draft: Scene) => void, label: string) => {
      void label;
      const current = draft.current ?? base();
      if (!current) return;
      const next = clone(current);
      change(next);
      const h = history.current;
      h.past.push(clone(current));
      if (h.past.length > HISTORY_DEPTH) h.past.shift();
      h.future = [];
      apply(next);
    },
    [apply, base],
  );

  const undo = useCallback(() => {
    const h = history.current;
    const current = draft.current ?? base();
    const previous = h.past.pop();
    if (!current || !previous) return;
    h.future.push(clone(current));
    apply({ ...clone(previous), rev: current.rev });
  }, [apply, base]);

  const redo = useCallback(() => {
    const h = history.current;
    const current = draft.current ?? base();
    const next = h.future.pop();
    if (!current || !next) return;
    h.past.push(clone(current));
    apply({ ...clone(next), rev: current.rev });
  }, [apply, base]);

  /** Sends the live values of one group that were asked for since the last send. */
  const sendLive = useCallback(
    async (groupId: string) => {
      const p = pending.current.get(groupId);
      const scene = draft.current ?? base();
      pending.current.delete(groupId);
      if (!p || !scene) return;
      try {
        if (p.props) await api.scenes.patchGroupProps(scene.id, groupId, p.props);
        if (p.brightness !== undefined) await api.scenes.putGroupBrightness(scene.id, groupId, p.brightness);
        // The daemon holds the values now; its own answer replaces the overlay, unless the
        // slider has moved on since.
        await refreshScenes();
        if (!pending.current.has(groupId)) live.current.delete(groupId);
      } catch (e) {
        live.current.delete(groupId);
        fail(messageOf(e));
        return;
      }
      publish();
    },
    [base, fail, publish],
  );

  const queueLive = useCallback(
    (groupId: string, change: (p: Pending) => void, show: (l: Live) => void) => {
      const l = live.current.get(groupId) ?? {};
      show(l);
      live.current.set(groupId, l);
      const existing = pending.current.get(groupId);
      if (existing) {
        change(existing);
      } else {
        const p: Pending = {
          timer: setTimeout(() => {
            void sendLive(groupId);
          }, LIVE_THROTTLE_MS),
        };
        change(p);
        pending.current.set(groupId, p);
      }
      publish();
    },
    [publish, sendLive],
  );

  const setProps = useCallback(
    (groupId: string, props: Props) => {
      queueLive(
        groupId,
        (p) => {
          p.props = { ...p.props, ...props };
        },
        (l) => {
          l.props = { ...l.props, ...props };
        },
      );
    },
    [queueLive],
  );

  const setBrightness = useCallback(
    (groupId: string, n: number) => {
      const value = Math.max(0, Math.min(100, Math.round(n)));
      queueLive(
        groupId,
        (p) => {
          p.brightness = value;
        },
        (l) => {
          l.brightness = value;
        },
      );
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
