// Which light the pattern editor's shape picker writes to (#468).
//
// A pattern is built on a kind of device (a count and a shape), and the shape is `Light.arrangement`:
// the owner says whether a line of LEDs is a line or a ring. So the picker in the editor's header
// writes the arrangement of the light the pattern was built on: the connected light with this many
// LEDs, preferring one already in the pattern's shape. No such light (a pattern for a device that is
// off the machine, or a document looked at in memory) leaves the picker changing the pattern alone.

import { shapesFor, type Shape } from "@/components/ShapePicker";
import type { BuiltOn } from "@/proto/BuiltOn";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { LightView } from "@/proto/LightView";
import type { ZoneInfo } from "@/proto/ZoneInfo";

/**
 * The light a pattern built on `builtOn` is laid out by, or null when none of the lights fits: one
 * with this many LEDs and, for a grid, a matrix zone of the same cells.
 */
export function referenceLight(builtOn: BuiltOn, lights: LightView[] | null, devices: DeviceInfo[] | null = null): LightView | null {
  const fits = (view: LightView): boolean => {
    if (view.led_count !== builtOn.led_count || view.led_count <= 0) return false;
    if (!builtOn.grid) return true;
    const zone = zoneOf(view, devices);
    return zone?.kind.type === "matrix" && zone.kind.width === builtOn.grid.width && zone.kind.height === builtOn.grid.height;
  };
  const rank = (view: LightView) => (view.connected ? 0 : 2) + (view.light.arrangement === builtOn.arrangement ? 0 : 1);
  return (lights ?? []).filter(fits).sort((a, b) => rank(a) - rank(b))[0] ?? null;
}

/** The zone a light is, from the devices the daemon lists. */
export function zoneOf(view: LightView | null, devices: DeviceInfo[] | null): ZoneInfo | undefined {
  if (!view) return undefined;
  return devices?.find((d) => d.id === view.light.device_id)?.zones.find((z) => z.index === view.light.zone_index);
}

/**
 * The shapes the header offers for this pattern: the zone's, else a line and a ring (and a grid when
 * the pattern is one). The shape the pattern is in now is always among them, so the picker never
 * shows no choice selected.
 */
export function shapesOf(builtOn: BuiltOn, zone: ZoneInfo | undefined): Shape[] {
  if (builtOn.arrangement === "single" && !zone) return [];
  const offered = zone ? shapesFor(zone.kind.type) : builtOn.grid ? shapesFor("matrix") : shapesFor("linear");
  const now: Shape = builtOn.arrangement === "ring" || builtOn.arrangement === "grid" ? builtOn.arrangement : "strip";
  return offered.length > 0 && !offered.includes(now) ? [...offered, now] : offered;
}

/** The cells a grid shape takes from a zone, when the zone is a matrix. */
export function gridOfZone(zone: ZoneInfo | undefined): { width: number; height: number } | undefined {
  return zone?.kind.type === "matrix" ? { width: zone.kind.width, height: zone.kind.height } : undefined;
}
