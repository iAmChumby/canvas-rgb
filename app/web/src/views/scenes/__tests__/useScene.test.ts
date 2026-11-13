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

/** The scene the daemon answers a live write with: the whole scene, one rev on. */
function liveAnswer(scene: Scene, groupId: string, props?: Props, brightness?: number): Scene {
  const next = clone(scene);
  const group = next.groups.find((g) => g.id === groupId);
  if (group) {
    if (props && group.plays.type === "effect") {
      group.plays = { ...group.plays, props: { ...group.plays.props, ...props } };
    }
    if (brightness !== undefined) group.brightness = brightness;
  }
  next.rev = scene.rev + 1;
  return next;
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
  // A live write answers the whole scene too, with its new rev.
  api.scenes.patchGroupProps.mockImplementation(async (_id: string, groupId: string, props: Props) =>
    liveAnswer(gaming(), groupId, props),
  );
  api.scenes.putGroupBrightness.mockImplementation(async (_id: string, groupId: string, brightness: number) =>
    liveAnswer(gaming(), groupId, undefined, brightness),
  );
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
    const [id, body] = api.scenes.put.mock.calls[0] as [string, { scene: Scene; expected_rev: number }];
    expect(id).toBe(result.current.scene?.id);
    expect(body.expected_rev).toBe(7);
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
    const before = result.current.scene;
    if (!before) throw new Error("the scene is not there");
    const beforeGroups = clone(before.groups);
    const beforeName = before.name;

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
    // The whole scene goes back, not one field of it.
    expect(body.scene.groups).toEqual(beforeGroups);
    expect(body.scene.name).toBe(beforeName);
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

  it("a drag sends at most one request per 60 ms, with the scene and group, and the last value goes", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const sceneId = result.current.scene?.id ?? "";
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    vi.useFakeTimers();
    // A drag: a value every 10 ms for 200 ms.
    for (let i = 1; i <= 20; i += 1) {
      act(() => {
        result.current.setProps(groupId, number(i));
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10);
      });
    }
    expect(api.scenes.patchGroupProps.mock.calls.length).toBeLessThanOrEqual(Math.ceil(200 / 60));

    // The trailing value goes out as well.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(120);
    });
    const last = api.scenes.patchGroupProps.mock.calls.at(-1) as [string, string, Props];
    expect(last[0]).toBe(sceneId);
    expect(last[1]).toBe(groupId);
    expect(last[2].amount).toEqual({ type: "number", value: 20 });
    // The live write answers the whole scene, so the drag reads nothing.
    expect(api.scenes.list).toHaveBeenCalledTimes(1);
  });

  it("setBrightness sends the group's brightness with the scene and group id", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const sceneId = result.current.scene?.id ?? "";
    const groupId = result.current.scene?.groups[1]?.id ?? "";

    vi.useFakeTimers();
    act(() => {
      result.current.setBrightness(groupId, 42);
    });
    expect(api.scenes.putGroupBrightness).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.putGroupBrightness).toHaveBeenCalledTimes(1);
    const call = api.scenes.putGroupBrightness.mock.calls[0] as [string, string, number];
    expect(call[0]).toBe(sceneId);
    expect(call[1]).toBe(groupId);
    expect(call[2]).toBe(42);
  });

  it("never has two live writes of one group on their way at once", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    let onTheWay = 0;
    api.scenes.patchGroupProps.mockImplementation(async () => {
      onTheWay += 1;
      expect(onTheWay).toBe(1);
      await new Promise((resolve) => setTimeout(resolve, 100));
      onTheWay -= 1;
      return liveAnswer(gaming(), groupId);
    });
    api.scenes.putGroupBrightness.mockImplementation(async () => {
      onTheWay += 1;
      expect(onTheWay).toBe(1);
      await new Promise((resolve) => setTimeout(resolve, 100));
      onTheWay -= 1;
      return liveAnswer(gaming(), groupId);
    });

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(1));
      result.current.setBrightness(groupId, 42);
    });
    // The first write is on its way (it takes 100 ms) and the values move on.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(70);
    });
    act(() => {
      result.current.setProps(groupId, number(2));
      result.current.setBrightness(groupId, 43);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });

    expect(onTheWay).toBe(0);
    // Both kinds went out, and the values that came last are the ones that went.
    const lastProps = api.scenes.patchGroupProps.mock.calls.at(-1) as [string, string, Props];
    expect(lastProps[2].amount).toEqual({ type: "number", value: 2 });
    const lastBrightness = api.scenes.putGroupBrightness.mock.calls.at(-1) as [string, string, number];
    expect(lastBrightness[2]).toBe(43);
  });

  it("the last live value goes out when the page goes away", async () => {
    const { result, unmount } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(7));
    });
    expect(api.scenes.patchGroupProps).not.toHaveBeenCalled();

    unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(120);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);
    const props = api.scenes.patchGroupProps.mock.calls[0]?.[2] as Props;
    expect(props.amount).toEqual({ type: "number", value: 7 });
  });

  it("an edit that lands while a write is in flight carries the answered rev", async () => {
    let release!: () => void;
    api.scenes.put.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = () => resolve({ ...gaming(), rev: 8 });
        }),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    act(() => {
      result.current.edit((draft) => {
        draft.name = "First";
      }, "Rename");
    });
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(1));

    // The second edit lands while the first write is still out.
    act(() => {
      result.current.edit((draft) => {
        draft.name = "Second";
      }, "Rename again");
    });
    release();
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(2));

    const [, second] = api.scenes.put.mock.calls[1] as [string, { scene: Scene; expected_rev: number }];
    // The second write carries the rev the first one answered.
    expect(second.expected_rev).toBe(8);
    expect(second.scene.rev).toBe(8);
    expect(second.scene.name).toBe("Second");
  });

  it("an edit made while a conflict is answered is sent, not dropped", async () => {
    const other = gaming();
    other.name = "Gaming (other window)";
    other.rev = 9;
    let release!: (value: ScenesResponse) => void;
    // The first read is this window's; the read after the refusal waits, so the second edit
    // lands while the daemon's scene is being read.
    api.scenes.list
      .mockResolvedValueOnce(answer())
      .mockImplementationOnce(
        () =>
          new Promise<ScenesResponse>((resolve) => {
            release = resolve;
          }),
      )
      .mockResolvedValue({
        active: other.id,
        scenes: DEMO_SCENES.scenes.map((s) => (s.id === other.id ? other : s)),
        group_states: other.groups.map((g) => ({ id: g.id, failed: null })),
      });
    api.scenes.put.mockRejectedValueOnce(new ApiFailure(409, "conflict", "the scene changed"));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    act(() => {
      result.current.edit((draft) => {
        draft.name = "First";
      }, "Rename");
    });
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(1));

    act(() => {
      result.current.edit((draft) => {
        draft.name = "Second";
      }, "Rename again");
    });
    release({
      active: other.id,
      scenes: DEMO_SCENES.scenes.map((s) => (s.id === other.id ? other : s)),
      group_states: other.groups.map((g) => ({ id: g.id, failed: null })),
    });

    // The second edit goes out, on the daemon's scene.
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(2));
    const [, second] = api.scenes.put.mock.calls[1] as [string, { scene: Scene; expected_rev: number }];
    expect(second.expected_rev).toBe(9);
    expect(second.scene.name).toBe("Second");
    // The other window's change is still there.
    expect(second.scene.groups).toEqual(other.groups);
  });

  it("a whole-scene save keeps the live values and the rev the live write answered", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    vi.useFakeTimers();
    act(() => {
      result.current.setBrightness(groupId, 42);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(120);
    });
    expect(api.scenes.putGroupBrightness).toHaveBeenCalledTimes(1);

    await act(async () => {
      result.current.edit((draft) => {
        draft.name = "Renamed";
      }, "Rename");
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });

    expect(api.scenes.put).toHaveBeenCalledTimes(1);
    const [, body] = api.scenes.put.mock.calls[0] as [string, { scene: Scene; expected_rev: number }];
    // The rev the live write answered, so the save cannot 409 against this window's own write.
    expect(body.expected_rev).toBe(8);
    // The live value goes with the save: it is not reverted.
    expect(body.scene.groups[0]?.brightness).toBe(42);
  });

  it("an edit that has not gone out goes out when the scene switches", async () => {
    // The first write hangs: the second edit waits behind it.
    let release!: () => void;
    api.scenes.put.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = () => resolve({ ...gaming(), rev: 8 });
        }),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const sceneId = result.current.scene?.id ?? "";

    act(() => {
      result.current.edit((draft) => {
        draft.name = "First";
      }, "Rename");
    });
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(1));

    act(() => {
      result.current.edit((draft) => {
        draft.name = "Second";
      }, "Rename again");
    });
    // Another window sets another scene active.
    patchSnap({
      scenes: {
        active: "focus",
        scenes: clone(DEMO_SCENES.scenes),
        group_states: [],
      },
    });
    release();

    // The edit that had not gone out goes out to the scene it belongs to.
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(2));
    const [id, body] = api.scenes.put.mock.calls[1] as [string, { scene: Scene; expected_rev: number }];
    expect(id).toBe(sceneId);
    expect(body.scene.name).toBe("Second");
  });

  it("a failed write leaves nothing on the undo stack", async () => {
    api.scenes.put.mockRejectedValueOnce(new ApiFailure(500, "http", "the daemon said no"));
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    await act(async () => {
      result.current.edit((draft) => {
        draft.name = "Mine";
      }, "Rename");
    });

    await waitFor(() => expect(result.current.error).toBe("the daemon said no"));
    // The change that failed is not on the undo stack: the first undo is not a no-op.
    expect(result.current.canUndo).toBe(false);
    await act(async () => {
      result.current.undo();
    });
    expect(api.scenes.put).toHaveBeenCalledTimes(1);
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

  it("hold every member to a light the demo has", () => {
    // Every group names lights the daemon knows: a group with a light that is not there is
    // refused by the daemon, and the demo would show a group that can never play.
    const ids = new Set(DEMO_LIGHTS.map((l) => l.light.id));
    for (const scene of DEMO_SCENES.scenes) {
      for (const group of scene.groups) {
        for (const member of group.members) {
          expect(ids.has(member), `${scene.name} / ${group.name} / ${member}`).toBe(true);
        }
      }
    }
  });
});
