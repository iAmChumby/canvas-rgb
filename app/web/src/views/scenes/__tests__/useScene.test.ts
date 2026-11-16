import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The editor's scene data layer, against its own answers. The routes are the spine's
// (`crates/canvas-proto/src/api.rs`); the daemon that serves them does not exist yet, so the
// tests answer `api` themselves and check what the hook sends.

const { api, notifySpy } = vi.hoisted(() => ({
  api: {
    scenes: {
      list: vi.fn(),
      create: vi.fn(),
      put: vi.fn(),
      remove: vi.fn(),
      setActive: vi.fn(),
      patchGroupProps: vi.fn(),
      putGroupBrightness: vi.fn(),
      moveGroup: vi.fn(),
      write: vi.fn(),
    },
    lights: {
      list: vi.fn(),
      patch: vi.fn(),
      flash: vi.fn(),
    },
  },
  notifySpy: vi.fn(),
}));

vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("@/components/Notify", () => ({ notify: notifySpy }));

import { ApiFailure } from "@/lib/errors";
import { getSnap, patchSnap, refreshScenes } from "@/lib/store";
import { DEMO_LIGHTS, DEMO_SCENES, demoSceneProblem } from "@/lib/fixtures";
import { htmlLimitSentence } from "@/lib/htmlLimit";
import { NOTHING_MOVED, NOTHING_UNDONE, useScene } from "@/views/scenes/useScene";
import type { Group } from "@/proto/Group";
import type { GroupMove } from "@/proto/GroupMove";
import type { GroupMoved } from "@/proto/GroupMoved";
import type { Scene } from "@/proto/Scene";
import type { ScenesResponse } from "@/proto/ScenesResponse";
import type { ScenesWrite } from "@/proto/ScenesWrite";
import type { Props } from "@/proto/props";

const clone = <T,>(value: T): T => structuredClone(value);

/** One group of a made-up scene. */
function group(id: string, members: string[]): Group {
  return {
    id,
    name: id.toUpperCase(),
    members,
    plays: { type: "effect", effect: "solid", props: {} },
    mode: "flow_across",
    brightness: 100,
  };
}

/** The answer `GET /scenes` gives for a list of scenes. */
function listOf(scenes: Scene[]): ScenesResponse {
  return {
    active: scenes[0]?.id ?? "",
    scenes: clone(scenes),
    group_states: scenes[0]?.groups.map((g) => ({ id: g.id, failed: null, fits: [] })) ?? [],
  };
}

/**
 * The two scenes a move runs between: `main` holds LEFT and RIGHT, `other` holds ALL with both
 * lights. The answer is the daemon's own: the scenes after, the scenes before, what was taken.
 */
function moveRig() {
  const beforeMain: Scene = {
    id: "main",
    name: "Main",
    rev: 3,
    groups: [group("left", ["a"]), group("right", ["b"])],
  };
  const beforeOther: Scene = {
    id: "other",
    name: "Other",
    rev: 4,
    groups: [group("all", ["a", "b"])],
  };
  const afterMain: Scene = {
    ...clone(beforeMain),
    rev: 4,
    groups: [clone(beforeMain.groups[1]!)],
  };
  const afterOther: Scene = {
    ...clone(beforeOther),
    rev: 5,
    groups: [
      { ...clone(beforeOther.groups[0]!), members: ["b"] },
      clone(beforeMain.groups[0]!),
    ],
  };
  const answer: GroupMoved = {
    group_id: "left",
    saved: true,
    scenes: [clone(afterMain), clone(afterOther)],
    before: [clone(beforeMain), clone(beforeOther)],
    taken: [{ group_id: "all", group_name: "ALL", lights: ["a"] }],
    emptied: [],
  };
  return { beforeMain, beforeOther, afterMain, afterOther, answer };
}

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
    group_states: scene.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
  };
}

function number(value: number): Props {
  return { amount: { type: "number", value } };
}

/** The active scene with one group's effect props replaced, at `rev`. */
function withProps(groupId: string, props: Props, rev: number): Scene {
  const scene = gaming();
  scene.rev = rev;
  const group = scene.groups.find((g) => g.id === groupId);
  if (group && group.plays.type === "effect") group.plays = { ...group.plays, props: { ...props } };
  return scene;
}

/** The answer `GET /scenes` gives for one scene. */
function oneScene(scene: Scene): ScenesResponse {
  return {
    active: scene.id,
    scenes: [scene],
    group_states: scene.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
  };
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

/** The rev the fake daemon is at. Every write it answers moves it one on, as the daemon does. */
let serverRev = 0;

/**
 * A fake daemon that holds the scenes, so the scene routes answer with the same list: a scene
 * that is made is in the list, a scene that is deleted is out of it.
 */
function fakeDaemon(): { active: string; activeName: string } {
  const scenes = clone(DEMO_SCENES.scenes);
  let active = DEMO_SCENES.active;
  const list = () => ({
    active,
    scenes: clone(scenes),
    group_states: scenes.find((s) => s.id === active)?.groups.map((g) => ({ id: g.id, failed: null, fits: [] })) ?? [],
  });
  api.scenes.list.mockImplementation(async () => list());
  api.scenes.setActive.mockImplementation(async (id: string) => {
    active = id;
    return list();
  });
  api.scenes.remove.mockImplementation(async (id: string) => {
    const at = scenes.findIndex((s) => s.id === id);
    if (at < 0) throw new ApiFailure(404, "not_found", `no scene with the id ${id}`);
    scenes.splice(at, 1);
    if (active === id) active = scenes[0]?.id ?? "";
    return { ok: true };
  });
  api.scenes.create.mockImplementation(async (body: { name: string; copy_of?: string }) => {
    const source = body.copy_of ? scenes.find((s) => s.id === body.copy_of) : null;
    const made: Scene = {
      id: `scene-${scenes.length + 1}`,
      name: body.name,
      rev: 1,
      groups: source ? clone(source.groups) : [],
    };
    scenes.push(made);
    return clone(made);
  });
  api.scenes.put.mockImplementation(async (id: string, put: { scene: Scene; expected_rev: number }) => {
    const saved: Scene = { ...clone(put.scene), rev: put.expected_rev + 1 };
    const at = scenes.findIndex((s) => s.id === id);
    if (at < 0) scenes.push(saved);
    else scenes[at] = saved;
    return clone(saved);
  });
  return {
    get active() {
      return active;
    },
    get activeName() {
      return scenes.find((s) => s.id === active)?.name ?? "";
    },
  };
}

/** A live write answered the way the daemon answers: the whole scene, one rev on. */
function answerLive(groupId: string, props?: Props, brightness?: number): Scene {
  serverRev += 1;
  const scene = gaming();
  scene.rev = serverRev - 1;
  return liveAnswer(scene, groupId, props, brightness);
}

beforeEach(() => {
  for (const group of Object.values(api.scenes)) group.mockReset();
  for (const fn of Object.values(api.lights)) fn.mockReset();
  notifySpy.mockReset();
  serverRev = gaming().rev;
  api.scenes.list.mockResolvedValue(answer());
  // Every write answers the scene the daemon saved: the rev it was sent plus one.
  api.scenes.put.mockImplementation(async (_id: string, body: { scene: Scene; expected_rev: number }) => {
    serverRev = body.expected_rev + 1;
    return { ...body.scene, rev: serverRev };
  });
  // A live write answers the whole scene too, with its new rev.
  api.scenes.patchGroupProps.mockImplementation(async (_id: string, groupId: string, props: Props) =>
    answerLive(groupId, props),
  );
  api.scenes.putGroupBrightness.mockImplementation(async (_id: string, groupId: string, brightness: number) =>
    answerLive(groupId, undefined, brightness),
  );
  api.lights.list.mockResolvedValue({ lights: clone(DEMO_LIGHTS) });
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
      group_states: other.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
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

  it("an undo puts a deleted scene back with its groups, and plays it again", async () => {
    const daemon = fakeDaemon();
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const before = result.current.scenes.length;

    act(() => {
      result.current.remove(daemon.active);
    });
    await waitFor(() => expect(result.current.scenes.length).toBe(before - 1));
    expect(result.current.canUndo).toBe(true);

    act(() => {
      result.current.undo();
    });
    await waitFor(() => expect(result.current.scenes.length).toBe(before));
    const back = result.current.scenes.find((s) => s.name === daemon.activeName);
    expect(back?.groups.length).toBeGreaterThan(0);
    // The scene that played before plays again.
    expect(result.current.scene?.id).toBe(back?.id);
  });

  it("an undo puts a renamed scene's old name back", async () => {
    const daemon = fakeDaemon();
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const id = result.current.scene?.id ?? "";

    act(() => {
      result.current.renameScene(id, "Evening wind-down");
    });
    await waitFor(() => expect(result.current.scene?.name).toBe("Evening wind-down"));
    expect(result.current.canUndo).toBe(true);

    act(() => {
      result.current.undo();
    });
    await waitFor(() => expect(result.current.scene?.name).toBe(daemon.activeName));
  });

  it("an undo takes a made scene away, and a redo makes it again", async () => {
    fakeDaemon();
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const before = result.current.scenes.length;

    act(() => {
      result.current.create("Stage");
    });
    await waitFor(() => expect(result.current.scenes.length).toBe(before + 1));
    expect(result.current.canUndo).toBe(true);

    act(() => {
      result.current.undo();
    });
    await waitFor(() => expect(result.current.scenes.length).toBe(before));

    act(() => {
      result.current.redo();
    });
    await waitFor(() => expect(result.current.scenes.length).toBe(before + 1));
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

    // Overlaps are counted here and asserted after the run. An expect inside the mock would throw
    // into the page's own catch, which would turn the overlap into a failed write and hide it.
    let onTheWay = 0;
    let overlaps = 0;
    const onWay = async (answer: () => Scene): Promise<Scene> => {
      onTheWay += 1;
      if (onTheWay > 1) overlaps += 1;
      await new Promise((resolve) => setTimeout(resolve, 100));
      onTheWay -= 1;
      return answer();
    };
    api.scenes.patchGroupProps.mockImplementation((_id: string, group: string, props: Props) =>
      onWay(() => answerLive(group, props)),
    );
    api.scenes.putGroupBrightness.mockImplementation((_id: string, group: string, brightness: number) =>
      onWay(() => answerLive(group, undefined, brightness)),
    );

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

    expect(overlaps).toBe(0);
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

  it("a live write that is answered keeps the edit the draft carries", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    // The live write stays out until the test lets it land.
    let release!: () => void;
    api.scenes.putGroupBrightness.mockImplementationOnce(
      () =>
        new Promise<Scene>((resolve) => {
          release = () => resolve(answerLive(groupId, undefined, 42));
        }),
    );

    vi.useFakeTimers();
    act(() => {
      result.current.setBrightness(groupId, 42);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.putGroupBrightness).toHaveBeenCalledTimes(1);

    // The user renames the group while the live write is out.
    act(() => {
      result.current.edit((draft) => {
        const group = draft.groups[0];
        if (group) group.name = "Renamed";
      }, "Rename group");
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    // The save waits for the live write: it goes out after it.
    expect(api.scenes.put).not.toHaveBeenCalled();

    // The live write answers with the scene and its new rev.
    await act(async () => {
      release();
      await vi.advanceTimersByTimeAsync(20);
    });

    expect(api.scenes.put).toHaveBeenCalledTimes(1);
    const [, body] = api.scenes.put.mock.calls[0] as [string, { scene: Scene; expected_rev: number }];
    // The rename the user made while the live write was out is still there.
    expect(body.scene.groups[0]?.name).toBe("Renamed");
    // The live value goes with it, and the rev is the one the live write answered.
    expect(body.scene.groups[0]?.brightness).toBe(42);
    expect(body.expected_rev).toBe(8);
    expect(body.scene.rev).toBe(8);
  });

  it("a live write that fails does not take the value that came while it was out with it", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    // The first write stays out until the test lets it be refused.
    let refuse!: (e: unknown) => void;
    api.scenes.patchGroupProps.mockImplementationOnce(
      () =>
        new Promise<Scene>((_resolve, reject) => {
          refuse = reject;
        }),
    );

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(1));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);

    // The value that comes while the first is out waits for it.
    act(() => {
      result.current.setProps(groupId, number(2));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);

    // The first write is refused.
    await act(async () => {
      refuse(new ApiFailure(500, "http", "the daemon said no"));
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(result.current.error).toBe("the daemon said no");
    // The value that came while it was out still goes.
    const last = api.scenes.patchGroupProps.mock.calls.at(-1) as [string, string, Props];
    expect(last[2].amount).toEqual({ type: "number", value: 2 });
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
        group_states: other.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
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
      group_states: other.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
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

  it("a save that waits for a live write carries the rev that write answered", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    // The live write answers only when the test says so.
    let release!: (scene: Scene) => void;
    api.scenes.putGroupBrightness.mockImplementationOnce(
      () =>
        new Promise<Scene>((resolve) => {
          release = resolve;
        }),
    );

    vi.useFakeTimers();
    act(() => {
      result.current.setBrightness(groupId, 42);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.putGroupBrightness).toHaveBeenCalledTimes(1);

    // The user edits while the live write is still out.
    act(() => {
      result.current.edit((draft) => {
        draft.name = "Renamed";
      }, "Rename");
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    // The save waits for the live write: it goes out after it, not with a rev the daemon has
    // already left behind.
    expect(api.scenes.put).not.toHaveBeenCalled();

    // The live write answers with the scene and its new rev.
    await act(async () => {
      release(answerLive(groupId, undefined, 42));
      await vi.advanceTimersByTimeAsync(10);
    });

    const [, body] = api.scenes.put.mock.calls[0] as [string, { scene: Scene; expected_rev: number }];
    // The rev the live write answered with, so the save cannot 409 against this window's own
    // live write.
    expect(body.expected_rev).toBe(8);
    expect(body.scene.rev).toBe(8);
    // The live value goes with the save: it is not reverted.
    expect(body.scene.groups[0]?.brightness).toBe(42);
    // The save goes out once: a save that goes out twice would fight its own answer.
    expect(api.scenes.put).toHaveBeenCalledTimes(1);
  });
  it("a live answer that skips a rev is another window's save: the unsent draft is not sent", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    // The live write stays out until the other window has saved.
    let release!: () => void;
    api.scenes.putGroupBrightness.mockImplementationOnce(
      () =>
        new Promise<Scene>((resolve) => {
          release = () => {
            // The other window renamed the scene at rev 8; this live write is saved after it, at rev 9.
            const other = gaming();
            other.name = "Gaming (other window)";
            other.rev = 8;
            resolve(liveAnswer(other, groupId, undefined, 42));
          };
        }),
    );

    vi.useFakeTimers();
    act(() => {
      result.current.setBrightness(groupId, 42);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.putGroupBrightness).toHaveBeenCalledTimes(1);

    // This window renames the scene while the live write is out: the save waits behind it.
    act(() => {
      result.current.edit((draft) => {
        draft.name = "Mine";
      }, "Rename scene");
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    expect(api.scenes.put).not.toHaveBeenCalled();

    await act(async () => {
      release();
      await vi.advanceTimersByTimeAsync(50);
    });

    // This window's last rev was 7, so its live write should answer 8. It answered 9: another
    // window saved in between. The unsent "Mine" is not sent on top of the other window's rename.
    expect(api.scenes.put).not.toHaveBeenCalled();
    expect(result.current.error).toBe("Changed in another window");
    expect(result.current.scene?.name).toBe("Gaming (other window)");
  });

  it("a live write whose answer was lost is not taken for another window's save", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    // A daemon of this test's own: every write it takes is saved and moves its rev one on, and
    // every read answers the scene it holds.
    const held = gaming();
    let applied = 0;
    const apply = (props: Props): Scene => {
      const group = held.groups.find((g) => g.id === groupId);
      if (group && group.plays.type === "effect") {
        group.plays = { ...group.plays, props: { ...group.plays.props, ...props } };
      }
      held.rev += 1;
      applied += 1;
      return clone(held);
    };
    api.scenes.list.mockImplementation(async () => ({
      active: held.id,
      scenes: [clone(held)],
      group_states: held.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
    }));
    // The first write reaches the daemon and is saved; its answer is lost on the way back.
    let lost = true;
    api.scenes.patchGroupProps.mockImplementation(async (_id: string, _group: string, props: Props) => {
      const answer = apply(props);
      if (lost) {
        lost = false;
        throw new TypeError("Failed to fetch");
      }
      return answer;
    });

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(42));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });

    // The write that landed is not sent again, and no other window is named.
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);
    expect(applied).toBe(1);
    expect(result.current.error).toBeNull();
    expect(result.current.scene?.rev).toBe(8);
    const shownPlays = result.current.scene?.groups[0]?.plays;
    expect(shownPlays?.type === "effect" ? shownPlays.props.amount : undefined).toEqual({
      type: "number",
      value: 42,
    });

    // The next save continues from the rev the read found: it neither 409s against this window's
    // own write nor drops the edit.
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
    expect(body.expected_rev).toBe(8);
    expect(body.scene.name).toBe("Renamed");
  });

  it("a write that did not land is not taken for another window's save of the same key", async () => {
    // This window's write never reaches the daemon. Another window saves the same group's same key
    // with another value, so the rev moves one on: one rev on is this write's own only when the
    // value that was sent is there.
    const groupId = gaming().groups[0]?.id ?? "";
    const mine = withProps(groupId, { amount: { type: "number", value: 1 } }, gaming().rev);
    const other = withProps(groupId, { amount: { type: "number", value: 7 } }, gaming().rev + 1);
    api.scenes.list.mockResolvedValueOnce(oneScene(mine)).mockResolvedValue(oneScene(other));
    api.scenes.patchGroupProps.mockRejectedValue(
      new ApiFailure(0, "unreachable", "canvas-rgb did not answer."),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(42));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });

    // The read found the other window's 7, not this window's 42: a conflict, said out loud, and
    // this window's value is not taken for saved.
    expect(result.current.error).toBe("Changed in another window");
    const plays = result.current.scene?.groups[0]?.plays;
    expect(plays?.type === "effect" ? plays.props.amount : undefined).toEqual({ type: "number", value: 7 });
  });

  it("a write that did not land is not taken for another window's save of another key", async () => {
    // The other window changed a different key. The sent key is still there with the value it had
    // before this write, which is not the value that was sent: one rev on is not this write's own.
    const groupId = gaming().groups[0]?.id ?? "";
    const mine = withProps(
      groupId,
      { amount: { type: "number", value: 1 }, speed: { type: "number", value: 2 } },
      gaming().rev,
    );
    const other = withProps(
      groupId,
      { amount: { type: "number", value: 1 }, speed: { type: "number", value: 3 } },
      gaming().rev + 1,
    );
    api.scenes.list.mockResolvedValueOnce(oneScene(mine)).mockResolvedValue(oneScene(other));
    api.scenes.patchGroupProps.mockRejectedValue(
      new ApiFailure(0, "unreachable", "canvas-rgb did not answer."),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(42));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });

    expect(result.current.error).toBe("Changed in another window");
    const plays = result.current.scene?.groups[0]?.plays;
    expect(plays?.type === "effect" ? plays.props.amount : undefined).toEqual({ type: "number", value: 1 });
  });

  it("a write the daemon refuses is not read back", async () => {
    api.scenes.patchGroupProps.mockRejectedValue(
      new ApiFailure(422, "validation", "the value is not in range"),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(1));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });

    // A 4xx is the daemon's own refusal: nothing was saved, so the scene is not read again and the
    // value is tried three times like any refusal.
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(3);
    expect(api.scenes.list).toHaveBeenCalledTimes(1);
  });

  it("a read-back that never answers is given up on, and the value goes again", async () => {
    // The write's outcome is unknown and the read never answers: the write's slot must not be held
    // by it for good.
    api.scenes.list.mockResolvedValueOnce(answer()).mockImplementation(() => new Promise(() => undefined));
    let first = true;
    api.scenes.patchGroupProps.mockImplementation(async (_id: string, group: string, props: Props) => {
      if (first) {
        first = false;
        throw new ApiFailure(0, "unreachable", "canvas-rgb did not answer.");
      }
      return answerLive(group, props);
    });

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(1));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);

    // The read gives up after its deadline (5 s), and the value goes again on its throttle.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(6000);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(2);
  });

  it("a colour write that did not land is not taken for another window's colour", async () => {
    // A colour is a `#rrggbb` string on the wire, so two colours must not compare equal.
    const groupId = gaming().groups[0]?.id ?? "";
    const mine = withProps(groupId, { tint: { type: "color", value: "#ff0000" } }, gaming().rev);
    const other = withProps(groupId, { tint: { type: "color", value: "#00ff00" } }, gaming().rev + 1);
    api.scenes.list.mockResolvedValueOnce(oneScene(mine)).mockResolvedValue(oneScene(other));
    api.scenes.patchGroupProps.mockRejectedValue(
      new ApiFailure(0, "unreachable", "canvas-rgb did not answer."),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, { tint: { type: "color", value: "#ff0000" } });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });

    // The read found the other window's green, not this window's red: a conflict, said out loud,
    // and the red is not taken for saved.
    expect(result.current.error).toBe("Changed in another window");
    const plays = result.current.scene?.groups[0]?.plays;
    expect(plays?.type === "effect" ? plays.props.tint : undefined).toEqual({
      type: "color",
      value: "#00ff00",
    });
  });

  it("each kind gets its own three tries: one kind's failures do not refuse the other", async () => {
    api.scenes.patchGroupProps.mockRejectedValue(new ApiFailure(500, "http", "the daemon said no"));
    api.scenes.putGroupBrightness.mockRejectedValue(new ApiFailure(500, "http", "the daemon said no"));
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(1));
      result.current.setBrightness(groupId, 42);
    });
    // Three tries of each kind, a throttle window apart. A count shared by both kinds would take
    // the brightness value off the screen and out of the queue on the props value's last try,
    // though the brightness value had a try of its own left.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(3);
    expect(api.scenes.putGroupBrightness).toHaveBeenCalledTimes(3);
    expect(result.current.error).toBe("the daemon said no");
  });

  it("a value of the other kind that was never tried survives a kind's third failure", async () => {
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    // Every props write stays out until the test refuses it, so a brightness value can be queued
    // and never tried while the props value is being refused.
    const held: Array<(e: unknown) => void> = [];
    api.scenes.patchGroupProps.mockImplementation(
      () =>
        new Promise<Scene>((_resolve, reject) => {
          held.push(reject);
        }),
    );

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(1));
    });
    for (let attempt = 0; attempt < 3; attempt += 1) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(60);
      });
      expect(held.length).toBe(attempt + 1);
      if (attempt === 2) {
        // The user moves the brightness while the third props try is out: it waits, never tried.
        act(() => {
          result.current.setBrightness(groupId, 42);
        });
      }
      await act(async () => {
        held[attempt]?.(new ApiFailure(500, "http", "the daemon said no"));
        await vi.advanceTimersByTimeAsync(0);
      });
    }
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(3);

    // The props value that failed is gone; the brightness value, never tried, still goes.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.putGroupBrightness).toHaveBeenCalledTimes(1);
    const call = api.scenes.putGroupBrightness.mock.calls[0] as [string, string, number];
    expect(call[2]).toBe(42);
    expect(result.current.scene?.groups[0]?.brightness).toBe(42);
  });

  it("a write left at unmount that is refused retries on the throttle, not back to back", async () => {
    const { result, unmount } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    // The first write stays out until the test refuses it.
    let refuse!: (e: unknown) => void;
    api.scenes.patchGroupProps.mockImplementationOnce(
      () =>
        new Promise<Scene>((_resolve, reject) => {
          refuse = reject;
        }),
    );

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(1));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);

    // The page goes away, and the write on its way is refused.
    unmount();
    await act(async () => {
      refuse(new ApiFailure(500, "http", "the daemon said no"));
      await vi.advanceTimersByTimeAsync(1);
    });
    // The retry waits its 60 ms like any other write: leaving the page does not make it go back
    // to back.
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(2);
  });

  it("a live value of one scene is never saved into another scene that has the same group id", async () => {
    const first = gaming();
    const second = clone(first);
    second.id = "scene-two";
    second.name = "Second";
    const groupId = first.groups[0]?.id ?? "";
    const original = first.groups[0]?.brightness ?? -1;
    api.scenes.list.mockResolvedValue({
      active: first.id,
      scenes: [first, second],
      group_states: first.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
    });

    // This window's first save is out until the test lets it land.
    let release!: () => void;
    api.scenes.put.mockImplementationOnce(
      () =>
        new Promise<Scene>((resolve) => {
          release = () => resolve({ ...clone(first), name: "First, saved", rev: first.rev + 1 });
        }),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    act(() => {
      result.current.edit((draft) => {
        draft.name = "First, edited";
      }, "Rename");
    });
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(1));
    act(() => {
      result.current.edit((draft) => {
        draft.name = "First, edited again";
      }, "Rename again");
    });

    // The other scene becomes active, and this window sets the brightness of the group that has
    // the same id in that scene.
    act(() => {
      patchSnap({ scenes: { active: second.id, scenes: [first, second], group_states: [] } });
    });
    vi.useFakeTimers();
    act(() => {
      result.current.setBrightness(groupId, 17);
    });

    // The first save is answered. The edit that waited behind it goes out on the first scene.
    await act(async () => {
      release();
      await vi.advanceTimersByTimeAsync(100);
    });

    expect(api.scenes.put).toHaveBeenCalledTimes(2);
    const [id, body] = api.scenes.put.mock.calls[1] as [string, { scene: Scene; expected_rev: number }];
    expect(id).toBe(first.id);
    expect(body.scene.groups[0]?.brightness).toBe(original);
  });

  it("a live value of one scene is not sent in a save of another scene that has the same group id", async () => {
    const first = gaming();
    const second = clone(first);
    second.id = "scene-two";
    second.name = "Second";
    const groupId = first.groups[0]?.id ?? "";
    api.scenes.list.mockResolvedValue({
      active: first.id,
      scenes: [first, second],
      group_states: first.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
    });

    // The brightness write of the first scene stays out until the test lets it land.
    let release!: () => void;
    api.scenes.putGroupBrightness.mockImplementationOnce(
      () =>
        new Promise<Scene>((resolve) => {
          release = () => resolve(liveAnswer(first, groupId, undefined, 17));
        }),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());

    vi.useFakeTimers();
    act(() => {
      result.current.setBrightness(groupId, 17);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60);
    });
    expect(api.scenes.putGroupBrightness).toHaveBeenCalledTimes(1);

    // The other scene becomes active before the write lands, then the write lands.
    act(() => {
      patchSnap({ scenes: { active: second.id, scenes: [first, second], group_states: [] } });
    });
    await act(async () => {
      release();
      await vi.advanceTimersByTimeAsync(20);
    });

    act(() => {
      result.current.edit((draft) => {
        draft.name = "Second, edited";
      }, "Rename");
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });

    expect(api.scenes.put).toHaveBeenCalledTimes(1);
    const [id, body] = api.scenes.put.mock.calls[0] as [string, { scene: Scene; expected_rev: number }];
    expect(id).toBe(second.id);
    expect(body.scene.groups[0]?.brightness).toBe(second.groups[0]?.brightness);
  });

  it("a live write left at unmount waits for the save on its way, and its answer is kept", async () => {
    const { result, unmount } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const sceneId = result.current.scene?.id ?? "";
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    // A live write and a save of the scene are never on their way together. Counted here and
    // asserted after the run: an expect inside a mock is swallowed by the page's own catch.
    let onTheWay = 0;
    let overlaps = 0;
    const track = <T,>(call: () => Promise<T>): Promise<T> => {
      onTheWay += 1;
      if (onTheWay > 1) overlaps += 1;
      return call().finally(() => {
        onTheWay -= 1;
      });
    };

    // This window's first save is out until the test lets it land.
    let holdFirst = true;
    let releaseFirst!: () => void;
    api.scenes.put.mockImplementation((_id: string, body: { scene: Scene; expected_rev: number }) =>
      track(async () => {
        if (holdFirst) {
          holdFirst = false;
          await new Promise<void>((resolve) => {
            releaseFirst = resolve;
          });
        }
        serverRev = body.expected_rev + 1;
        return { ...clone(body.scene), rev: serverRev };
      }),
    );
    api.scenes.patchGroupProps.mockImplementation((_id: string, group: string, props: Props) =>
      track(async () => answerLive(group, props)),
    );

    vi.useFakeTimers();
    act(() => {
      result.current.edit((draft) => {
        draft.name = "First";
      }, "Rename");
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(api.scenes.put).toHaveBeenCalledTimes(1);

    // A slider value waits behind the save, and a second edit lands while the save is out.
    act(() => {
      result.current.setProps(groupId, number(9));
      result.current.edit((draft) => {
        draft.name = "Second";
      }, "Rename again");
    });

    // The page goes away with both still waiting; then the first save lands.
    unmount();
    await act(async () => {
      releaseFirst();
      await vi.advanceTimersByTimeAsync(200);
    });

    expect(overlaps).toBe(0);
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(1);
    const props = api.scenes.patchGroupProps.mock.calls[0]?.[2] as Props;
    expect(props.amount).toEqual({ type: "number", value: 9 });
    // The second save carries the rev the first one answered; the slider's answer is kept.
    expect(api.scenes.put).toHaveBeenCalledTimes(2);
    const [, second] = api.scenes.put.mock.calls[1] as [string, { scene: Scene; expected_rev: number }];
    expect(second.expected_rev).toBe(8);
    expect(getSnap().scenes?.scenes.find((s) => s.id === sceneId)?.rev).toBe(10);
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

  it("a live write refused three times is dropped: it leaves the screen, and no later save carries it", async () => {
    api.scenes.patchGroupProps.mockRejectedValue(new ApiFailure(500, "http", "the daemon said no"));
    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene).not.toBeNull());
    const groupId = result.current.scene?.groups[0]?.id ?? "";

    vi.useFakeTimers();
    act(() => {
      result.current.setProps(groupId, number(5));
    });
    // Three tries, a throttle window apart, and then the group stops.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(3);
    expect(result.current.error).toBe("the daemon said no");
    // The screen shows what the daemon saved, not the refused value.
    const shownPlays = result.current.scene?.groups[0]?.plays;
    expect(shownPlays?.type === "effect" ? shownPlays.props : {}).toEqual({});

    // A later save does not carry the refused value, and the group does not try again on its own.
    act(() => {
      result.current.edit((draft) => {
        draft.name = "Renamed";
      }, "Rename");
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    expect(api.scenes.put).toHaveBeenCalledTimes(1);
    const [, body] = api.scenes.put.mock.calls[0] as [string, { scene: Scene; expected_rev: number }];
    const savedPlays = body.scene.groups[0]?.plays;
    expect(savedPlays?.type === "effect" ? savedPlays.props : {}).toEqual({});
    expect(api.scenes.patchGroupProps).toHaveBeenCalledTimes(3);
  });

  it("after a conflict, undo takes back the kept edit and never the other window's change", async () => {
    const other = gaming();
    other.name = "Gaming (other window)";
    other.rev = 9;
    const otherAnswer: ScenesResponse = {
      active: other.id,
      scenes: DEMO_SCENES.scenes.map((s) => (s.id === other.id ? other : s)),
      group_states: other.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
    };
    // The first read is this window's; the read after the refusal waits, so the second edit lands
    // while the daemon's scene is being read.
    let release!: (value: ScenesResponse) => void;
    api.scenes.list
      .mockResolvedValueOnce(answer())
      .mockImplementationOnce(
        () =>
          new Promise<ScenesResponse>((resolve) => {
            release = resolve;
          }),
      )
      .mockResolvedValue(otherAnswer);
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
    release(otherAnswer);
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(2));

    // Undo takes back the kept edit: the other window's scene comes back, with the other window's name.
    act(() => {
      result.current.undo();
    });
    expect(api.scenes.put).toHaveBeenCalledTimes(3);
    const [, undone] = api.scenes.put.mock.calls[2] as [string, { scene: Scene; expected_rev: number }];
    expect(undone.scene.name).toBe("Gaming (other window)");

    // Nothing of this window is left to take back: a second undo sends nothing, so the other
    // window's rename stays.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    act(() => {
      result.current.undo();
    });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(api.scenes.put).toHaveBeenCalledTimes(3);
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

describe("the move of a group and the order of the scenes", () => {
  it("a move goes through the daemon's route, with the revs the page holds", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));

    await act(async () => {
      const answer = await result.current.moveGroup("left", "other", false);
      expect(answer?.group_id).toBe("left");
    });

    expect(api.scenes.moveGroup).toHaveBeenCalledTimes(1);
    const [sceneId, groupId, body] = api.scenes.moveGroup.mock.calls[0] as [string, string, GroupMove];
    expect(sceneId).toBe("main");
    expect(groupId).toBe("left");
    expect(body).toEqual({ to: "other", copy: false, expected_rev_from: 3, expected_rev_to: 4 });
  });

  it("an undo puts both scenes back, with the revs it just read", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));
    await act(async () => {
      await result.current.moveGroup("left", "other", false);
    });
    await waitFor(() => expect(result.current.canUndo).toBe(true));

    await act(async () => {
      result.current.undo();
    });
    await waitFor(() => expect(api.scenes.write).toHaveBeenCalledTimes(1));

    const [body] = api.scenes.write.mock.calls[0] as [ScenesWrite];
    expect(body.scenes?.map((s) => s.scene.id)).toEqual(["main", "other"]);
    // The revs are the ones the undo just read, not the ones the move left.
    expect(body.scenes?.map((s) => s.expected_rev)).toEqual([4, 5]);
    expect(body.scenes?.[0]?.scene.groups.map((g) => g.id)).toEqual(["left", "right"]);
    expect(body.scenes?.[1]?.scene.groups.map((g) => g.id)).toEqual(["all"]);
  });

  it("an undo that finds the scene changed writes nothing and says so", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));
    await act(async () => {
      await result.current.moveGroup("left", "other", false);
    });

    // Another window changed the source while the move was on the screen.
    held = [
      { ...rig.afterMain, groups: [{ ...rig.afterMain.groups[0]!, name: "Renamed" }] },
      rig.afterOther,
    ];
    await act(async () => {
      result.current.undo();
    });
    await waitFor(() => expect(result.current.error).toBe(NOTHING_UNDONE));
    expect(api.scenes.write).not.toHaveBeenCalled();
  });

  it("an undo of a copy writes the target only", async () => {
    const rig = moveRig();
    const copyAnswer: GroupMoved = {
      ...clone(rig.answer),
      group_id: "left-2",
      // A copy leaves the source as it was.
      scenes: [clone(rig.beforeMain), clone(rig.afterOther)],
    };
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [copyAnswer.scenes[0]!, copyAnswer.scenes[1]!];
      return clone(copyAnswer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));
    await act(async () => {
      await result.current.moveGroup("left", "other", true);
    });
    const call = api.scenes.moveGroup.mock.calls[0] as [string, string, GroupMove];
    expect(call[2].copy).toBe(true);

    await act(async () => {
      result.current.undo();
    });
    await waitFor(() => expect(api.scenes.write).toHaveBeenCalledTimes(1));
    const [body] = api.scenes.write.mock.calls[0] as [ScenesWrite];
    // The source was not changed by the copy, so it is not written back.
    expect(body.scenes?.map((s) => s.scene.id)).toEqual(["other"]);
  });

  it("a save the daemon refuses shows its message and drops the undo", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockRejectedValue(
      new ApiFailure(503, "unavailable", "the scenes cannot be saved yet: the file is blocked"),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));
    await act(async () => {
      await result.current.moveGroup("left", "other", false);
    });
    await act(async () => {
      result.current.undo();
    });
    await waitFor(() =>
      expect(result.current.error).toBe("the scenes cannot be saved yet: the file is blocked"),
    );
    // The step is dropped: it is neither undone nor redoable.
    expect(result.current.canUndo).toBe(false);
    expect(result.current.canRedo).toBe(false);
  });

  it("a move another window made first says that nothing moved", async () => {
    const rig = moveRig();
    api.scenes.list.mockImplementation(async () => listOf([rig.beforeMain, rig.beforeOther]));
    api.scenes.moveGroup.mockRejectedValue(
      new ApiFailure(409, "conflict", "this scene was changed in another window"),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));
    await act(async () => {
      await result.current.moveGroup("left", "other", false);
    });
    await waitFor(() => expect(result.current.error).toBe(NOTHING_MOVED));
    expect(api.scenes.write).not.toHaveBeenCalled();
  });

  it("an edit that lands first is carried by the rev the move sends", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));

    await act(async () => {
      result.current.edit((draft) => {
        draft.name = "Mine";
      }, "Rename the scene");
    });
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(1));

    await act(async () => {
      await result.current.moveGroup("left", "other", false);
    });
    const body = api.scenes.moveGroup.mock.calls.at(-1)?.[2] as GroupMove;
    // The edit's own answer (rev 4), not the rev the move was planned on.
    expect(body.expected_rev_from).toBe(4);
  });

  it("the order goes to the daemon with the order it last confirmed", async () => {
    api.scenes.list.mockImplementation(async () => clone(answer()));
    api.scenes.write.mockImplementation(async () => clone(answer()));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scenes.length).toBe(3));

    act(() => {
      result.current.reorderScenes(["focus", "gaming", "evening"]);
    });
    await waitFor(() => expect(api.scenes.write).toHaveBeenCalledTimes(1));
    const [body] = api.scenes.write.mock.calls[0] as [ScenesWrite];
    expect(body.order).toEqual(["focus", "gaming", "evening"]);
    expect(body.expected_order).toEqual(["gaming", "focus", "evening"]);
    // The order shows at once, before the daemon has answered.
    expect(result.current.scenes.map((s) => s.id)).toEqual(["focus", "gaming", "evening"]);
  });

  it("a burst of key presses is one order and one write", async () => {
    api.scenes.list.mockImplementation(async () => clone(answer()));
    api.scenes.write.mockImplementation(async () => clone(answer()));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scenes.length).toBe(3));

    vi.useFakeTimers();
    act(() => {
      result.current.reorderScenes(["focus", "gaming", "evening"], { coalesce: true });
    });
    act(() => {
      result.current.reorderScenes(["gaming", "focus", "evening"], { coalesce: true });
    });
    expect(api.scenes.write).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400);
    });
    expect(api.scenes.write).toHaveBeenCalledTimes(1);
    const [body] = api.scenes.write.mock.calls[0] as [ScenesWrite];
    expect(body.order).toEqual(["gaming", "focus", "evening"]);
  });

  it("an order another window made first says that nothing moved", async () => {
    api.scenes.list.mockImplementation(async () => clone(answer()));
    api.scenes.write.mockRejectedValue(
      new ApiFailure(409, "conflict", "the scenes were put in another order in another window"),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scenes.length).toBe(3));

    act(() => {
      result.current.reorderScenes(["focus", "gaming", "evening"]);
    });
    await waitFor(() => expect(result.current.error).toBe(NOTHING_MOVED));
    // The daemon's order is the one that counts: the page goes back to it.
    expect(result.current.scenes.map((s) => s.id)).toEqual(["gaming", "focus", "evening"]);
  });

  it("two orders that come one after the other are written one after the other", async () => {
    api.scenes.list.mockImplementation(async () => clone(answer()));
    const writes: ScenesWrite[] = [];
    api.scenes.write.mockImplementation(async (body: ScenesWrite) => {
      writes.push(clone(body));
      return clone(answer());
    });

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scenes.length).toBe(3));

    act(() => {
      result.current.reorderScenes(["focus", "gaming", "evening"]);
    });
    act(() => {
      result.current.reorderScenes(["gaming", "focus", "evening"]);
    });

    await waitFor(() => expect(api.scenes.write).toHaveBeenCalledTimes(2));
    // The second write names the order the first one sent: the daemon has that one.
    expect(writes[0]?.order).toEqual(["focus", "gaming", "evening"]);
    expect(writes[0]?.expected_order).toEqual(["gaming", "focus", "evening"]);
    expect(writes[1]?.order).toEqual(["gaming", "focus", "evening"]);
    expect(writes[1]?.expected_order).toEqual(["focus", "gaming", "evening"]);
    // Nothing was refused.
    expect(result.current.error).toBeNull();
  });

  it("the strip follows the daemon's order again after the reorder", async () => {
    // The daemon's order: the list and the write answer with it.
    let held = clone(DEMO_SCENES.scenes);
    const list = (): ScenesResponse => {
      const scene = held.find((s) => s.id === DEMO_SCENES.active) ?? held[0];
      return {
        active: DEMO_SCENES.active,
        scenes: clone(held),
        group_states: scene?.groups.map((g) => ({ id: g.id, failed: null, fits: [] })) ?? [],
      };
    };
    const put = (order: string[]): void => {
      const byId = new Map(held.map((s) => [s.id, s]));
      held = order.map((id) => byId.get(id)).filter((s): s is Scene => s !== undefined);
    };
    api.scenes.list.mockImplementation(async () => list());
    api.scenes.write.mockImplementation(async (body: ScenesWrite) => {
      if (body.order) put(body.order);
      return list();
    });

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scenes.length).toBe(3));
    const before = result.current.scenes.map((s) => s.id);

    // The scenes are put in another order.
    const after = [before[1]!, before[0]!, before[2]!];
    act(() => {
      result.current.reorderScenes(after);
    });
    await waitFor(() => expect(api.scenes.write).toHaveBeenCalledTimes(1));
    // The daemon's answer is the order now, and the strip follows it.
    await waitFor(() => expect(result.current.scenes.map((s) => s.id)).toEqual(after));

    // Another window put the scenes in another order: the strip shows that one too.
    const other = [before[2]!, before[0]!, before[1]!];
    put(other);
    await act(async () => {
      await refreshScenes();
    });
    await waitFor(() => expect(result.current.scenes.map((s) => s.id)).toEqual(other));
  });

  it("the dry run asks the daemon with its own dry run, and writes nothing", async () => {
    const rig = moveRig();
    api.scenes.list.mockImplementation(async () => listOf([rig.beforeMain, rig.beforeOther]));
    api.scenes.moveGroup.mockImplementation(async () => ({ ...clone(rig.answer), saved: false }));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));

    await act(async () => {
      const check = await result.current.dryRunMove("left", "other", false);
      expect(check.state).toBe("answered");
    });

    const [sceneId, groupId, body] = api.scenes.moveGroup.mock.calls[0] as [string, string, GroupMove];
    expect(sceneId).toBe("main");
    expect(groupId).toBe("left");
    expect(body.dry_run).toBe(true);
    expect(body.expected_rev_from).toBe(3);
    expect(body.expected_rev_to).toBe(4);
    expect(api.scenes.write).not.toHaveBeenCalled();
    expect(api.scenes.put).not.toHaveBeenCalled();
  });

  it("a dry run the daemon refuses says the daemon's own words", async () => {
    const rig = moveRig();
    api.scenes.list.mockImplementation(async () => listOf([rig.beforeMain, rig.beforeOther]));
    api.scenes.moveGroup.mockRejectedValue(
      new ApiFailure(422, "validation", htmlLimitSentence(3)),
    );

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));

    await act(async () => {
      const check = await result.current.dryRunMove("left", "other", false);
      expect(check).toEqual({ state: "refused", message: htmlLimitSentence(3) });
    });
  });

  it("a move sends the live edits of the scene's groups first, and carries the revision they left", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    // The brightness write answers with the scene one rev on, the way the daemon answers.
    api.scenes.putGroupBrightness.mockImplementation(async () => ({
      ...clone(rig.beforeMain),
      rev: rig.beforeMain.rev + 1,
    }));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));

    await act(async () => {
      // The brightness is moved while the throttle window is still open: the write waits.
      result.current.setBrightness("left", 40);
      await result.current.moveGroup("left", "other", false);
    });

    expect(api.scenes.putGroupBrightness).toHaveBeenCalledTimes(1);
    // The edit went out before the move.
    const edit = api.scenes.putGroupBrightness.mock.invocationCallOrder[0]!;
    const move = api.scenes.moveGroup.mock.invocationCallOrder[0]!;
    expect(edit).toBeLessThan(move);
    // The move carries the revision the edit left.
    const body = api.scenes.moveGroup.mock.calls[0]?.[2] as GroupMove;
    expect(body.expected_rev_from).toBe(rig.beforeMain.rev + 1);
  });

  it("the move waits for the target scene's own write before it reads its revision", async () => {
    // The scene that plays changes while the old scene's write is still on its way: the move
    // goes to that old scene, and it may not carry a revision the write has not left yet.
    let active = "gaming";
    const list = () => ({
      active,
      scenes: clone(DEMO_SCENES.scenes),
      group_states: [] as ScenesResponse["group_states"],
    });
    api.scenes.list.mockImplementation(async () => list());
    api.scenes.setActive.mockImplementation(async (id: string) => {
      active = id;
      return list();
    });
    let release: (scene: Scene) => void = () => {};
    api.scenes.put.mockImplementation(
      async () =>
        new Promise<Scene>((resolve) => {
          release = resolve;
        }),
    );
    api.scenes.moveGroup.mockImplementation(async () => ({
      group_id: "g-case",
      saved: true,
      scenes: [],
      before: [],
      taken: [],
      emptied: [],
    }));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("gaming"));

    // An edit of the scene that plays goes out and stays on its way.
    await act(async () => {
      result.current.edit((draft) => {
        draft.name = "Mine";
      }, "Rename the scene");
    });
    expect(api.scenes.put).toHaveBeenCalledTimes(1);

    // Another scene plays now, and the move goes to the one whose write is still on its way.
    await act(async () => {
      result.current.setActive("focus");
    });
    await waitFor(() => expect(result.current.scene?.id).toBe("focus"));

    let done = false;
    await act(async () => {
      const moved = result.current
        .moveGroup("g-case", "gaming", false)
        .then(() => {
          done = true;
        });
      // The move waits for the write of the scene it goes to.
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      expect(api.scenes.moveGroup).not.toHaveBeenCalled();
      const pending = api.scenes.put.mock.calls[0] as [string, { expected_rev: number }];
      // The write lands.
      release({ ...clone(DEMO_SCENES.scenes[0]!), rev: pending[1].expected_rev + 1 });
      await moved;
    });

    expect(done).toBe(true);
    const pending = api.scenes.put.mock.calls[0] as [string, { expected_rev: number }];
    const body = api.scenes.moveGroup.mock.calls[0]?.[2] as GroupMove;
    // The revision the write left, not the one before it.
    expect(body.expected_rev_to).toBe(pending[1].expected_rev + 1);
  });

  it("an undo that finds the scene renamed writes nothing and says so", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));
    await act(async () => {
      await result.current.moveGroup("left", "other", false);
    });

    // Another window renamed the scene after the move: the undo would take the name back.
    held = [{ ...rig.afterMain, name: "Renamed" }, rig.afterOther];
    await act(async () => {
      result.current.undo();
    });
    await waitFor(() => expect(result.current.error).toBe(NOTHING_UNDONE));
    expect(api.scenes.write).not.toHaveBeenCalled();
  });

  it("the move says what happened, and offers the Undo while it is the top step", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));
    await act(async () => {
      await result.current.moveGroup("left", "other", false);
    });

    await waitFor(() => expect(notifySpy).toHaveBeenCalled());
    const [text, tone, action] = notifySpy.mock.calls.at(-1)! as [
      string,
      string,
      { label: string; onClick: () => void },
    ];
    expect(text).toBe("Moved LEFT to Other.");
    expect(tone).toBe("ok");
    expect(action.label).toBe("Undo");
    // The Undo takes the move back.
    action.onClick();
    await waitFor(() => expect(api.scenes.write).toHaveBeenCalledTimes(1));
  });

  it("the toast offers no Undo when another change came after the move", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));
    await act(async () => {
      await result.current.moveGroup("left", "other", false);
    });
    await waitFor(() => expect(notifySpy).toHaveBeenCalled());
    expect((notifySpy.mock.calls.at(-1) as unknown[])[2]).toBeDefined();

    // Another change came after the move: the Undo would take that change back instead, so the
    // toast says what happened and offers nothing.
    await act(async () => {
      result.current.edit((draft) => {
        draft.name = "Mine";
      }, "Rename the scene");
    });
    await waitFor(() => expect((notifySpy.mock.calls.at(-1) as unknown[])[2]).toBeUndefined());
  });

  it("the move's toast is said once, and a later change does not say it again", async () => {
    const rig = moveRig();
    let held = [rig.beforeMain, rig.beforeOther];
    api.scenes.list.mockImplementation(async () => listOf(held));
    api.scenes.moveGroup.mockImplementation(async () => {
      held = [rig.afterMain, rig.afterOther];
      return clone(rig.answer);
    });
    api.scenes.write.mockImplementation(async () => listOf(held));

    const { result } = renderHook(() => useScene());
    await waitFor(() => expect(result.current.scene?.id).toBe("main"));
    await act(async () => {
      await result.current.moveGroup("left", "other", false);
    });

    // The toast is said once, with the Undo of the move.
    await waitFor(() => expect(notifySpy).toHaveBeenCalledTimes(1));
    const [text, tone, action] = notifySpy.mock.calls.at(0)! as [
      string,
      string,
      { label: string; onClick: () => void },
    ];
    expect(text).toBe("Moved LEFT to Other.");
    expect(tone).toBe("ok");
    expect(action.label).toBe("Undo");

    // Another change came after the move: the toast says what happened and offers nothing.
    await act(async () => {
      result.current.edit((draft) => {
        draft.name = "Mine";
      }, "Rename the scene");
    });
    await waitFor(() => expect((notifySpy.mock.calls.at(-1) as unknown[])[2]).toBeUndefined());
    const said = notifySpy.mock.calls.length;

    // A later change does not say the move again: the toast is gone, and it stays gone.
    await act(async () => {
      result.current.edit((draft) => {
        const group = draft.groups[0];
        if (group) group.brightness = 40;
      }, "Change the brightness");
    });
    expect(notifySpy).toHaveBeenCalledTimes(said);
  });
});
