// The stack builder's words: the blend list, a layer's effect name, and a free name for a new
// layer. Split out of StackBuilder.tsx in #257 phase 4 so the layer list and the inspector say
// these the same way.

import type { BlendMode } from "@/proto/BlendMode";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { StackLayer } from "@/proto/StackLayer";

export const BLENDS: { value: BlendMode; label: string; note: string }[] = [
  { value: "normal", label: "Normal", note: "Normal: covers what is under it; a black part hides it." },
  { value: "add", label: "Add", note: "Add: lightens what is under it; a black part leaves it alone." },
  { value: "multiply", label: "Multiply", note: "Multiply: darkens what is under it; a black part makes it black." },
  { value: "screen", label: "Screen", note: "Screen: lifts what is under it; a black part is invisible." },
];

export function blendOf(blend: BlendMode): string {
  return BLENDS.find((entry) => entry.value === blend)?.label ?? "Normal";
}

export function nameOf(id: string): string {
  return id.startsWith("stack:") ? id.slice("stack:".length) : id;
}

/** An effect's name for a row and the summary: the schema's, then the library's, and never the
 *  raw id (its fields arrive after the stack's own read). */
export function layerLabel(
  schemas: Record<string, PropertySchema | null>,
  effects: EffectSummary[] | null,
  effect: string,
): string | null {
  const schema = schemas[effect];
  if (schema?.name) return schema.name;
  const listed = effects?.find((entry) => entry.id === effect)?.name;
  if (listed) return listed;
  // null: the name has not arrived. The caller draws an InlineSkeleton, never a word standing in for it.
  return schema === null ? "Missing effect" : null;
}

/** A name for a new layer that is not already one of the stack's. */
export function freeLayerName(base: string, layers: StackLayer[]): string {
  const used = new Set(layers.map((layer) => layer.name.toLowerCase()));
  if (!used.has(base.toLowerCase())) return base;
  for (let n = 2; n < 99; n++) {
    const candidate = `${base} ${n}`;
    if (!used.has(candidate.toLowerCase())) return candidate;
  }
  return base;
}
