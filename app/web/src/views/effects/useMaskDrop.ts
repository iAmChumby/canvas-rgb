"use client";

// The mask picker and the mask drop (#256, #257 phase 3), moved out of StackBuilder.tsx unchanged
// in #257 phase 4. A mask is another effect run over the layer's area; dropping a Masks tile on a
// row or the picture gives the layer that mask in one undo step, and a drop that lands after an
// Undo is abandoned so it cannot overwrite the step back.

import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { api } from "@/lib/endpoints";
import { closePopover } from "@/views/effects/popoverFocus";
import {
  makeMask,
  maskRefusal,
  patchMask,
  setLayerMask,
  MASK_LEAD_EFFECTS,
} from "@/views/effects/stackDraft";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { LayerMask } from "@/proto/LayerMask";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Stack } from "@/proto/Stack";

export function useMaskDrop({
  draft,
  draftRef,
  effects,
  mounted,
  historyMoves,
  maskProblem,
  setMaskProblem,
  setDraft,
  setSchemas,
  setAnnounce,
}: {
  draft: Stack;
  draftRef: { current: Stack };
  effects: EffectSummary[] | null;
  mounted: { current: boolean };
  historyMoves: { current: number };
  maskProblem: string | null;
  setMaskProblem: (words: string | null) => void;
  setDraft: (update: Stack | ((current: Stack) => Stack), run?: string, hold?: boolean) => void;
  setSchemas: (update: (current: Record<string, PropertySchema | null>) => Record<string, PropertySchema | null>) => void;
  setAnnounce: (words: string) => void;
}) {
  const [maskFor, setMaskFor] = useState<string | null>(null);
  /** The row the mask picker's keyboard cursor is on: an index into leads + rest (#256). */
  const [maskActive, setMaskActive] = useState(0);
  const maskAnchor = useRef<HTMLElement | null>(null);

  /** The mask picker's list (#256): the six effects that read well as masks, then every other. */
  const maskList = useMemo(() => {
    const all = (effects ?? []).filter((effect) => !effect.id.startsWith("stack:"));
    const leads: EffectSummary[] = [];
    for (const id of MASK_LEAD_EFFECTS) {
      const effect = all.find((entry) => entry.id === id);
      if (effect) leads.push(effect);
    }
    const leadIds = new Set(leads.map((effect) => effect.id));
    return { leads, rest: all.filter((effect) => !leadIds.has(effect.id)) };
  }, [effects]);

  /** The picker's rows in the order they are drawn: the six leads, then the rest. */
  const maskChoices = useMemo(() => [...maskList.leads, ...maskList.rest], [maskList]);

  const maskListRef = useRef<HTMLUListElement>(null);

  // The mask picker's listbox takes the focus when it opens, and its keyboard cursor starts on
  // the layer's current mask (the APG listbox pattern, #256).
  useEffect(() => {
    if (maskFor === null) return;
    maskListRef.current?.focus();
  }, [maskFor, maskListRef]);

  // The active row scrolls into view as the arrow keys move it.
  useEffect(() => {
    if (maskFor === null) return;
    const effect = maskChoices[maskActive];
    if (effect) {
      document.getElementById(`mask-option-${effect.id}`)?.scrollIntoView({ block: "nearest" });
    }
  }, [maskActive, maskChoices, maskFor]);

  /** The mask picker hangs under the control that opened it, and returns focus there on Escape. */
  const openMaskPicker = (id: string, nextAnchor: HTMLElement) => {
    maskAnchor.current = nextAnchor;
    setMaskProblem(null);
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    const at = maskChoices.findIndex((effect) => effect.id === layer?.mask?.effect);
    setMaskActive(at >= 0 ? at : 0);
    setMaskFor(id);
  };

  const closeMaskPicker = () => {
    closePopover(".rd-mask-picker", maskAnchor, () => setMaskFor(null));
  };

  const removeMask = (id: string) => {
    setMaskProblem(null);
    setDraft((current) => ({ ...current, layers: setLayerMask(current.layers, id, null) }));
  };

  const patchMaskDraft = (id: string, change: Partial<LayerMask>, run?: string) => {
    setDraft((current) => ({ ...current, layers: patchMask(current.layers, id, change) }), run);
  };

  /** One mask property changed: a functional patch, so two quick changes cannot drop one (#256). */
  const patchMaskProps = (id: string, key: string, value: PropValue) => {
    setDraft(
      (current) => ({
        ...current,
        layers: current.layers.map((layer) =>
          layer.id === id && layer.mask !== undefined
            ? { ...layer, mask: { ...layer.mask, props: { ...layer.mask.props, [key]: value } } }
            : layer,
        ),
      }),
      `mask:${id}:${key}`,
    );
  };

  /** A mask effect's schema and starting values, read from the daemon; nothing when it is gone. */
  const maskOf = async (effect: string): Promise<{ schema: PropertySchema | null; props: Record<string, PropValue> }> => {
    try {
      const detail = await api.effect(effect);
      if (mounted.current) setSchemas((current) => ({ ...current, [effect]: detail.schema }));
      return { schema: detail.schema as PropertySchema | null, props: detail.props };
    } catch {
      if (mounted.current) setSchemas((current) => ({ ...current, [effect]: null }));
      return { schema: null, props: {} };
    }
  };

  /** Gives a layer a mask of `effect`, or changes the mask it has (#256). */
  const chooseMaskEffect = async (id: string, effect: string) => {
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    if (!layer) return;
    if (layer.mask?.effect === effect) {
      closeMaskPicker();
      return;
    }
    const refusal = maskRefusal(draftRef.current.layers, id, effect);
    if (refusal) {
      setMaskProblem(refusal);
      return;
    }
    setMaskProblem(null);
    const { schema, props } = await maskOf(effect);
    if (!mounted.current) return;
    // The draft may have moved while the effect was read: check the cap again.
    const latest = draftRef.current.layers.find((entry) => entry.id === id);
    if (!latest) return;
    const lateRefusal = maskRefusal(draftRef.current.layers, id, effect);
    if (lateRefusal) {
      setMaskProblem(lateRefusal);
      return;
    }
    setDraft((current) => ({
      ...current,
      layers: setLayerMask(current.layers, id, makeMask(effect, schema, props)),
    }));
    closeMaskPicker();
  };

  /**
   * A mask dropped on a layer (#257 phase 3): the layer a row or the picture names gets it,
   * replacing the mask it had in the same single step. Nothing changes when the drop is refused.
   */
  const applyMask = async (id: string, effect: string): Promise<boolean> => {
    const layer = draftRef.current.layers.find((entry) => entry.id === id);
    if (!layer || layer.mask?.effect === effect) return false;
    const refusal = maskRefusal(draftRef.current.layers, id, effect);
    if (refusal) {
      setAnnounce(refusal);
      return false;
    }
    // An Undo, a Redo or a reset while the effect is read must not be overwritten and its redo
    // must survive, so the drop is abandoned then. Any other edit in that gap (a slider, a
    // handle) is kept: the mask lands on top of it, by layer id (#257 review).
    const moves = historyMoves.current;
    const { schema, props } = await maskOf(effect);
    if (!mounted.current) return false;
    if (historyMoves.current !== moves) return false;
    const latest = draftRef.current.layers.find((entry) => entry.id === id);
    if (!latest) return false;
    const lateRefusal = maskRefusal(draftRef.current.layers, id, effect);
    if (lateRefusal) {
      setAnnounce(lateRefusal);
      return false;
    }
    setDraft((current) => ({
      ...current,
      layers: setLayerMask(current.layers, id, makeMask(effect, schema, props)),
    }));
    return true;
  };

  /** The mask picker's keys, as a listbox: arrows move, Home/End jump, Enter/Space choose. */
  const onMaskKey = (event: ReactKeyboardEvent<HTMLUListElement>) => {
    const last = maskChoices.length - 1;
    if (last < 0) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      const effect = maskChoices[maskActive];
      if (effect && maskFor) void chooseMaskEffect(maskFor, effect.id);
      return;
    }
    let next: number | null = null;
    if (event.key === "ArrowDown") next = Math.min(last, maskActive + 1);
    else if (event.key === "ArrowUp") next = Math.max(0, maskActive - 1);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = last;
    if (next === null) return;
    event.preventDefault();
    setMaskActive(next);
  };

  /** The rows the picker draws, each with its refusal and whether it is the layer's current mask. */
  const maskRowState = (effect: EffectSummary, index: number) => {
    const refusal = maskFor ? maskRefusal(draft.layers, maskFor, effect.id) : null;
    const current = maskFor ? draft.layers.find((layer) => layer.id === maskFor)?.mask?.effect === effect.id : false;
    return { effect, index, refusal, current };
  };

  return {
    maskFor,
    maskActive,
    setMaskActive,
    maskProblem,
    setMaskProblem,
    maskList,
    maskChoices,
    maskRowState,
    maskListRef,
    anchorRef: maskAnchor,
    openMaskPicker,
    closeMaskPicker,
    chooseMaskEffect,
    applyMask,
    removeMask,
    patchMaskDraft,
    patchMaskProps,
    onMaskKey,
  };
}
