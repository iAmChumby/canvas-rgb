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
const THIRD = DEMO_LIGHTS[2];
const FOURTH = DEMO_LIGHTS[3];

/**
 * The scene: one group with two devices, one with one, and one with one. A device that leaves a
 * group with two leaves the group alive; a device that leaves a group with one takes the group
 * with it.
 */
function scene(): Scene {
  return {
    id: "gaming",
    name: "Gaming",
    rev: 7,
    groups: [
      {
        id: "g-one",
        name: "Two devices",
        members: [FIRST?.light.id ?? "", THIRD?.light.id ?? ""],
        plays: { type: "off" },
        mode: "flow_across",
        brightness: 100,
      },
      {
        id: "g-two",
        name: "One device",
        members: [SECOND?.light.id ?? ""],
        plays: { type: "off" },
        mode: "flow_across",
        brightness: 100,
      },
      {
        id: "g-three",
        name: "Another one",
        members: [FOURTH?.light.id ?? ""],
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
        groupId="g-two"
        lights={clone(DEMO_LIGHTS)}
        use={use}
        onClose={() => {}}
      />,
    );
    const pick = row(FIRST?.light.id ?? "");
    expect(within(pick).getByText("In Two devices")).toBeTruthy();
    fireEvent.click(pick);
    const edits = use.calls.edit ?? [];
    expect(edits).toHaveLength(1);
    const [draft] = edits[0] as [Scene];
    const one = draft.groups.find((g) => g.id === "g-one");
    const two = draft.groups.find((g) => g.id === "g-two");
    expect(two?.members).toContain(FIRST?.light.id);
    expect(one?.members).not.toContain(FIRST?.light.id);
    // The group it came from still has a device: it stays.
    expect(one?.members).toContain(THIRD?.light.id);
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

  it("every light that is flashing stops on its own, and all of them on close", () => {
    // The pointer is on one light and the focus is on another: both are flashing, and each one
    // stops when the pointer leaves it or it loses the focus. The list closing stops the rest.
    const use = fake();
    const view = render(
      <AddDevices
        groupId="g-one"
        lights={clone(DEMO_LIGHTS)}
        use={use}
        onClose={() => {}}
      />,
    );
    const a = FIRST?.light.id ?? "";
    const b = SECOND?.light.id ?? "";
    fireEvent.pointerEnter(row(b));
    fireEvent.focus(row(a));
    expect(api.lights.flash).toHaveBeenCalledWith({ lights: [b], stop: false });
    expect(api.lights.flash).toHaveBeenCalledWith({ lights: [a], stop: false });
    fireEvent.pointerLeave(row(b));
    expect(api.lights.flash).toHaveBeenCalledWith({ lights: [b], stop: true });
    // The light the focus is on is still flashing.
    expect(api.lights.flash).not.toHaveBeenCalledWith({ lights: [a], stop: true });
    view.unmount();
    expect(api.lights.flash).toHaveBeenCalledWith({ lights: [a], stop: true });
  });

  it("moving the only light out of a group takes that group with it", () => {
    // A group with one device and nowhere to put it: the group goes with the device, or the
    // daemon refuses the whole save.
    const use = fake();
    const emptied = vi.fn();
    render(
      <AddDevices
        groupId="g-two"
        lights={clone(DEMO_LIGHTS)}
        use={use}
        onClose={() => {}}
        onEmptied={emptied}
      />,
    );
    fireEvent.click(row(FOURTH?.light.id ?? ""));
    const edits = use.calls.edit ?? [];
    expect(edits).toHaveLength(1);
    const [draft] = edits[0] as [Scene];
    const two = draft.groups.find((g) => g.id === "g-two");
    const three = draft.groups.find((g) => g.id === "g-three");
    expect(two?.members).toContain(FOURTH?.light.id);
    // The group the light came from had only that light: it is gone.
    expect(three).toBeUndefined();
    // The page says which group went, so the Undo can bring it back.
    expect(emptied).toHaveBeenCalledWith(["Another one"]);
  });
});
