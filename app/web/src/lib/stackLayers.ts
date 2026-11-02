import type { EffectDetail } from "@/proto/EffectDetail";

/**
 * The layer names a stack's schema carries: the daemon labels each property
 * `{layer} · {property}` (D95.4.3), so a layer's name is everything before the last
 * separator and is never split on one it holds.
 */
export function stackLayerNames(schema: EffectDetail["schema"]): string[] {
  const names: string[] = [];
  for (const def of schema.properties) {
    const at = def.label.lastIndexOf(" · ");
    const name = at === -1 ? def.label : def.label.slice(0, at);
    if (name && !names.includes(name)) names.push(name);
  }
  if (names.length === 0) {
    // A stack whose layers have no properties still names them in its description.
    const after = schema.description.split(": ")[1];
    if (after) return after.split(", ");
  }
  return names;
}
