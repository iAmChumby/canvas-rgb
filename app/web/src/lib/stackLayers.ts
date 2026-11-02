import type { EffectDetail } from "@/proto/EffectDetail";

/**
 * The layer names a stack's schema carries: the daemon labels each property
 * `{layer} · {property}` (D95.4.3), so a layer's name is everything before the first
 * separator. A layer's name cannot hold one (the daemon's name rule allows letters, digits,
 * spaces, `-`, `_` and `.`), but a user effect's property label can, so the first separator
 * is the boundary.
 */
export function stackLayerNames(schema: EffectDetail["schema"]): string[] {
  const names: string[] = [];
  for (const def of schema.properties) {
    const at = def.label.indexOf(" · ");
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
