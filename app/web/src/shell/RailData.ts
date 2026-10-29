"use client";

import { useMemo } from "react";
import { useSnap } from "@/lib/store";
import { usePeripheralCount } from "@/shell/peripheralCount";

/**
 * What the rail's rows say about the app right now.
 *
 * Every value comes from the daemon's own answers: the status, the devices, the effects, the
 * layouts, the fans, the displays and the peripherals. A value the daemon has not given is
 * absent, and the row that would carry it says nothing - a made-up figure is worse than a
 * quiet one.
 */
export interface RailValues {
  /** The active layout's name. */
  layout: string | null;
  /** The running effect's name. */
  effect: string | null;
  /** The lights are paused. */
  paused: boolean;
  /** The devices that are connected, and how many there are. */
  devices: { connected: number; total: number } | null;
  /** How many numeric displays the daemon sees. */
  displays: number | null;
  /** How many peripherals the adapters see. */
  peripherals: number | null;
  /** "BIOS" when the chip is under BIOS control, or the fan profile's name. */
  fans: string | null;
}

export function useRailData(): RailValues {
  const status = useSnap((s) => s.status);
  const effects = useSnap((s) => s.effects);
  const devices = useSnap((s) => s.devices);
  const displays = useSnap((s) => s.displays);
  const layouts = useSnap((s) => s.layouts);
  const activeLayout = useSnap((s) => s.activeLayout);
  const fans = useSnap((s) => s.fans);
  const peripherals = usePeripheralCount();

  return useMemo(() => {
    // The status names the active layout; the layouts list names it.
    const layoutId = status?.active_layout ?? activeLayout;
    const layout = layoutId
      ? (layouts?.find((l) => l.id === layoutId)?.name ?? layoutId)
      : null;

    // The status names the active effect; the effects list names it.
    const effectId = status?.active_effect ?? null;
    const effect = effectId
      ? (effects?.find((e) => e.id === effectId)?.name ?? effectId)
      : null;

    const deviceCounts = devices
      ? { connected: devices.filter((d) => d.connected).length, total: devices.length }
      : null;

    // The fans row says who is in charge of the chip: the BIOS, or the profile that is
    // running. A chip with no headers under the app's hand has nothing to say.
    const fanWord =
      fans && fans.headers.length > 0
        ? fans.headers.every((h) => h.on_bios)
          ? "BIOS"
          : fans.active_profile
        : null;

    return {
      layout,
      effect,
      paused: status?.power === "paused",
      devices: deviceCounts,
      displays: displays ? displays.length : null,
      peripherals,
      fans: fanWord,
    };
  }, [status, effects, devices, displays, layouts, activeLayout, fans, peripherals]);
}
