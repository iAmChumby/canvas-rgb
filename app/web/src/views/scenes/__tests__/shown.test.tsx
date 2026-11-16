// A header whose devices mirror each other is one light shown several times (#382). Scenes calls a
// light a device, so a mirrored light is "1 device" there and has to say "shown 8 times" wherever
// the screen prints a light's count: the group row's summary, the member list, the list of
// devices to add and the screen reader's line for a run.

import { cleanup, render, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/endpoints", () => ({ api: { lights: { flash: vi.fn(), list: vi.fn(), patch: vi.fn() } } }));

import { DEMO_LIGHTS } from "@/lib/fixtures";
import type { LightView } from "@/proto/LightView";
import { AddDevices } from "@/views/scenes/AddDevices";
import { memberSummary } from "@/views/scenes/GroupRow";
import { runLabel } from "@/views/scenes/LedRun";
import type { UseScene } from "@/views/scenes/useScene";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => cleanup());

/** The same 8 LEDs, shown 8 times: Luke's JARGB 1. */
function mirrored(): LightView {
  const base = DEMO_LIGHTS[0] as LightView;
  return {
    ...structuredClone(base),
    light: { ...structuredClone(base.light), id: "mirror#z1p1", name: "Same lights, shown 8 times", copies: 8 },
    led_count: 8,
    label: "Same lights, shown 8 times",
  };
}

describe("a mirrored light is shown several times", () => {
  it("the group row's summary adds the times for a group of that one light", () => {
    const one = memberSummary([mirrored()]);
    expect(one.shown).toBe("shown 8 times");
    expect(one.leds).toBe(8);
    expect(memberSummary([DEMO_LIGHTS[0] as LightView]).shown).toBe("");
    expect(memberSummary([mirrored(), DEMO_LIGHTS[1] as LightView]).shown).toBe("");
  });

  it("the screen reader's line for a run names the mirrored light", () => {
    expect(runLabel("Case", [mirrored()])).toBe(
      "Case: 1 device, 8 LEDs, Same lights, shown 8 times shown 8 times",
    );
    expect(runLabel("Case", [DEMO_LIGHTS[0] as LightView])).not.toMatch(/shown/);
  });

  it("the list of devices to add says it under the LED count", () => {
    const use = {
      scene: { id: "s", name: "S", rev: 1, groups: [] },
      scenes: [],
      unassigned: [],
      states: [],
      edit: () => {},
    } as unknown as UseScene;
    render(<AddDevices groupId="g" lights={[mirrored()]} use={use} onClose={() => {}} />);
    const pick = document.querySelector<HTMLElement>('[data-light="mirror#z1p1"]');
    expect(pick).not.toBeNull();
    expect(within(pick as HTMLElement).getByText(/8 LEDs · shown 8 times/)).toBeTruthy();
  });
});
