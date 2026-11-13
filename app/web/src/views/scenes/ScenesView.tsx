"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { ArrowClockwise, ArrowCounterClockwise, Plus, SquaresFour } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { EmptyState } from "@/components/EmptyState";
import { ErrorPanel, LoadFrame } from "@/components/Loading";
import { Skeleton, InlineSkeleton } from "@/components/Skeleton";
import { notify } from "@/components/Notify";
import { PageActions, PageTitle } from "@/shell/PageHeader";
import { effectMissing, effectNameOf } from "@/lib/status";
import { retryScenes, useSnap } from "@/lib/store";
import { GroupRow, UnassignedRow, type LightsState } from "@/views/scenes/GroupRow";
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
 * LEDs, the lights no group holds, and the notice for a light that was just detected. Every
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
  const { scene, unassigned, error } = use;

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

  const addFresh = (groupId: string) => {
    const ids = fresh.map((l) => l.light.id);
    use.edit((draft) => {
      const group = draft.groups.find((g) => g.id === groupId);
      if (group) group.members.push(...ids.filter((id) => !group.members.includes(id)));
    }, "Add a new light to a group");
  };

  const newScene = () => {
    if (!scene) return;
    // A scene's name is at most 40 characters; the copy keeps room for " copy".
    use.create(`${[...scene.name].slice(0, 35).join("")} copy`, scene.id);
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
          ) : state === "loading" ? (
            <span className="rd-scenes-context">
              <InlineSkeleton width="22ch" />
            </span>
          ) : (
            <span className="rd-scenes-context">
              <b>Scenes not read</b>
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
          onClick={use.undo}
        />
        <IconButton
          label="Redo"
          icon={<ArrowClockwise size={18} />}
          size="sm"
          disabled={!use.canRedo}
          onClick={use.redo}
        />
      </PageActions>

      <SceneStrip
        scenes={use.scenes}
        active={scene?.id ?? null}
        loading={state === "loading"}
        canCopy={state === "ready"}
        onSet={use.setActive}
        onNew={newScene}
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
            {fresh.length > 0 && scene && (
              <NewLightNotice
                fresh={fresh}
                groups={scene.groups}
                onAdd={addFresh}
                onDismiss={() => {
                  for (const l of fresh) dismiss(l.light.id);
                }}
              />
            )}
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
                  onOpen={(id) => setOpenId((cur) => (cur === id ? null : id))}
                  onBrightness={use.setBrightness}
                />
              );
            })}
            {/* With no list of lights there is nothing to say is unassigned. */}
            {lightsState === "ready" && unassigned.length > 0 && <UnassignedRow lights={unassigned} />}
            <div className={s.newGroup}>
              <Button
                icon={<Plus size={16} />}
                disabled
                title="Pick the devices first. The group sheet arrives next."
              >
                New group
              </Button>
            </div>
          </div>
        </LoadFrame>
      </LightFeedContext.Provider>
    </div>
  );
}
