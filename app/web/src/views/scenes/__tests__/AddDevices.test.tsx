import { cleanup, fireEvent, render, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The list of devices a group can take: every light with the group it is in, a flash on the
// hardware while the pointer is on it, and the move when one is chosen.

const { api } = vi.hoisted(() => ({
  api: { lights: { flash: vi.fn(), list: vi.fn(), patch: vi.fn() } },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { DEMO_LIGHTS } from "@/lib/fixtures";
import { AddDevices } from "@/views/scenes/AddDevices";
import type { Scene } from "@/proto/Scene";
import type { UseScene } from "@/views/scenes/useScene";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const clone = <T,>(value: T): T => structuredClone(value);

const FIRST = DEMO_LIGHTS[0];
const SECOND = DEMO_LIGHTS[1];

/** The scene: the first light is in one group, the second in another. */
function scene(): Scene {
  return {
    id: "gaming",
    name: "Gaming",
    rev: 7,
    groups: [
      {
        id: "g-one",
        name: "One device",
        members: [FIRST?.light.id ?? ""],
        plays: { type: "off" },
        mode: "flow_across",
        brightness: 100,
      },
      {
        id: "g-two",
        name: "Other",
        members: [SECOND?.light.id ?? ""],
        plays: { type: "off" },
        mode: "flow_across",
        brightness: 100,
      },
    ],
  };
}

function fake(): UseScene & { calls: Record<string, unknown[][]> } {
  const calls: Record<string, unknown[][]> = {};
  const spy =
    (name: string) =>
    (...args: unknown[]) => {
      (calls[name] ??= []).push(args);
    };
  const active = scene();
  const base: UseScene = {
    scene: active,
    scenes: [clone(active)],
    unassigned: [],
    states: active.groups.map((g) => ({ id: g.id, failed: null })),
    edit: (change: (draft: Scene) => void, label: string) => {
      const draft = clone(active);
      change(draft);
      (calls.edit ??= []).push([draft, label]);
    },
    setProps: spy("setProps"),
    setBrightness: spy("setBrightness"),
    setActive: spy("setActive"),
    create: spy("create"),
    remove: spy("remove"),
    renameScene: spy("renameScene"),
    undo: spy("undo"),
    redo: spy("redo"),
    canUndo: false,
    canRedo: false,
    error: null,
  };
  return Object.assign({ ...base }, { calls });
}

beforeEach(() => {
  api.lights.flash.mockReset();
  api.lights.flash.mockResolvedValue({ ok: true });
});

afterEach(() => {
  cleanup();
});

describe("the devices you can add", () => {
  const row = (id: string): HTMLElement => {
    const el = document.querySelector<HTMLElement>(`[data-light="${id}"]`);
    if (!el) throw new Error(`no row for ${id}`);
    return el;
  };

  it("adding a light that is in another group removes it there", () => {
    const use = fake();
    render(
      <AddDevices
        groupId="g-one"
        lights={clone(DEMO_LIGHTS)}
        use={use}
        onClose={() => {}}
      />,
    );
    const pick = row(SECOND?.light.id ?? "");
    expect(within(pick).getByText("In Other")).toBeTruthy();
    fireEvent.click(pick);
    const edits = use.calls.edit ?? [];
    expect(edits).toHaveLength(1);
    const [draft] = edits[0] as [Scene];
    const one = draft.groups.find((g) => g.id === "g-one");
    const two = draft.groups.find((g) => g.id === "g-two");
    expect(one?.members).toContain(SECOND?.light.id);
    expect(two?.members).not.toContain(SECOND?.light.id);
  });

  it("hovering a device flashes it and leaving stops", () => {
    const use = fake();
    render(
      <AddDevices
        groupId="g-one"
        lights={clone(DEMO_LIGHTS)}
        use={use}
        onClose={() => {}}
      />,
    );
    const id = FIRST?.light.id ?? "";
    const pick = row(id);
    fireEvent.pointerEnter(pick);
    expect(api.lights.flash).toHaveBeenCalledWith({ lights: [id], stop: false });
    fireEvent.pointerLeave(pick);
    expect(api.lights.flash).toHaveBeenCalledWith({ lights: [id], stop: true });
  });
});
