"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { ArrowClockwise, ArrowCounterClockwise, ArrowLeft, ArrowRight, CopySimple, DotsThree, PencilSimple, Plus, SquaresFour, Trash } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { writeListeners } from "@/lib/api";
import { EmptyState } from "@/components/EmptyState";
import { ErrorPanel, LoadFrame } from "@/components/Loading";
import { Skeleton, InlineSkeleton } from "@/components/Skeleton";
import { notify } from "@/components/Notify";
import { NameField } from "@/components/NameField";
import { Popover } from "@/components/Popover";
import { PageActions, PageTitle, useBarFlag, useContextInBody } from "@/shell/PageHeader";
import { effectMissing, effectNameOf } from "@/lib/status";
import { getSnap, retryScenes, useSnap } from "@/lib/store";
import { AddDevices } from "@/views/scenes/AddDevices";
import { GroupRow, UnassignedRow, type LightsState } from "@/views/scenes/GroupRow";
import { GroupSheet, Sheet } from "@/views/scenes/GroupSheet";
import { LightFeedContext } from "@/views/scenes/LedRun";
import { NewLightNotice } from "@/views/scenes/NewLightNotice";
import { SceneStrip } from "@/views/scenes/SceneStrip";
import { groupGoneMessage, groupOrderMoved, moveLight } from "@/views/scenes/groupEdit";
import { useNewLights } from "@/views/scenes/newLights";
import { useLightFeed } from "@/views/scenes/useLightFeed";
import { useGroupReorder } from "@/views/scenes/useGroupReorder";
import { useScene } from "@/views/scenes/useScene";
import type { Group } from "@/proto/Group";
import { Onboarding } from "@/views/editor/Onboarding";
import { ChainTakeover } from "@/views/chain/ChainTakeover";
import { chainKey } from "@/views/chain/offered";
import { requestChainSetup } from "@/views/chain/request";
import { shouldOpen } from "@/views/editor/flow";
import { SetupLayout } from "@/views/setup/SetupLayout";
import type { LightView } from "@/proto/LightView";
import s from "@/views/scenes/scenes.module.css";
import r from "@/views/scenes/groupReorder.module.css";
import { toastText } from "@/lib/errors";

/** The room the list holds while the scenes are read: the strip's rows, drawn as placeholders. */
const SKEL_NAMES = ["52%", "38%", "46%"] as const;

function ScenesShape({ rows }: { rows: number }) {
  return (
    <div className={s.list}>
      {Array.from({ length: rows }, (_, i) => (
        <div className={s.row} key={i} aria-hidden="true">
          {/* The handle's own column: the placeholder keeps the place of the real row. */}
          <span className={s.gripSpace} data-testid="grip-space" />
          <div className={s.who}>
            <Skeleton width={SKEL_NAMES[i % SKEL_NAMES.length]} height={16} />
            <Skeleton width="68%" height={14} />
          </div>
          <div className={s.runWell}>
            <Skeleton className={s.skelRun} height="100%" radius={8} />
          </div>
          <Skeleton className={s.skelKey} height={32} radius={10} />
        </div>
      ))}
    </div>
  );
}

/** The scene line: its name, unless something above already says it, and what is in it. */
function SceneLine({ name, groups, leds }: { name: string | null; groups: number; leds: number | null }) {
  const counts =
    groups === 0
      ? "no groups yet"
      : `${groups} ${groups === 1 ? "group" : "groups"}${leds === null ? "" : ` · ${leds} LEDs`}`;
  return (
    <span className="rd-scenes-context">
      {name !== null && <b>{name}</b>}
      <span className="rd-scenes-counts">{name === null ? counts : ` · ${counts}`}</span>
    </span>
  );
}

/**
 * The Scenes page: the scene keys, then one row per group of the active scene with its live
 * LEDs, the lights no group holds, and under them the notice for a light that was just detected. Every
 * change reaches the lights at once (`useScene`); there is no Save.
 */
export function ScenesView() {
  const use = useScene();
  const live = useLightFeed();
  // Safe start (#577): the saved scene is shown and not played, so its rows show no colours.
  const safe = useSnap((st) => st.status?.safe_start?.active ?? false);
  const feed = useMemo<typeof live>(() => (safe ? () => null : live), [safe, live]);
  useEffect(() => {
    if (!safe) return;
    const heard = (_method: string, path: string) => {
      if (/^\/api\/v1\/(scenes|lights|chains)\b/.test(path)) notify("Saved. Applies when you start normally.", "ok");
    };
    writeListeners.add(heard);
    return () => {
      writeListeners.delete(heard);
    };
  }, [safe]);
  const effects = useSnap((st) => st.effects);
  const lights = useSnap((st) => st.lights);
  const lightsError = useSnap((st) => st.lightsError);
  const answered = useSnap((st) => st.scenes);
  const scenesError = useSnap((st) => st.scenesError);
  const chains = useSnap((st) => st.chains);
  const retrying = useSnap((st) => st.pending.includes("scenes"));
  const [openId, setOpenId] = useState<string | null>(null);
  // The first run's hello (#310): the page behind it is Scenes, so the greeting lives here.
  const [hello, setHello] = useState(() => shouldOpen(getSnap().status));
  const [sheetStep, setSheetStep] = useState<"group" | "name" | "effect">("group");
  const [making, setMaking] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [sceneName, setSceneName] = useState("");
  const [sceneMenu, setSceneMenu] = useState(false);
  const [newSceneName, setNewSceneName] = useState(false);
  const noticeRef = useRef<HTMLDivElement>(null);
  const sceneMenuAnchor = useRef<HTMLButtonElement>(null);
  /** The id a group made here gets. */
  const groupSeq = useRef(0);
  /** The active scene before this render: a new scene is the one that changed it. */
  const lastActive = useRef<string | null>(null);
  const { scene, unassigned, error, undo, redo } = use;
  /** The groups of the scene that plays, in the order they are in. */
  const groups = scene?.groups ?? [];
  /** The list the group rows are in: the drag's marker stands in the gaps of it. */
  const listRef = useRef<HTMLDivElement>(null);
  /** What the keyboard is told: where a group went, or that it is already at an end. */
  const [groupStatus, setGroupStatus] = useState("");

  const byId = useMemo(() => new Map((lights ?? []).map((l) => [l.light.id, l])), [lights]);
  const rowLights = useMemo(() => {
    const out = new Map<string, LightView[]>();
    for (const g of scene?.groups ?? []) {
      out.set(
        g.id,
        g.members.flatMap((id) => {
          const l = byId.get(id);
          return l ? [l] : [];
        }),
      );
    }
    return out;
  }, [scene, byId]);

  // A row cannot count lights it has not been told about: until the list arrives, or when it
  // failed, the rows say so instead of "0 devices".
  const lightsState: LightsState = lights ? "ready" : lightsError ? "failed" : "loading";

  const effectFor = useCallback(
    (g: Group) =>
      g.plays.type === "off"
        ? { name: "Off", id: undefined, missing: false }
        : {
            name: effectNameOf(effects, g.plays.effect),
            id: g.plays.effect,
            missing: effectMissing(effects, g.plays.effect),
          },
    [effects],
  );

  const { fresh, dismiss } = useNewLights(lights, unassigned);

  // The header a new light is on that is not set up: the notice offers its setup, so the owner can
  // tell the app what is plugged into it before the light joins a group (#380). One key only: a
  // notice is a line, and the other headers are on the Devices page.
  const freshHeader = useMemo(() => {
    const chain = fresh
      .map((l) => chains?.find((c) => c.device_id === l.light.device_id && c.zone_index === l.light.zone_index))
      .find((c) => c?.state === "not_set_up");
    return chain ? { key: chainKey(chain.device_id, chain.zone_index), label: chain.zone_name } : null;
  }, [fresh, chains]);

  // A refused write says so in the toast lane, which floats: the page does not move for it.
  useEffect(() => {
    if (error) notify(toastText(error), "down");
  }, [error]);

  // Ctrl+Z and Ctrl+Shift+Z are the undo and redo of every change on this page. A key in a text
  // field is the field's own: the browser undoes the typing there.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey && !event.metaKey) return;
      const where = event.target as HTMLElement | null;
      if (where && (where.tagName === "INPUT" || where.tagName === "TEXTAREA" || where.isContentEditable)) {
        return;
      }
      const key = event.key.toLowerCase();
      if (key === "z") {
        event.preventDefault();
        if (event.shiftKey) redo();
        else undo();
        return;
      }
      if (key === "y") {
        event.preventDefault();
        redo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [redo, undo]);

  const addFresh = (groupId: string) => {
    const ids = fresh.map((l) => l.light.id);
    use.edit((draft) => {
      const group = draft.groups.find((g) => g.id === groupId);
      if (group) group.members.push(...ids.filter((id) => !group.members.includes(id)));
    }, "Add a new light to a group");
  };

  // The "N new" key in the strip: the notice is at the foot of the list, so scroll to it and move focus there.
  const jumpToNotice = () => {
    const el = noticeRef.current;
    if (!el) return;
    const calm = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
    el.scrollIntoView?.({ block: "nearest", behavior: calm ? "auto" : "smooth" });
    el.focus({ preventScroll: true });
  };

  /** A new scene is a copy of the one that plays, and it plays at once. */
  const newScene = (empty: boolean) => {
    if (!scene) return;
    setNewSceneName(true);
    if (empty) use.create("New scene");
    // A scene's name is at most 40 characters; the copy keeps room for " copy".
    else use.create(`${[...scene.name].slice(0, 35).join("")} copy`, scene.id);
  };

  // The new scene is the one that became active: its name goes into edit.
  const activeId = scene?.id ?? null;
  useEffect(() => {
    const before = lastActive.current;
    lastActive.current = activeId;
    if (!newSceneName || !activeId || activeId === before) return;
    setNewSceneName(false);
    setSceneName(scene?.name ?? "");
    setRenaming(true);
  }, [activeId, newSceneName, scene]);

  const commitSceneName = (name: string) => {
    const clean = name.trim();
    setRenaming(false);
    if (!scene || !clean || clean === scene.name) return;
    use.renameScene(scene.id, clean);
  };

  /** An id no group of the scene has. */
  const freshGroupId = (): string => {
    const taken = new Set((scene?.groups ?? []).map((g) => g.id));
    let id = "";
    do {
      groupSeq.current += 1;
      id = `g-new-${groupSeq.current}`;
    } while (taken.has(id));
    return id;
  };

  /**
   * The scenes in another order, with the one that plays moved one place (#371). This is the
   * phone's path, and the path for a finger: the strip scrolls sideways there.
   */
  const moveScene = (delta: number) => {
    const ids = use.scenes.map((s) => s.id);
    const at = ids.indexOf(scene?.id ?? "");
    if (at < 0) return;
    const to = at + delta;
    if (to < 0 || to >= ids.length) return;
    const order = [...ids];
    const [moved] = order.splice(at, 1);
    order.splice(to, 0, moved!);
    use.reorderScenes(order);
  };

  /** Where the scene that plays is in the order of the keys. */
  const sceneAt = use.scenes.findIndex((s) => s.id === scene?.id);

  // The strip names what is playing; where that is this scene and the rail's deck is collapsed,
  // the scene line leaves the name out, so it is on screen once (#586 B11). The body's status line
  // is the name's home when the strip could not hold it.
  const namedInStrip = useBarFlag("nowWhere", "strip");
  const namedInBody = useBarFlag("nowWhere", "body");
  const playingName = useSnap((x) => x.status?.active_scene?.name);
  const nameSaid = (namedInStrip || namedInBody) && scene !== undefined && scene !== null && scene.name === playingName;
  // Undo and Redo are secondary keys: a bar that cannot hold them keeps them in the scene's menu.
  const keysInMenu = useBarFlag("barSecondary", "menu");

  // The rename field stands where the scene line is. When the strip moves that line between the
  // bar and the body, the focus follows the copy that is on screen (#586 R3).
  const contextInBody = useContextInBody();
  useEffect(() => {
    if (!renaming) return;
    const fields = document.querySelectorAll<HTMLInputElement>('input[aria-label="The name of this scene"]');
    const shown = [...fields].find((field) => field.getClientRects().length > 0);
    if (!shown || document.activeElement === shown) return;
    shown.focus();
    shown.select();
  }, [renaming, contextInBody]);

  /**
   * The groups of the scene, in another order (#384).
   *
   * The rows are dragged by their handle, and the arrow keys on the handle move a group one
   * place. The order shows at once and the whole scene goes to the daemon in one write.
   */
  const reorder = useGroupReorder(groups, listRef, (order) => use.reorderGroups(order));

  /**
   * A group one place up or down. The keyboard is told where it went, or that it is already at
   * that end.
   */
  const moveGroup = useCallback(
    (group: Group, delta: number) => {
      const ids = groups.map((g) => g.id);
      const at = ids.indexOf(group.id);
      const order = groupOrderMoved(ids, group.id, delta);
      if (!order) {
        setGroupStatus(delta < 0 ? `${group.name} is already first.` : `${group.name} is already last.`);
        return;
      }
      setGroupStatus(`${group.name}, position ${at + delta + 1} of ${ids.length}.`);
      use.reorderGroups(order);
    },
    [groups, use],
  );

  /** The arrow keys on a row's handle. No modifier: the handle has no other meaning for them. */
  const onGripKey = useCallback(
    (group: Group, event: ReactKeyboardEvent<HTMLButtonElement>) => {
      if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      event.preventDefault();
      moveGroup(group, event.key === "ArrowUp" ? -1 : 1);
    },
    [moveGroup],
  );

  /** A device was picked for the group being made: the group exists and plays from now on. */
  const onPicked = (lightId: string) => {    const id = freshGroupId();
    const name = `Group ${(scene?.groups.length ?? 0) + 1}`;
    let emptied: string[] = [];
    use.edit((draft) => {
      // A light is in one group: the group it was in lets it go, and goes with it when it was
      // the last device.
      emptied = moveLight(draft, lightId, null);
      draft.groups.push({
        id,
        name,
        members: [lightId],
        plays: { type: "off" },
        mode: "flow_across",
        brightness: 100,
      });
    }, "Make a group");
    if (emptied.length > 0) {
      notify(groupGoneMessage(emptied), "warn", { label: "Undo", onClick: () => use.undo() });
    }
    setMaking(false);
    setSheetStep("name");
    setOpenId(id);
  };

  // The scene on the page may be named in the URL: the builder's "Edit effect" links back with
  // the group it came from. An id no group has opens nothing.
  const [wantedGroup] = useState<string | null>(() =>
    typeof window === "undefined" ? null : new URLSearchParams(window.location.search).get("group"),
  );
  const wantedHandled = useRef(false);
  useEffect(() => {
    if (wantedHandled.current || !wantedGroup || !scene) return;
    if (!scene.groups.some((g) => g.id === wantedGroup)) return;
    wantedHandled.current = true;
    // A one-time decision when the scene first arrives, so it is made in this effect.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setOpenId(wantedGroup);
  }, [wantedGroup, scene]);

  const state = scene
    ? "ready"
    : answered === null && scenesError && !retrying
      ? "error"
      : "loading";

  const totalLeds = groups.reduce(
    (n, g) => n + (rowLights.get(g.id) ?? []).reduce((m, l) => m + l.led_count, 0),
    0,
  );

  // The first run's hello takes the whole window (#310): the page behind it is this one.
  if (hello) {
    return (
      <div className="onboarding-page">
        <Onboarding onDone={() => setHello(false)} />
      </div>
    );
  }

  // The header setup is offered once the hello is behind the owner: it is a takeover over this
  // page, so the lights and the groups are still the thing behind the question.
  return (
    <>
      <ChainTakeover />
      <SetupLayout onTroubleshoot={() => setHello(true)}>
      <div className={`rd-page rd-scenes ${s.page}`}>
      <PageTitle
        title="Scenes"
        contextBusy={state === "loading"}
        context={
          scene ? (
            renaming ? (
              <NameField
                value={sceneName}
                label="The name of this scene"
                placeholder="Scene name"
                onChange={setSceneName}
                onEnter={() => commitSceneName(sceneName)}
                onEscape={() => setRenaming(false)}
              />
            ) : (
              <SceneLine name={nameSaid ? null : scene.name} groups={groups.length} leds={lightsState === "ready" ? totalLeds : null} />
            )
          ) : state === "loading" ? (
            <span className="rd-scenes-context">
              <InlineSkeleton width="22ch" />
            </span>
          ) : (
            <span className="rd-scenes-context">
              <b>not read</b>
            </span>
          )
        }
      />
      {/* #586 R3: when the strip gives up the scene line, the name and its rename field move
          together to the head of the body. Both places use the same state, so rename works here
          too. CSS shows one copy, decided by the strip's own width. */}
      {scene && (
        <div className="bar-context-mirror scene-context-mirror">
          {renaming ? (
            <NameField
              value={sceneName}
              label="The name of this scene"
              placeholder="Scene name"
              onChange={setSceneName}
              onEnter={() => commitSceneName(sceneName)}
              onEscape={() => setRenaming(false)}
            />
          ) : (
            <SceneLine name={nameSaid ? null : scene.name} groups={groups.length} leds={lightsState === "ready" ? totalLeds : null} />
          )}
        </div>
      )}
      <PageActions>
        <span className="rd-secondary">
          <IconButton
            label="Undo"
            icon={<ArrowCounterClockwise size={18} />}
            size="sm"
            disabled={!use.canUndo}
            onClick={undo}
          />
          <IconButton
            label="Redo"
            icon={<ArrowClockwise size={18} />}
            size="sm"
            disabled={!use.canRedo}
            onClick={redo}
          />
        </span>
        <Button
          ref={sceneMenuAnchor}
          className={s.iconKey}
          data-testid="scene-menu"
          aria-label="More about this scene"
          aria-haspopup="true"
          aria-expanded={sceneMenu}
          icon={<DotsThree size={18} weight="bold" />}
          onClick={() => setSceneMenu((v) => !v)}
        />
        <Popover
          anchor={sceneMenuAnchor}
          open={sceneMenu}
          onClose={() => setSceneMenu(false)}
          label="More about this scene"
          className={s.sheetMenu}
        >
          {keysInMenu && (
            <>
              <Button
                variant="ghost"
                size="sm"
                icon={<ArrowCounterClockwise size={16} />}
                disabled={!use.canUndo}
                onClick={() => {
                  setSceneMenu(false);
                  undo();
                }}
              >
                Undo
              </Button>
              <Button
                variant="ghost"
                size="sm"
                icon={<ArrowClockwise size={16} />}
                disabled={!use.canRedo}
                onClick={() => {
                  setSceneMenu(false);
                  redo();
                }}
              >
                Redo
              </Button>
            </>
          )}
          <Button
            variant="ghost"
            size="sm"
            icon={<PencilSimple size={16} />}
            disabled={!scene}
            onClick={() => {
              setSceneMenu(false);
              if (!scene) return;
              setSceneName(scene.name);
              setRenaming(true);
            }}
          >
            Rename
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={<CopySimple size={16} />}
            disabled={!scene}
            onClick={() => {
              setSceneMenu(false);
              newScene(false);
            }}
          >
            Duplicate
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={<Plus size={16} />}
            disabled={!scene}
            onClick={() => {
              setSceneMenu(false);
              newScene(true);
            }}
          >
            New empty scene
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={<ArrowLeft size={16} />}
            disabled={!scene || sceneAt <= 0}
            title={sceneAt <= 0 ? "It is already first." : undefined}
            onClick={() => {
              setSceneMenu(false);
              moveScene(-1);
            }}
          >
            Move earlier
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={<ArrowRight size={16} />}
            disabled={!scene || sceneAt < 0 || sceneAt >= use.scenes.length - 1}
            title={sceneAt >= 0 && sceneAt >= use.scenes.length - 1 ? "It is already last." : undefined}
            onClick={() => {
              setSceneMenu(false);
              moveScene(1);
            }}
          >
            Move later
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={<Trash size={16} />}
            disabled={!scene || use.scenes.length <= 1}
            onClick={() => {
              setSceneMenu(false);
              if (scene) use.remove(scene.id);
            }}
          >
            Delete
          </Button>
        </Popover>
      </PageActions>

      <SceneStrip
        scenes={use.scenes}
        active={scene?.id ?? null}
        loading={state === "loading"}
        canCopy={state === "ready"}
        fresh={scene ? fresh.length : 0}
        onSet={use.setActive}
        onNew={() => newScene(false)}
        onJumpToNew={jumpToNotice}
        onReorder={(order, options) => use.reorderScenes(order, options)}
      />

      {/* In the page and not in the status strip: a phone's strip has no room for it. */}
      {safe && scene && (
        <div className={s.safeOff}>
          <Chip testId="safe-off">Off in safe start</Chip>
        </div>
      )}

      <LightFeedContext.Provider value={feed}>
        <LoadFrame
          className={s.frame}
          state={state}
          shape={<ScenesShape rows={3} />}
          label="Reading the scenes"
          error={
            <ErrorPanel
              title="The scenes could not be read"
              summary="canvas-rgb could not read the scenes. Try the read again."
              error={scenesError}
              onRetry={() => void retryScenes()}
              busy={retrying}
            />
          }
        >
          <div className={s.list} ref={listRef}>
            {groups.length === 0 && (
              <EmptyState icon={<SquaresFour size={22} />} title="This scene has no groups yet">
                Make a group from the devices below and they light up.
              </EmptyState>
            )}
            {groups.map((g) => {
              const effect = effectFor(g);
              return (
                <GroupRow
                  key={g.id}
                  group={g}
                  lights={rowLights.get(g.id) ?? []}
                  lightsState={lightsState}
                  effectName={effect.name}
                  effectId={effect.id}
                  missing={effect.missing}
                  failed={use.states.find((x) => x.id === g.id)?.failed ?? null}
                  open={openId === g.id}
                  use={use}
                  onOpen={(id) => {
                    setSheetStep("group");
                    setOpenId((cur) => (cur === id ? null : id));
                  }}
                  onBrightness={use.setBrightness}
                  onGripDown={reorder.begin}
                  onGripDrag={reorder.refuse}
                  onGripKey={onGripKey}
                />
              );
            })}
            {/* With no list of lights there is nothing to say is unassigned. */}
            {lightsState === "ready" && unassigned.length > 0 && <UnassignedRow lights={unassigned} />}
            {/* The notice is at the foot, under the Unassigned row the new light is in: it arrives and
                leaves below every group row, so none of them moves. */}
            {fresh.length > 0 && scene && (
              <NewLightNotice
                noticeRef={noticeRef}
                fresh={fresh}
                groups={scene.groups}
                onAdd={addFresh}
                setup={freshHeader}
                onSetUp={(key) => requestChainSetup(key)}
                onDismiss={() => {
                  for (const l of fresh) dismiss(l.light.id);
                }}
              />
            )}
            <div className={s.newGroup}>
              <Button data-testid="new-group" icon={<Plus size={16} />} onClick={() => setMaking(true)}>
                New group
              </Button>
            </div>
            {/* The gap the lifted row would land in. It stands in the list's own box, so it stays
                in the gap when the page scrolls. */}
            {reorder.marker !== null && (
              <span
                className={r.marker}
                style={{ top: reorder.marker }}
                data-testid="group-marker"
                aria-hidden="true"
              />
            )}
          </div>
        </LoadFrame>

        {/* What the keyboard is told: where a group went. */}
        <span role="status" className="sr-only" data-testid="group-status">
          {groupStatus}
        </span>

        {/* The copy of the lifted row, drawn over the page: the list scrolls with the page and
            would clip it. */}
        {reorder.drag &&
          createPortal(
            <span
              className={r.copy}
              style={{
                left: reorder.drag.x - reorder.drag.grabX,
                top: reorder.drag.y - reorder.drag.grabY,
                minWidth: reorder.drag.width,
                "--tone": `var(${reorder.drag.tone})`,
              } as CSSProperties}
              data-testid="group-copy"
              aria-hidden="true"
            >
              <span className="one-line">{reorder.drag.label}</span>
            </span>,
            document.body,
          )}

        {openId && (
          <GroupSheet
            key={openId}
            groupId={openId}
            use={use}
            lights={lights ?? []}
            initial={sheetStep}
            onClose={() => setOpenId(null)}
          />
        )}
        {making && (
          <Sheet title="New group" onClose={() => setMaking(false)}>
            <p className={s.sheetLead}>Which devices go in it?</p>
            <AddDevices
              groupId="new"
              lights={lights ?? []}
              use={use}
              onClose={() => setMaking(false)}
              onPicked={onPicked}
            />
          </Sheet>
        )}
      </LightFeedContext.Provider>
      </div>
      </SetupLayout>
    </>
  );
}
