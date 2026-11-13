// A group canvas built by hand, for the pure tests: the arrangement is the real one, and the
// boxes are the ones the test wants to reason about.

import type { GroupCanvas } from "@/lib/groupCanvas";

export interface FixtureLight {
  id: string;
  label: string;
  device: string;
  zone?: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** One canvas, flow across, with the boxes given. */
export function canvasWith(lights: FixtureLight[]): GroupCanvas {
  const deviceIds: string[] = [];
  for (const light of lights) if (!deviceIds.includes(light.device)) deviceIds.push(light.device);
  return {
    id: "fixture",
    name: "Fixture",
    mode: "flow_across",
    deviceIds,
    lights: lights.map((light) => ({
      id: light.id,
      label: light.label,
      device_id: light.device,
      zone_index: light.zone ?? 0,
      arrangement: "strip",
      reversed: false,
      box: { x: light.x, y: light.y, width: light.width, height: light.height },
      leds: [{ x: light.x + light.width / 2, y: light.y + light.height / 2 }],
    })),
  };
}
