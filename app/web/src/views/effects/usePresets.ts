"use client";

// The builder's presets (#257 phase 4): the stack effect's built-in and user presets, the merged
// values a preset writes, and saving the current values. Moved out of StackBuilder.tsx unchanged.

import { useMemo, useState } from "react";
import { notify } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { applyMergedProps, mergedProps } from "@/views/effects/stackDraft";
import type { Preset } from "@/proto/Preset";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";

export function usePresets({
  layers,
  schemas,
  schemasRef,
  base,
  presets,
  setPresets,
  setDraft,
}: {
  layers: StackLayer[];
  schemas: Record<string, PropertySchema | null>;
  schemasRef: { current: Record<string, PropertySchema | null> };
  base: Stack | null;
  presets: { builtIn: Preset[]; user: Preset[] };
  setPresets: (update: (current: { builtIn: Preset[]; user: Preset[] }) => { builtIn: Preset[]; user: Preset[] }) => void;
  setDraft: (update: Stack | ((current: Stack) => Stack), run?: string, hold?: boolean) => void;
}) {
  const [presetName, setPresetName] = useState<string | null>(null);
  const [presetProblem, setPresetProblem] = useState<string | null>(null);

  const merged = useMemo(() => mergedProps(layers, schemas), [layers, schemas]);
  const appliedPreset = useMemo(() => {
    const all = [...presets.builtIn, ...presets.user];
    return (
      all.find((preset) => {
        const keys = Object.keys(preset.props);
        return keys.length > 0 && keys.every((key) => JSON.stringify(merged[key]) === JSON.stringify(preset.props[key]));
      })?.name ?? null
    );
  }, [presets, merged]);

  const applyPreset = (preset: Preset) => {
    setDraft((current) => ({
      ...current,
      layers: applyMergedProps(current.layers, preset.props, schemasRef.current),
    }));
  };

  const savePreset = async () => {
    const wanted = presetName?.trim() ?? "";
    if (!wanted || !base) return;
    try {
      const answer = await api.createPreset(base.id, { name: wanted, props: merged });
      setPresets((current) => ({ ...current, user: answer.user_presets }));
      setPresetName(null);
    } catch (e) {
      notify(messageOf(e), "down");
    }
  };

  return {
    presetName,
    setPresetName,
    presetProblem,
    setPresetProblem,
    appliedPreset,
    applyPreset,
    savePreset,
  };
}
