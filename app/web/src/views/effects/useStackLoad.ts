"use client";

// The builder's read (#257 phase 4): the stack, every layer's effect schema, the layer statuses and
// the other stacks' names. Moved out of StackBuilder.tsx unchanged.

import { useCallback, useEffect, useRef, useState } from "react";
import type { LoadState } from "@/components/Loading";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import type { Preset } from "@/proto/Preset";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { Stack } from "@/proto/Stack";
import type { StackLayerStatus } from "@/proto/StackLayerStatus";

export function useStackLoad({
  initialName,
  isNew,
  mountedRef,
  areaDraggingRef,
  resetDraft,
  pick,
}: {
  initialName: string;
  isNew: boolean;
  mountedRef: { current: boolean };
  areaDraggingRef: { current: boolean };
  resetDraft: (next: Stack) => void;
  pick: (id: string | null) => void;
}) {
  const [phase, setPhase] = useState<LoadState>(isNew ? "ready" : "loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [base, setBase] = useState<Stack | null>(null);
  const [statuses, setStatuses] = useState<Record<string, StackLayerStatus>>({});
  const [schemas, setSchemas] = useState<Record<string, PropertySchema | null>>({});
  const [presets, setPresets] = useState<{ builtIn: Preset[]; user: Preset[] }>({ builtIn: [], user: [] });
  const [otherStacks, setOtherStacks] = useState<{ id: string; name: string }[]>([]);
  const schemasRef = useRef<Record<string, PropertySchema | null>>({});
  const fetching = useRef(new Set<string>());

  const loadPresets = useCallback((id: string) => {
    api
      .effect(id)
      .then((detail) => {
        if (!mountedRef.current) return;
        setPresets({ builtIn: detail.schema.presets, user: detail.user_presets });
      })
      .catch(() => {
        if (!mountedRef.current) return;
        setPresets({ builtIn: [], user: [] });
      });
  }, [mountedRef]);

  useEffect(() => {
    schemasRef.current = schemas;
  }, [schemas]);

  const loadDetails = useCallback((stack: Stack) => {
    for (const layer of stack.layers) {
      // A mask's effect has its own schema and values (#256), read the same way.
      const ids = [layer.effect, ...(layer.mask ? [layer.mask.effect] : [])];
      for (const id of ids) {
        if (schemasRef.current[id] !== undefined || fetching.current.has(id)) continue;
        fetching.current.add(id);
        api
          .effect(id)
          .then((detail) => {
            if (!mountedRef.current) return;
            setSchemas((current) => ({ ...current, [id]: detail.schema }));
          })
          .catch(() => {
            if (!mountedRef.current) return;
            setSchemas((current) => ({ ...current, [id]: null }));
          })
          .finally(() => fetching.current.delete(id));
      }
    }
  }, [mountedRef]);

  const load = useCallback(() => {
    // A drag cut off by a reload never sent its end: undo and redo are free again (#257).
    areaDraggingRef.current = false;
    if (isNew) {
      setPhase("ready");
      setBase(null);
      resetDraft({ id: "stack:", layers: [] });
      pick(null);
      return;
    }
    setPhase("loading");
    setLoadError(null);
    api
      .stack(initialName)
      .then((detail) => {
        if (!mountedRef.current) return;
        const stack: Stack = { id: detail.stack.id, layers: detail.stack.layers };
        setBase(stack);
        resetDraft(stack);
        setStatuses(Object.fromEntries(detail.layers.map((layer) => [layer.id, layer])));
        pick(stack.layers[stack.layers.length - 1]?.id ?? null);
        loadDetails(stack);
        loadPresets(stack.id);
        setPhase("ready");
      })
      .catch((e: unknown) => {
        if (!mountedRef.current) return;
        setLoadError(messageOf(e));
        setPhase("error");
      });
  }, [initialName, isNew, loadDetails, loadPresets, mountedRef, areaDraggingRef, pick, resetDraft]);
  useEffect(() => {
    mountedRef.current = true;
    // One tick after the first paint: the builder's own placeholder is on screen before the
    // read starts, and a load started here never calls a setter in the effect body.
    const timer = setTimeout(load, 0);
    api
      .stacks()
      .then((answer) => {
        if (mountedRef.current) setOtherStacks(answer.stacks.map((entry) => ({ id: entry.id, name: entry.name })));
      })
      .catch(() => undefined);
    return () => {
      clearTimeout(timer);
      mountedRef.current = false;
    };
    // The builder is mounted for one stack; opening another mounts a new one.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return {
    phase,
    loadError,
    base,
    setBase,
    statuses,
    setStatuses,
    schemas,
    setSchemas,
    schemasRef,
    presets,
    setPresets,
    loadPresets,
    otherStacks,
    setOtherStacks,
    loadDetails,
    load,
  };
}


