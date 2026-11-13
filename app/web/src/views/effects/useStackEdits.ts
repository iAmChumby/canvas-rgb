"use client";

// The builder's layer edits (#257 phase 4): patch, make rectangle, choose an effect, add, remove,
// duplicate, reorder and rename. Moved out of StackBuilder.tsx unchanged; every change is one
// draft step, and an add waits its turn so two fast clicks cannot share an id or pass the HTML cap.

import { useRef, useState } from "react";
import { notify } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import type { StarterStack } from "@/lib/demoStacks";
import { freeLayerName } from "@/views/effects/layerText";
import {
  addLayer,
  addLayerRefusal,
  areaAsRect,
  duplicateLayer,
  htmlLayers,
  insertLayerAt,
  isHtmlEffect,
  layerNameProblem,
  layerValues,
  makeLayer,
  makeMask,
  moveLayer,
  nextLayerId,
  patchLayer,
  removeLayer,
  MAX_HTML_LAYERS,
  MAX_LAYERS,
} from "@/views/effects/stackDraft";
import type { LayerArea } from "@/proto/LayerArea";
import type { GroupCanvas } from "@/lib/groupCanvas";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";

export function useStackEdits({
  draft,
  picked,
  canvas,
  draftRef,
  mounted,
  historyMoves,
  setDraft,
  pick,
  setSheetOpen,
  setSchemas,
}: {
  draft: Stack;
  picked: string | null;
  canvas: GroupCanvas | null;
  draftRef: { current: Stack };
  mounted: { current: boolean };
  historyMoves: { current: number };
  setDraft: (update: Stack | ((current: Stack) => Stack), run?: string, hold?: boolean) => void;
  pick: (id: string | null) => void;
  setSheetOpen: (open: boolean) => void;
  setSchemas: (update: (current: Record<string, PropertySchema | null>) => Record<string, PropertySchema | null>) => void;
}) {
  const [layerProblem, setLayerProblem] = useState<string | null>(null);
  const [addProblem, setAddProblem] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameText, setRenameText] = useState("");
  const [renameProblem, setRenameProblem] = useState<string | null>(null);
  // One add and one effect choice at a time: adds wait their turn in `addChain`, so two fast
  // clicks cannot share a layer id or pass the HTML cap twice.
  const addChain = useRef<Promise<void>>(Promise.resolve());
  const addQueued = useRef(new Set<string>());
  const choosing = useRef(false);
  /** The starters waiting in `addChain` or being built: a second click on the same one is the same add. */
  const starterQueued = useRef(new Set<string>());

  const patch = (id: string, change: Partial<StackLayer>, run?: string, hold = false) => {
    setDraft((current) => ({ ...current, layers: patchLayer(current.layers, id, change) }), run, hold);
  };

  /**
   * A refusal from a key on the stage: the inspector shows it where it always has, but the
   * inspector may be off screen (a short window, a phone), so it is also a note.
   */
  const refuse = (words: string) => {
    setLayerProblem(words);
    notify(words, "warn");
  };

  /** The picked layer's area made into the rectangle it covers now, by its own key's name (#257). */
  const makeRect = (id: string) => {
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    if (!layer) return;
    const rect = areaAsRect(layer.area, canvas);
    if (!rect) {
      refuse("That area is not on the desk, so there is no rectangle to make.");
      return;
    }
    setLayerProblem(null);
    patch(id, { area: rect });
  };

  const chooseEffect = async (id: string, effect: string) => {
    if (choosing.current) return;
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    if (!layer || effect === layer.effect) return;
    if (isHtmlEffect(effect) && !isHtmlEffect(layer.effect) && htmlLayers(draftRef.current.layers) >= MAX_HTML_LAYERS) {
      setLayerProblem(`A stack holds ${MAX_HTML_LAYERS} HTML layers`);
      return;
    }
    setLayerProblem(null);
    choosing.current = true;
    try {
      let detail = null;
      try {
        detail = await api.effect(effect);
        if (!mounted.current) return;
        setSchemas((current) => ({ ...current, [effect]: detail!.schema }));
      } catch {
        if (!mounted.current) return;
        setSchemas((current) => ({ ...current, [effect]: null }));
      }
      // The draft may have moved while the effect was read: check the cap again.
      const latest = draftRef.current.layers.find((entry) => entry.id === id);
      if (!latest || effect === latest.effect) return;
      if (isHtmlEffect(effect) && !isHtmlEffect(latest.effect) && htmlLayers(draftRef.current.layers) >= MAX_HTML_LAYERS) {
        setLayerProblem(`A stack holds ${MAX_HTML_LAYERS} HTML layers`);
        return;
      }
      patch(id, { effect, props: detail ? layerValues(detail.schema, detail.props) : {} });
    } finally {
      choosing.current = false;
    }
  };

  const addOne = async (effect: string, place?: { area?: LayerArea; at?: number }) => {
    const refusal = addLayerRefusal(draftRef.current.layers, effect);
    if (refusal) {
      setAddProblem(refusal);
      notify(refusal, "warn");
      return;
    }
    setAddProblem(null);
    let schema: PropertySchema | null = null;
    let props: Record<string, PropValue> = {};
    try {
      const detail = await api.effect(effect);
      schema = detail.schema;
      props = detail.props;
      setSchemas((current) => ({ ...current, [effect]: detail.schema }));
    } catch {
      setSchemas((current) => ({ ...current, [effect]: null }));
    }
    if (!mounted.current) return;
    // The draft may have moved while the effect was read: check the cap and pick the id
    // against the latest draft, so two fast clicks cannot exceed the HTML cap or share an id.
    const layers = draftRef.current.layers;
    const lateRefusal = addLayerRefusal(layers, effect);
    if (lateRefusal) {
      setAddProblem(lateRefusal);
      notify(lateRefusal, "warn");
      return;
    }
    const id = nextLayerId(layers);
    if (!id) return;
    const layer: StackLayer = {
      ...makeLayer(effect, freeLayerName(schema?.name ?? effect, layers), schema, props),
      id,
      ...(place?.area ? { area: place.area } : {}),
    };
    // The ref is updated with the draft, so a queued add sees this layer.
    const next = {
      ...draftRef.current,
      layers:
        place?.at === undefined ? addLayer(layers, layer) : insertLayerAt(layers, layer, place.at),
    };
    draftRef.current = next;
    setDraft(next);
    pick(id);
  };

  const addEffect = (effect: string, place?: { area?: LayerArea; at?: number }) => {
    // One add at a time, in the order the clicks came (D95.5.3). A second click on a different
    // row waits its turn and lands; a second click on the same row is the same add.
    if (addQueued.current.has(effect)) return;
    addQueued.current.add(effect);
    const run = addChain.current.then(async () => {
      try {
        await addOne(effect, place);
      } finally {
        addQueued.current.delete(effect);
      }
    });
    addChain.current = run.catch(() => undefined);
    return run;
  };

  /**
   * A starter stack (#257 phase 4, spec §5): every layer is read and built first, then the whole
   * stack lands in one draft change, so one undo takes it all back. A click on another starter
   * while the effects are read waits its turn in `addChain`, as `addEffect` does, and lands after
   * it; a second click on the same starter is the same add. An Undo or a reset in that gap
   * abandons the add rather than overwriting the step back.
   */
  const addStack = (starter: StarterStack): Promise<void> => {
    if (starterQueued.current.has(starter.id)) return Promise.resolve();
    starterQueued.current.add(starter.id);
    // The history as it stood at the click: an Undo while this add waits behind another one
    // abandons it too (#257 adversarial review).
    const moves = historyMoves.current;
    const run = addChain.current.then(async () => {
      try {
        const room = MAX_LAYERS - draftRef.current.layers.length;
        if (starter.layers.length > room) {
          const refusal = `A stack holds ${MAX_LAYERS} layers`;
          setAddProblem(refusal);
          notify(refusal, "warn");
          return;
        }
        if (historyMoves.current !== moves) return;
        setAddProblem(null);
        const ids = new Set<string>();
        for (const def of starter.layers) {
          ids.add(def.effect);
          if (def.mask) ids.add(def.mask.effect);
        }
        const details = new Map<string, { schema: PropertySchema | null; props: Record<string, PropValue> }>();
        await Promise.all(
          [...ids].map(async (id) => {
            try {
              const detail = await api.effect(id);
              if (mounted.current) setSchemas((current) => ({ ...current, [id]: detail.schema }));
              details.set(id, { schema: detail.schema, props: detail.props });
            } catch {
              if (mounted.current) setSchemas((current) => ({ ...current, [id]: null }));
              details.set(id, { schema: null, props: {} });
            }
          }),
        );
        if (!mounted.current) return;
        if (historyMoves.current !== moves) return;
        const layers = draftRef.current.layers;
        if (starter.layers.length > MAX_LAYERS - layers.length) {
          const refusal = `A stack holds ${MAX_LAYERS} layers`;
          setAddProblem(refusal);
          notify(refusal, "warn");
          return;
        }
        let working = [...layers];
        const added: StackLayer[] = [];
        for (const def of starter.layers) {
          const id = nextLayerId(working);
          if (!id) break;
          const detail = details.get(def.effect);
          const maskDetail = def.mask ? details.get(def.mask.effect) : undefined;
          const layer: StackLayer = {
            ...makeLayer(def.effect, freeLayerName(def.name, working), detail?.schema, {
              ...(detail?.props ?? {}),
              ...(def.props ?? {}),
            }),
            id,
            ...(def.blend ? { blend: def.blend } : {}),
            ...(def.opacity !== undefined ? { opacity: def.opacity } : {}),
            ...(def.area ? { area: def.area } : {}),
            ...(def.mask
              ? {
                  mask: makeMask(def.mask.effect, maskDetail?.schema, {
                    ...(maskDetail?.props ?? {}),
                    ...(def.mask.props ?? {}),
                  }),
                }
              : {}),
          };
          working = [...working, layer];
          added.push(layer);
        }
        if (added.length === 0) return;
        const next = { ...draftRef.current, layers: working };
        draftRef.current = next;
        setDraft(next);
        pick(added[added.length - 1]!.id);
      } finally {
        starterQueued.current.delete(starter.id);
      }
    });
    addChain.current = run.catch(() => undefined);
    return run;
  };

  const remove = (id: string) => {
    const index = draft.layers.findIndex((layer) => layer.id === id);
    const next = removeLayer(draft.layers, id);
    setDraft((current) => ({ ...current, layers: next }));
    if (picked === id) {
      pick(next[Math.min(index, next.length - 1)]?.id ?? null);
      // The sheet is the removed layer's: it closes rather than showing the next one unsought.
      setSheetOpen(false);
    }
  };

  const duplicate = (id: string) => {
    if (draft.layers.length >= MAX_LAYERS) {
      refuse(`A stack holds ${MAX_LAYERS} layers`);
      return;
    }
    const answer = duplicateLayer(draft.layers, id);
    if (!answer.id) {
      refuse(`A stack holds ${MAX_HTML_LAYERS} HTML layers`);
      return;
    }
    setLayerProblem(null);
    setDraft((current) => ({ ...current, layers: answer.layers }));
    pick(answer.id);
  };

  const reorder = (id: string, to: number) => {
    setDraft((current) => ({ ...current, layers: moveLayer(current.layers, id, to) }));
  };

  const beginRename = (layer: StackLayer) => {
    setRenaming(layer.id);
    setRenameText(layer.name);
    setRenameProblem(null);
  };

  const commitRename = () => {
    if (!renaming) return;
    const problem = layerNameProblem(renameText.trim());
    if (problem) {
      setRenameProblem(problem);
      return;
    }
    setRenameProblem(null);
    patch(renaming, { name: renameText.trim() });
    setRenaming(null);
  };

  return {
    patch,
    refuse,
    makeRect,
    chooseEffect,
    addOne,
    addEffect,
    addStack,
    remove,
    duplicate,
    reorder,
    beginRename,
    commitRename,
    renaming,
    setRenaming,
    renameText,
    setRenameText,
    renameProblem,
    setRenameProblem,
    layerProblem,
    setLayerProblem,
    addProblem,
    setAddProblem,
  };
}
