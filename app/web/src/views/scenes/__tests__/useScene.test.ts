import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The editor's scene data layer, against its own answers. The routes are the spine's
// (`crates/canvas-proto/src/api.rs`); the daemon that serves them does not exist yet, so the
// tests answer `api` themselves and check what the hook sends.

const { api } = vi.hoisted(() => ({
  api: {
    scenes: {
      list: vi.fn(),
      create: vi.fn(),
      put: vi.fn(),
      remove: vi.fn(),
      setActive: vi.fn(),
      patchGroupProps: vi.fn(),
      putGroupBrightness: vi.fn(),
    },
    lights: {
      list: vi.fn(),
      patch: vi.fn(),
      flash: vi.fn(),
    },
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { ApiFailure } from "@/lib/errors";
import { patchSnap } from "@/lib/store";
import { DEMO_LIGHTS, DEMO_SCENES, demoSceneProblem } from "@/lib/fixtures";
import type { Scene } from "@/proto/Scene";
import type { ScenesResponse } from "@/proto/ScenesResponse";
import type { Props } from "@/proto/props";
import { useScene } from "@/views/scenes/useScene";

const clone = <T,>(value: T): T => structuredClone(value);

function gaming(): Scene {
  const scene = DEMO_SCENES.scenes.find((s) => s.id === DEMO_SCENES.active);
  if (!scene) throw new Error("the demo has no active scene");
  return clone(scene);
}

/** The answer the daemon gives `GET /scenes`, built from the demo's own scenes. */
function answer(active?: string): ScenesResponse {
  const id = active ?? DEMO_SCENES.active;
  const scene = DEMO_SCENES.scenes.find((s) => s.id === id) ?? gaming();
  return {
    active: id,
    scenes: clone(DEMO_SCENES.scenes),
    group_states: scene.groups.map((g) => ({ id: g.id, failed: null })),
  };
}

function number(value: number): Props {
  return { amount: { type: "number", value } };
}

beforeEach(() => {
  for (const group of Object.values(api.scenes)) group.mockReset();
  for (const fn of Object.values(api.lights)) fn.mockReset();
  api.scenes.list.mockResolvedValue(answer());
  // Every write answers the scene the daemon saved: the rev it was sent plus one.
  api.scenes.put.mockImplementation(async (_id: string, body: { scene: Scene; expected_rev: number }) => ({
    ...body.scene,
    rev: body.expected_rev + 1,
  }));
  api.scenes.patchGroupProps.mockResolvedValue({ ok: true });
  patchSnap({ scenes: null, scenesError: null, lights: clone(DEMO_LIGHTS), lightsError: null });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useScene", () => {
  it("edit applies before the server answers", async () => {
    // A write that never lands: the page must show the change anyway.
    api.scenes.put.mockImplementation(() => new Promise(() => undefined));
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    act(() => {
      result.current.edit((draft) => {
        const group = draft.groups[0];
        if (group) group.name = "Renamed";
      }, "Rename group");
    });

    expect(result.current.scene?.groups[0]?.name).toBe("Renamed");
    expect(api.scenes.put).toHaveBeenCalledTimes(1);
  });

  it("a 409 replaces local state and sets error", async () => {
    const other = gaming();
    other.name = "Gaming (other window)";
    other.rev = 9;
    // The first read is this window's; the read after the refusal is the other window's.
    api.scenes.list.mockResolvedValueOnce(answer()).mockResolvedValue({
      active: other.id,
      scenes: DEMO_SCENES.scenes.map((s) => (s.id === other.id ? other : s)),
      group_states: other.groups.map((g) => ({ id: g.id, failed: null })),
    });
    api.scenes.put.mockRejectedValueOnce(new ApiFailure(409, "conflict", "the scene changed"));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    await act(async () => {
      result.current.edit((draft) => {
        draft.name = "Mine";
      }, "Rename scene");
    });

    await waitFor(() => expect(result.current.error).toBe("Changed in another window"));
    expect(result.current.scene?.name).toBe("Gaming (other window)");
    expect(result.current.scene?.rev).toBe(9);
    expect(result.current.canUndo).toBe(false);
  });

  it("undo sends the previous scene with the current rev", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const before = result.current.scene?.groups[0]?.name;

    await act(async () => {
      result.current.edit((draft) => {
        const group = draft.groups[0];
        if (group) group.name = "Renamed";
      }, "Rename group");
    });
    await waitFor(() => expect(result.current.scene?.rev).toBe(8));
    expect(result.current.canUndo).toBe(true);

    await act(async () => {
      result.current.undo();
    });

    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(2));
    const [, body] = api.scenes.put.mock.calls.at(-1) as [
      string,
      { scene: Scene; expected_rev: number },
    ];
    expect(body.expected_rev).toBe(8);
    expect(body.scene.rev).toBe(8);
    expect(body.scene.groups[0]?.name).toBe(before);
    await waitFor(() => expect(result.current.canRedo).toBe(true));
  });

  it("setProps sends at most one request per 60 ms and always the last value", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(1));
      result.current.setProps(groupId, number(2));
      result.current.setProps(groupId, number(3));
    });
    expect(api.scenes.patchGroupProps).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);
    const props = api.scenes.patchGroupProps.mock.calls[0]?.[2] as Props;
    expect(props.amount).toEqual({ type: "number", value: 3 });

    act(() => {
      result.current.setProps(groupId, number(4));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(59);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(2);
    const last = api.scenes.patchGroupProps.mock.calls[1]?.[2] as Props;
    expect(last.amount).toEqual({ type: "number", value: 4 });
  });
});

describe("the demo's scenes", () => {
  it("pass the two membership rules", () => {
    // A TS port of canvas-proto's `validate_scene`: a light is in one group at most, and a
    // group has at least one member.
    for (const scene of DEMO_SCENES.scenes) {
      expect(demoSceneProblem(scene), scene.name).toBeNull();
    }
    // The port itself catches both, so a bad demo scene cannot pass by accident.
    expect(
      demoSceneProblem({
        id: "bad",
        name: "Bad",
        rev: 1,
        groups: [
          { id: "a", name: "A", members: ["x"], plays: { type: "off" }, mode: "flow_across", brightness: 100 },
          { id: "b", name: "B", members: ["x"], plays: { type: "off" }, mode: "flow_across", brightness: 100 },
        ],
      }),
    ).toContain("two groups");
    expect(
      demoSceneProblem({
        id: "bad",
        name: "Bad",
        rev: 1,
        groups: [{ id: "a", name: "A", members: [], plays: { type: "off" }, mode: "flow_across", brightness: 100 }],
      }),
    ).toContain("no members");
  });
});
