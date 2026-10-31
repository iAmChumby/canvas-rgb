// The preset an effect is running as, shared by the Effects view (which knows the live values)
// and the rail deck (which shows the name). The daemon stores props only, never a preset name, so
// "which preset" is client knowledge: the name the owner applied or saved last, and whether the
// live values still equal it.

import { useSyncExternalStore } from "react";
import type { EffectDetail } from "@/proto/EffectDetail";
import { matchingPreset } from "@/lib/presetMatch";

export interface RunningPreset {
  /** The preset's name. */
  name: string;
  /** The live values differ from the preset's. */
  modified: boolean;
}

/** The preset the owner chose last, per effect. In memory: a reload forgets it and matches again. */
const applied = new Map<string, string>();
const shown = new Map<string, RunningPreset | null>();
const listeners = new Set<() => void>();
let version = 0;

function emit() {
  version += 1;
  for (const l of listeners) l();
}

/** The name applied last for an effect, if the effect still has it. */
export function appliedName(effectId: string): string | null {
  return applied.get(effectId) ?? null;
}

export function setApplied(effectId: string, name: string | null) {
  if (name === null) applied.delete(effectId);
  else applied.set(effectId, name);
}

export function setShown(effectId: string, value: RunningPreset | null) {
  const before = shown.get(effectId);
  if (before?.name === value?.name && before?.modified === value?.modified && shown.has(effectId)) return;
  shown.set(effectId, value);
  emit();
}

/** Which preset the detail's live values mean: a match, else the one applied last, modified. */
export function runningOf(detail: EffectDetail, live = detail.props): RunningPreset | null {
  const defs = detail.schema.properties;
  const all = [...detail.schema.presets, ...detail.user_presets];
  const last = appliedName(detail.schema.id);
  const lastPreset = last ? all.find((p) => p.name === last) ?? null : null;
  // The one applied last wins a tie, so two presets with equal values do not swap names.
  const equalLast = lastPreset && matchingPreset(defs, [lastPreset], live);
  if (equalLast) return { name: equalLast.name, modified: false };
  const match = matchingPreset(defs, all, live);
  if (match) return { name: match.name, modified: false };
  if (lastPreset) return { name: lastPreset.name, modified: true };
  return null;
}

/** The running preset of an effect, live. */
export function useRunningPreset(effectId: string | null): RunningPreset | null {
  useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => version,
    () => 0,
  );
  return effectId ? (shown.get(effectId) ?? null) : null;
}
