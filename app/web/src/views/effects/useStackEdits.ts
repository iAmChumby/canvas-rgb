"use client";

// The builder's layer edits (#257 phase 4): patch, make rectangle, choose an effect, add, remove,
// duplicate, reorder and rename. Moved out of StackBuilder.tsx unchanged; every change is one
// draft step, and an add waits its turn so two fast clicks cannot share an id or pass the HTML cap.

import { useRef, useState } from "react";
import { notify } from "@/components/Notify";
import { api } from "@/lib/endpoints";
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
  moveLayer,
  nextLayerId,
  patchLayer,
  removeLayer,
  MAX_HTML_LAYERS,
  MAX_LAYERS,
} from "@/views/effects/stackDraft";
import type { LayerArea } from "@/proto/LayerArea";
import type { Layout } from "@/proto/Layout";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";

export function useStackEdits({
  draft,
  picked,
  layout,
  draftRef,
  mounted,
  setDraft,
  pick,
  setSheetOpen,
  setSchemas,
}: {
  draft: Stack;
  picked: string | null;
  layout: Layout | null;
  draftRef: { current: Stack };
  mounted: { current: boolean };
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
    const rect = areaAsRect(layer.area, layout);
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
