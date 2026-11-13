"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowClockwise, ArrowCounterClockwise, DotsThree, Plus, SquaresFour } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { EmptyState } from "@/components/EmptyState";
import { ErrorPanel, LoadFrame } from "@/components/Loading";
import { Skeleton, InlineSkeleton } from "@/components/Skeleton";
import { notify } from "@/components/Notify";
import { NameField } from "@/components/NameField";
import { Popover } from "@/components/Popover";
import { PageActions, PageTitle } from "@/shell/PageHeader";
import { effectMissing, effectNameOf } from "@/lib/status";
import { retryScenes, useSnap } from "@/lib/store";
import { AddDevices } from "@/views/scenes/AddDevices";
import { GroupRow, UnassignedRow, type LightsState } from "@/views/scenes/GroupRow";
import { GroupSheet, Sheet } from "@/views/scenes/GroupSheet";
import { LightFeedContext } from "@/views/scenes/LedRun";
import { NewLightNotice } from "@/views/scenes/NewLightNotice";
import { SceneStrip } from "@/views/scenes/SceneStrip";
import { useNewLights } from "@/views/scenes/newLights";
import { useLightFeed } from "@/views/scenes/useLightFeed";
import { useScene } from "@/views/scenes/useScene";
import type { Group } from "@/proto/Group";
import type { LightView } from "@/proto/LightView";
import s from "@/views/scenes/scenes.module.css";

/** The room the list holds while the scenes are read: the strip's rows, drawn as placeholders. */
const SKEL_NAMES = ["52%", "38%", "46%"] as const;

function ScenesShape({ rows }: { rows: number }) {
  return (
    <div className={s.list}>
      {Array.from({ length: rows }, (_, i) => (
        <div className={s.row} key={i} aria-hidden="true">
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

/**
 * The Scenes page: the scene keys, then one row per group of the active scene with its live
 * LEDs, the lights no group holds, and under them the notice for a light that was just detected. Every
 * change reaches the lights at once (`useScene`); there is no Save.
 */
export function ScenesView() {
  const use = useScene();
  const feed = useLightFeed();
  const effects = useSnap((st) => st.effects);
  const lights = useSnap((st) => st.lights);
  const lightsError = useSnap((st) => st.lightsError);
  const answered = useSnap((st) => st.scenes);
  const scenesError = useSnap((st) => st.scenesError);
  const retrying = useSnap((st) => st.pending.includes("scenes"));
  const [openId, setOpenId] = useState<string | null>(null);
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

  // A refused write says so in the toast lane, which floats: the page does not move for it.
  useEffect(() => {
    if (error) notify(error, "down");
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

  /** A device was picked for the group being made: the group exists and plays from now on. */
  const onPicked = (lightId: string) => {
    const id = freshGroupId();
    const name = `Group ${(scene?.groups.length ?? 0) + 1}`;
    use.edit((draft) => {
      draft.groups.push({
        id,
        name,
        members: [lightId],
        plays: { type: "off" },
        mode: "flow_across",
        brightness: 100,
      });
    }, "Make a group");
    setMaking(false);
    setSheetStep("name");
    setOpenId(id);
  };

  const state = scene
    ? "ready"
    : answered === null && scenesError && !retrying
      ? "error"
      : "loading";

  const totalLeds = (scene?.groups ?? []).reduce(
    (n, g) => n + (rowLights.get(g.id) ?? []).reduce((m, l) => m + l.led_count, 0),
    0,
  );
  const groups = scene?.groups ?? [];

  return (
    <div className={`rd-page ${s.page}`}>
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
              <span className="rd-scenes-context">
                <b>{scene.name}</b>
                {groups.length === 0 ? (
                  <span className="rd-scenes-counts"> · no groups yet</span>
                ) : (
                  <span className="rd-scenes-counts">
                    {" "}
                    · {groups.length} {groups.length === 1 ? "group" : "groups"}
                    {lightsState === "ready" ? ` · ${totalLeds} LEDs` : ""}
                  </span>
                )}
              </span>
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
      <PageActions>
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
        <Button
          ref={sceneMenuAnchor}
          className={s.iconKey}
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
          <Button
            variant="ghost"
            size="sm"
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
      />

      <LightFeedContext.Provider value={feed}>
        <LoadFrame
          className={s.frame}
          state={state}
          shape={<ScenesShape rows={3} />}
          label="Reading the scenes"
          error={
            <ErrorPanel
              title="The scenes could not be read"
              summary="canvas-rgb could not read the scenes. The lights are unaffected."
              message={scenesError}
              onRetry={() => void retryScenes()}
              busy={retrying}
            />
          }
        >
          <div className={s.list}>
            {groups.length === 0 && (
              <EmptyState icon={<SquaresFour size={22} />} title="This scene has no groups yet">
                Make a group from the devices below and they light up.
              </EmptyState>
            )}
            {groups.map((g, i) => {
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
                  index={i}
                  open={openId === g.id}
                  use={use}
                  onOpen={(id) => {
                    setSheetStep("group");
                    setOpenId((cur) => (cur === id ? null : id));
                  }}
                  onBrightness={use.setBrightness}
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
                onDismiss={() => {
                  for (const l of fresh) dismiss(l.light.id);
                }}
              />
            )}
            <div className={s.newGroup}>
              <Button icon={<Plus size={16} />} onClick={() => setMaking(true)}>
                New group
              </Button>
            </div>
          </div>
        </LoadFrame>

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
  );
}
