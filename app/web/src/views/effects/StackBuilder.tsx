"use client";

// The stack builder (D95.5): the Effects route in build mode. The picture and its layers on the
// left, the picked layer on the right, the stack's presets below it. The preview is the daemon's
// second engine (`?stack=1`); Try on the lights is a stack draft; Save writes the stack and
// activates it. Nothing here writes an effect file or a stack except those two routes.
//
// #257 phase 4 split the former 3,300-line component: the history (useStackHistory), the read
// (useStackLoad), the preview and the draft on the lights (usePreview, useDraftOnLights), the
// layer edits (useStackEdits), the mask drop (useMaskDrop), the drag session (useDragSession),
// the save and its guard (useSaveAndLeave) and the presets (usePresets) are hooks beside this
// file; the layer list, inspector, stage, presets and overlays are their own components.

import { useEffect, useMemo, useRef, useState } from "react";
import { LoadFrame, ErrorPanel } from "@/components/Loading";
import { Skeleton } from "@/components/Skeleton";
import { Panel } from "@/components/Panel";
import { Segmented } from "@/components/Segmented";
import { useMediaQuery } from "@/components/MediaQuery";
import { BuilderHeader } from "@/views/effects/BuilderHeader";
import { BuilderNotices } from "@/views/effects/BuilderNotices";
import { PhoneSummary } from "@/views/effects/PhoneSummary";
import { BuilderSide } from "@/views/effects/BuilderSide";
import { Library, type LibraryMode } from "@/views/effects/Library";
import { LayerList } from "@/views/effects/LayerList";
import { StackStage } from "@/views/effects/StackStage";
import { StarterTiles } from "@/views/effects/StarterTiles";
import { STARTER_STACKS } from "@/lib/demoStacks";
import {
  ChooseEffectPopover,
  DragGhost,
  DragHint,
  LeavePrompt,
  LiveRegion,
  MaskPickerPopover,
  NameStackDialog,
} from "@/views/effects/StackOverlays";
import { StackAreaPicker } from "@/views/effects/StackAreaPicker";
import { closePopover } from "@/views/effects/popoverFocus";
import { nameOf } from "@/views/effects/layerText";
import {
  htmlLayers,
  layersForServer,
  MAX_LAYERS,
  toggleLayer,
  StackSession,
  newEpoch,
  maskRefusal,
} from "@/views/effects/stackDraft";
import { endRun } from "@/lib/draftHistory";
import { useSnap } from "@/lib/store";
import { buildGroupCanvas, wholeLightsCanvas } from "@/lib/groupCanvas";
import { useStackHistory } from "@/views/effects/useStackHistory";
import { useStackLoad } from "@/views/effects/useStackLoad";
import { usePresets } from "@/views/effects/usePresets";
import { useStackEdits } from "@/views/effects/useStackEdits";
import { useMaskDrop } from "@/views/effects/useMaskDrop";
import { useDragSession } from "@/views/effects/useDragSession";
import { usePreview } from "@/views/effects/usePreview";
import { useDraftOnLights } from "@/views/effects/useDraftOnLights";
import { useSaveAndLeave } from "@/views/effects/useSaveAndLeave";
import { useSheetFocus } from "@/views/effects/useSheetFocus";
import { useBuilderKeys } from "@/views/effects/useBuilderKeys";
import { useAreaPicker } from "@/views/effects/useAreaPicker";
import type { Stack } from "@/proto/Stack";

export function StackBuilder({
  initialName,
  onClose,
  onName,
  onWriteCode,
}: {
  /** The `?stack=` value: a saved stack's name, or `new`. */
  initialName: string;
  onClose: () => void;
  /** A newly saved stack's name: the URL becomes `?stack=<name>` without reloading. */
  onName: (name: string) => void;
  /** The library's last tile: leave the builder and open the HTML effect flow. */
  onWriteCode?: () => void;
}) {
  const isNew = initialName === "new";
  const devices = useSnap((s) => s.devices);
  const lightList = useSnap((s) => s.lights);
  const scenes = useSnap((s) => s.scenes);
  const status = useSnap((s) => s.status);
  const effects = useSnap((s) => s.effects);
  // "Edit effect" from an open group opens the builder with `?group=<id>`: the stage then draws
  // that group. Without one it draws every light as one flow-across group.
  const [groupParam] = useState<string | null>(() =>
    typeof window === "undefined" ? null : new URLSearchParams(window.location.search).get("group"),
  );
  const group = useMemo(
    () =>
      scenes?.scenes
        .find((scene) => scene.id === scenes.active)
        ?.groups.find((entry) => entry.id === groupParam) ?? null,
    [scenes, groupParam],
  );
  const canvas = useMemo(() => {
    if (group) return buildGroupCanvas(group, lightList ?? [], devices ?? []);
    return wholeLightsCanvas(lightList ?? [], devices ?? []);
  }, [group, lightList, devices]);

  const [nameDraft, setNameDraft] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  // Parts (the desk's outlines) starts off; a drag shows them anyway (spec §3).
  const [parts, setParts] = useState(false);
  /** The live region's line: every lift, target and drop is said here. */
  const [announce, setAnnounce] = useState("");
  /** The phone's tab under the stage: the layer list, or the library. */
  const [phoneTab, setPhoneTab] = useState<"layers" | "library">("layers");
  const [chooseFor, setChooseFor] = useState<string | null>(null);
  // The phone's layer sheet: Tune pushes it over the builder (D95.5.7).
  const [sheetOpen, setSheetOpen] = useState(false);
  /** A mask control's own refusal (the HTML cap), shown beside the mask effect Select (#256). */
  const [maskProblem, setMaskProblem] = useState<string | null>(null);
  const narrow = useMediaQuery("(max-width: 720px)");
  /** The library's shape at this width (spec §3): a rail, a tray under the stage, or a tab. */
  const wide = useMediaQuery("(min-width: 1180px)");
  const libraryMode: LibraryMode = narrow ? "tab" : wide ? "rail" : "tray";

  const libraryRef = useRef<HTMLDivElement>(null);
  const tuneRef = useRef<HTMLSpanElement>(null);
  const chooseRef = useRef<HTMLSpanElement>(null);
  const sheetRef = useRef<HTMLDivElement>(null);
  const backRef = useRef<HTMLSpanElement>(null);
  const promptRef = useRef<HTMLDivElement>(null);
  /** A pointer drag or resize on the stage is in progress: undo and redo wait for it. */
  const areaDraggingRef = useRef(false);
  const nameRef = useRef("");
  const baseRef = useRef<Stack | null>(null);
  const pickedRef = useRef<string | null>(null);
  const mountedRef = useRef(true);
  // A stable session object: an initializer the render never reassigns.
  const [session] = useState(() => ({ current: new StackSession(newEpoch()) }));

  const history = useStackHistory({ id: "stack:", layers: [] }, () => areaDraggingRef.current);
  const { edit, setEdit, draft, setDraft, resetDraft, historyMoves, steppedRef, undoNow, redoNow, canUndo, canRedo } =
    history;
  const draftRef = useRef<Stack>(draft);

  /** Picking a layer: a mask refusal belongs to the layer it was said on, so it clears here (#256). */
  const pick = (id: string | null) => {
    setMaskProblem(null);
    setPicked(id);
  };

  const load = useStackLoad({ initialName, isNew, mountedRef, areaDraggingRef, resetDraft, pick });

  // The refs the async senders read. They are synced here, before the preview and draft hooks
  // register their own effects, so a send always sees the draft it was rendered from.
  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);
  useEffect(() => {
    nameRef.current = nameDraft;
  }, [nameDraft]);
  useEffect(() => {
    baseRef.current = load.base;
  }, [load.base]);
  useEffect(() => {
    pickedRef.current = picked;
  }, [picked]);

  const presets = usePresets({
    layers: draft.layers,
    schemas: load.schemas,
    schemasRef: load.schemasRef,
    base: load.base,
    presets: load.presets,
    setPresets: load.setPresets,
    setDraft,
  });

  const edits = useStackEdits({
    draft,
    picked,
    canvas,
    draftRef,
    mounted: mountedRef,
    historyMoves,
    setDraft,
    pick,
    setSheetOpen,
    setSchemas: load.setSchemas,
  });

  const mask = useMaskDrop({
    draft,
    draftRef,
    effects,
    mounted: mountedRef,
    historyMoves,
    maskProblem,
    setMaskProblem,
    setDraft,
    setSchemas: load.setSchemas,
    setAnnounce,
  });

  const dragSession = useDragSession({
    canvas,
    devices,
    draft,
    draftRef,
    picked,
    setAnnounce,
    setDraft,
    addEffect: edits.addEffect,
    applyMask: mask.applyMask,
  });

  const count = draft.layers.length;

  const name = isNew
    ? nameDraft.trim() || "New stack"
    : nameOf(load.base?.id ?? (initialName.startsWith("stack:") ? initialName : `stack:${initialName}`));
  const savedKey = load.base ? JSON.stringify({ id: load.base.id, layers: load.base.layers }) : null;
  const currentKey = JSON.stringify({ id: `stack:${name}`, layers: draft.layers });
  const dirty = load.base ? currentKey !== savedKey : draft.layers.length > 0 || nameDraft.length > 0;
  const pickedLayer = draft.layers.find((layer) => layer.id === picked) ?? null;
  const fpsTarget = status?.fps_target ?? 30;
  const htmlBudget = Math.min(8, 400 / Math.max(1, fpsTarget));
  const html = htmlLayers(draft.layers);

  function stackForServer(): Stack {
    const id = isNew
      ? `stack:${nameRef.current.trim() || "New stack"}`
      : baseRef.current?.id ?? (initialName.startsWith("stack:") ? initialName : `stack:${initialName}`);
    return { id, layers: layersForServer(draftRef.current.layers) };
  }

  const area = useAreaPicker(edits.patch);

  const preview = usePreview({
    phase: load.phase,
    draft,
    session,
    mounted: mountedRef,
    stackForServer,
    setStatuses: load.setStatuses,
  });

  const lights = useDraftOnLights({ count, session, mounted: mountedRef, stackForServer });
  const saver = useSaveAndLeave({
    isNew,
    name,
    nameDraft,
    setNameDraft,
    nameRef,
    otherStacks: load.otherStacks,
    setOtherStacks: load.setOtherStacks,
    draftRef,
    base: load.base,
    setBase: load.setBase,
    resetDraft,
    pick,
    count,
    dirty,
    setSheetOpen,
    onClose,
    onName,
    onWriteCode,
    endDraft: lights.endDraft,
    loadPresets: load.loadPresets,
    backRef,
    tuneRef,
    libraryRef,
    promptRef,
  });

  // A layer's schema arrives after the draft: fill its controls, and send the preview again so the
  // daemon's statuses name the effect.
  useEffect(() => {
    if (load.phase === "ready") load.loadDetails(draft);
    // The loader's own functions are stable; the loader object is rebuilt every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft, load.phase, load.loadDetails]);

  useSheetFocus({
    sheetOpen,
    pickerFor: area.pickerFor,
    leaving: saver.leaving,
    setSheetOpen,
    phoneTab,
    setPhoneTab,
    narrow,
    picked,
    sheetRef,
    tuneRef,
    libraryRef,
  });

  // After a step: a pick that the draft no longer holds (an undo took the layer back) moves to
  // the top layer. The layer sheet closes too: it was the removed layer's.
  useEffect(() => {
    if (!steppedRef.current) return;
    steppedRef.current = false;
    const current = pickedRef.current;
    if (current === null || edit.draft.layers.some((layer) => layer.id === current)) return;
    setMaskProblem(null);
    setPicked(edit.draft.layers[edit.draft.layers.length - 1]?.id ?? null);
    setSheetOpen(false);
  }, [edit, steppedRef]);

  useBuilderKeys({
    draft,
    sheetOpen,
    pickerFor: area.pickerFor,
    leaving: saver.leaving,
    naming: saver.naming,
    maskFor: mask.maskFor,
    dragSource: dragSession.drag.source,
    pickedRef,
    undoNow,
    redoNow,
    duplicate: edits.duplicate,
  });

  const closeChoose = () => closePopover(".rd-choose-effect", chooseRef, () => setChooseFor(null));

  /** The library, in whichever shape this width gives it. */
  const libraryPanel = (mode: LibraryMode) => (
    <Panel className="rd-library-panel" title="Library">
      <Library
        mode={mode}
        effects={effects}
        layers={draft.layers}
        problem={edits.addProblem}
        containerRef={libraryRef}
        onAdd={dragSession.onTileAdd}
        onLiftStart={dragSession.onTileLiftStart}
        onLiftKey={dragSession.onTileLiftKey}
        onWriteCode={saver.writeCode}
      />
    </Panel>
  );

  return (
    <div className={narrow ? "rd-page rd-stacks rd-stacks-phone" : "rd-page rd-stacks"} data-testid="stack-builder">
      <BuilderHeader
        name={name}
        count={count}
        narrow={narrow}
        dirty={dirty}
        saveState={saver.saveState}
        canSave={count > 0 && load.phase === "ready" && load.loadError === null}
        backRef={backRef}
        onBack={saver.askLeave}
        onRevert={saver.revert}
        onSave={() => void saver.save()}
      />

      <BuilderNotices
        draftNote={lights.draftNote}
        clash={saver.clash}
        saving={saver.saveState === "saving"}
        onReplace={() => void saver.replaceOther()}
        onCancelClash={() => saver.setClash(null)}
      />

      {load.phase === "error" ? (
        <div className="rd-stack-body" data-library={libraryMode}>
          <div className="rd-stack-main">
            <ErrorPanel
              title="The stack could not be read"
              summary="canvas-rgb could not read that stack. Nothing was changed."
              message={load.loadError}
              onRetry={load.load}
            />
          </div>
        </div>
      ) : (
        <LoadFrame
          className="rd-stack-frame"
          state={load.phase === "loading" ? "loading" : "ready"}
          shape={
            <div className="rd-stack-shape" aria-hidden="true">
              <Skeleton />
              <Skeleton />
            </div>
          }
          label="Loading the stack"
        >
          <div className="rd-stack-body" data-library={libraryMode}>
            {libraryMode === "rail" && libraryPanel("rail")}
            <div className="rd-stack-main" data-empty={count === 0 ? "true" : undefined}>
              {saver.startFailure && (
                <ErrorPanel
                  title="Saved, but it did not start"
                  summary="canvas-rgb saved that stack, but could not start it on the lights."
                  message={saver.startFailure.message}
                  onRetry={() => void saver.startAgain()}
                  retryLabel="Start again"
                  busy={saver.starting}
                />
              )}
              {saver.saveError && (
                <ErrorPanel
                  title="The stack could not be saved"
                  summary="canvas-rgb could not save that stack. Nothing on the lights changed."
                  message={saver.saveError}
                  onRetry={() => void saver.save()}
                />
              )}

              <StackStage
                draft={draft}
                canvas={canvas}
                devices={devices}
                pickedLayer={pickedLayer}
                parts={parts}
                onParts={setParts}
                canUndo={canUndo}
                canRedo={canRedo}
                onUndo={undoNow}
                onRedo={redoNow}
                onSelect={pick}
                onAreaChange={(area, run, hold) =>
                  pickedLayer && edits.patch(pickedLayer.id, { area }, run, hold)
                }
                onAreaChangeEnd={() => setEdit((current) => endRun(current))}
                onDragActive={(active) => {
                  areaDraggingRef.current = active;
                }}
                onDelete={edits.remove}
                onDuplicate={edits.duplicate}
                onMakeRect={edits.makeRect}
                drag={dragSession.drag}
                dragWords={dragSession.dragWords}
                schema={pickedLayer ? (load.schemas[pickedLayer.effect] ?? null) : null}
                onPropChange={(id, key, value, run, hold) =>
                  setDraft(
                    (current) => ({
                      ...current,
                      layers: current.layers.map((layer) =>
                        layer.id === id ? { ...layer, props: { ...layer.props, [key]: value } } : layer,
                      ),
                    }),
                    run,
                    hold,
                  )
                }
                onPropChangeEnd={() => setEdit((current) => endRun(current))}
                previewError={preview.previewError}
                draftError={lights.draftError}
                feedProblem={preview.feedProblem}
                count={count}
                fpsTarget={fpsTarget}
                htmlBudget={htmlBudget}
                html={html}
                draftState={lights.draftState}
                expiresIn={lights.expiresIn}
                startDraft={() => void lights.startDraft()}
                stopDraft={() => void lights.stopDraft()}
                save={saver.save}
                saveState={saver.saveState}
                leaving={saver.leaving}
              />

              {count === 0 && (
                <StarterTiles
                  starters={STARTER_STACKS}
                  onAdd={(starter) => {
                    void edits.addStack(starter);
                    if (narrow) setPhoneTab("layers");
                  }}
                />
              )}

              {/* An empty stack shows the library with the stage (spec §5). On a phone the library
                  is a tab normally; the empty state puts one row of tiles under the starters
                  instead, so "Drag an effect here" has something to drag without a second tap. */}
              {narrow && count === 0 && phoneTab === "layers" && libraryPanel("tray")}

              {libraryMode === "tray" && libraryPanel("tray")}

              {narrow && (
                <div className="rd-phone-tabs">
                  <Segmented<"layers" | "library">
                    label="What shows under the stage"
                    value={phoneTab}
                    options={[
                      { value: "layers", label: "Layers" },
                      { value: "library", label: "Library" },
                    ]}
                    onChange={setPhoneTab}
                  />
                </div>
              )}

              {narrow && phoneTab === "library" && libraryPanel("tab")}

              {(!narrow || phoneTab === "layers") && (
                <Panel
                  className="rd-layers"
                  title="Layers"
                  actions={
                    <span className="mute fig" style={{ fontSize: "var(--t14)" }}>
                      {count} / {MAX_LAYERS}
                    </span>
                  }
                >
                  <LayerList
                    layers={draft.layers}
                    picked={picked}
                    statuses={load.statuses}
                    schemas={load.schemas}
                    effects={effects}
                    canvas={canvas}
                    devices={devices}
                    drag={dragSession.drag}
                    isLifted={dragSession.isLifted}
                    insertAt={dragSession.insertAt}
                    renaming={edits.renaming}
                    renameText={edits.renameText}
                    renameProblem={edits.renameProblem}
                    onPick={pick}
                    onRemove={edits.remove}
                    onGripDown={dragSession.onGripDown}
                    onGripKey={dragSession.onGripKey}
                    onReorder={edits.reorder}
                    onToggleVisible={(id) =>
                      setDraft((current) => ({ ...current, layers: toggleLayer(current.layers, id) }))
                    }
                    onBeginRename={edits.beginRename}
                    onRenameText={(value) => {
                      edits.setRenameText(value);
                      edits.setRenameProblem(null);
                    }}
                    onRenameCommit={edits.commitRename}
                    onRenameEscape={() => edits.setRenaming(null)}
                    onRemoveMask={mask.removeMask}
                    onOpenMaskPicker={mask.openMaskPicker}
                  />
                </Panel>
              )}

              {narrow && pickedLayer && (
                <PhoneSummary
                  layer={pickedLayer}
                  schemas={load.schemas}
                  effects={effects}
                  canvas={canvas}
                  devices={devices}
                  tuneRef={tuneRef}
                  onTune={() => setSheetOpen(true)}
                />
              )}
            </div>

            <BuilderSide
              narrow={narrow}
              sheetOpen={sheetOpen}
              sheetRef={sheetRef}
              pickedLayer={pickedLayer}
              draft={draft}
              count={count}
              schemas={load.schemas}
              effects={effects}
              statuses={load.statuses}
              canvas={canvas}
              devices={devices}
              maskProblem={maskProblem}
              edits={edits}
              mask={mask}
              presets={presets}
              load={load}
              setEdit={setEdit}
              setSheetOpen={setSheetOpen}
              onOpenArea={area.open}
              chooseRef={chooseRef}
              onChooseEffectOpen={setChooseFor}
              onPropChange={(id, key, value) => {
                setDraft(
                  (current) => ({
                    ...current,
                    layers: current.layers.map((layer) =>
                      layer.id === id ? { ...layer, props: { ...layer.props, [key]: value } } : layer,
                    ),
                  }),
                  `props:${id}:${key}`,
                )
              }}
            />
          </div>
        </LoadFrame>
      )}

      <ChooseEffectPopover
        anchor={chooseRef}
        open={chooseFor !== null}
        effects={effects}
        pickedEffect={pickedLayer?.effect ?? null}
        html={html}
        onChoose={(effect) => {
          if (chooseFor) void edits.chooseEffect(chooseFor, effect);
        }}
        onClose={closeChoose}
      />

      <MaskPickerPopover
        anchor={mask.anchorRef}
        open={mask.maskFor !== null}
        layers={draft.layers}
        maskFor={mask.maskFor}
        leads={mask.maskList.leads}
        rest={mask.maskList.rest}
        active={mask.maskActive}
        listRef={mask.maskListRef}
        refusalOf={(effect) => (mask.maskFor ? maskRefusal(draft.layers, mask.maskFor, effect.id) : null)}
        onChoose={(effect) => {
          if (mask.maskFor) void mask.chooseMaskEffect(mask.maskFor, effect);
        }}
        onClose={mask.closeMaskPicker}
        onKey={mask.onMaskKey}
      />

      {area.pickerFor && (
        <StackAreaPicker
          layer={draft.layers.find((layer) => layer.id === area.pickerFor) ?? draft.layers[0]!}
          canvas={canvas}
          devices={devices}
          onUse={area.apply}
          onClose={area.close}
        />
      )}

      {saver.naming && (
        <NameStackDialog
          nameDraft={nameDraft}
          namingError={saver.namingError}
          onChange={(value) => {
            setNameDraft(value);
            saver.setNamingError(null);
          }}
          onEnter={saver.nameTheStack}
          onEscape={saver.escapeNaming}
          onCancel={saver.cancelNaming}
        />
      )}

      {saver.leaving && (
        <LeavePrompt
          promptRef={promptRef}
          saveState={saver.saveState}
          onKeepEditing={saver.keepEditing}
          onDiscard={saver.discardLeave}
          onSave={saver.saveLeave}
        />
      )}

      <DragGhost drag={dragSession.drag} />
      <DragHint drag={dragSession.drag} />
      <LiveRegion announce={announce} />
    </div>
  );
}

