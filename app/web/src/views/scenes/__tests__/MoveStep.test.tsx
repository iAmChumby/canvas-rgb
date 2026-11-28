import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Profiler } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The move step: one row per other scene, the key that does it, and the line that says what it
// does.
//
// The daemon is the truth: the step asks the daemon's own dry run, once per scene, and says
// what it answered. A row the daemon refuses says the daemon's own words.

const { api, notifySpy } = vi.hoisted(() => ({
  api: {
    stacks: vi.fn(),
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
    lights: { list: vi.fn(), patch: vi.fn(), flash: vi.fn() },
  },
  notifySpy: vi.fn(),
}));

vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("@/components/Notify", () => ({ notify: notifySpy }));

import { DEMO_LIGHTS } from "@/lib/fixtures";
import { htmlLimitSentence } from "@/lib/htmlLimit";
import { MoveStep } from "@/views/scenes/MoveStep";
import { type MoveCheck } from "@/views/scenes/useScene";
import type { Group } from "@/proto/Group";
import type { GroupMoved } from "@/proto/GroupMoved";
import type { GroupTaken } from "@/proto/GroupTaken";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
import type { UseScene } from "@/views/scenes/useScene";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
});

const clone = <T,>(value: T): T => structuredClone(value);

const FIRST = DEMO_LIGHTS[0]?.light.id ?? "light-1";
const SECOND = DEMO_LIGHTS[1]?.light.id ?? "light-2";
const THIRD = DEMO_LIGHTS[2]?.light.id ?? "light-3";

function group(id: string, name: string, members: string[], effect = "solid") {
  return {
    id,
    name,
    members,
    plays: { type: "effect" as const, effect, props: {} },
    mode: "flow_across" as const,
    brightness: 100,
  };
}

/** The scene the group is in, and the two scenes it may go to. */
function scenes(): { from: Scene; to: Scene; third: Scene } {
  const from: Scene = {
    id: "gaming",
    name: "Gaming",
    rev: 7,
    groups: [group("g-desk", "Desk", [FIRST]), group("g-case", "Case fans", [SECOND])],
  };
  const to: Scene = {
    id: "evening",
    name: "Evening",
    rev: 2,
    groups: [group("g-warm", "Warm Wash", [SECOND])],
  };
  const third: Scene = {
    id: "focus",
    name: "Focus",
    rev: 1,
    groups: [group("g-lamp", "Lamp", [THIRD])],
  };
  return { from, to, third };
}

/**
 * The daemon's own answer to a dry run of the move of `groupId` into Evening: the scenes after,
 * the scenes before, the lights it took and the groups it emptied.
 *
 * `revs` are the revisions the daemon answered with: the answer carries the scenes before the
 * move, at the revisions the daemon had.
 */
function dryAnswer(groupId: string, revs = { from: 7, to: 2 }): GroupMoved {
  const { from, to } = scenes();
  const moving = from.groups.find((g) => g.id === groupId);
  if (!moving) throw new Error(`the scene holds no group ${groupId}`);
  const taken: GroupTaken[] = [];
  const emptied: string[] = [];
  const kept: Group[] = [];
  for (const other of to.groups) {
    const gone = other.members.filter((m) => moving.members.includes(m));
    if (gone.length === 0) {
      kept.push(other);
      continue;
    }
    taken.push({ group_id: other.id, group_name: other.name, lights: gone });
    const left = other.members.filter((m) => !moving.members.includes(m));
    if (left.length === 0) emptied.push(other.name);
    else kept.push({ ...other, members: left });
  }
  kept.push({ ...moving });
  return {
    group_id: groupId,
    saved: false,
    scenes: [
      { ...from, rev: from.rev + 1, groups: from.groups.filter((g) => g.id !== groupId) },
      { ...to, rev: to.rev + 1, groups: kept },
    ],
    before: [{ ...from, rev: revs.from }, { ...to, rev: revs.to }],
    taken,
    emptied,
  };
}

function fakeUse(over: Partial<UseScene> = {}): UseScene {
  const { from, to, third } = scenes();
  return {
    scene: from,
    scenes: [from, to, third],
    unassigned: [],
    states: [],
    edit: vi.fn(),
    setProps: vi.fn(),
    setBrightness: vi.fn(),
    setActive: vi.fn(),
    create: vi.fn(),
    remove: vi.fn(),
    renameScene: vi.fn(),
    dryRunMove: vi.fn(async (): Promise<MoveCheck> => ({ state: "answered", answer: dryAnswer("g-desk") })),
    moveGroup: vi.fn(async () => null),
    reorderScenes: vi.fn(),
    reorderGroups: vi.fn(),
    undo: vi.fn(),
    redo: vi.fn(),
    canUndo: false,
    canRedo: false,
    undoLabel: null,
    error: null,
    ...over,
  };
}

/** The lights of the editor: the second one is away. */
function lights(): LightView[] {
  return DEMO_LIGHTS.slice(0, 2).map((l, i) => ({
    ...clone(l),
    connected: i === 0,
  }));
}

beforeEach(() => {
  api.stacks.mockReset();
  api.stacks.mockResolvedValue({ stacks: [] });
  api.scenes.write.mockReset();
  api.scenes.put.mockReset();
  api.scenes.moveGroup.mockReset();
  notifySpy.mockReset();
});

afterEach(() => {
  cleanup();
});

describe("the move step", () => {
  it("lists the other scenes, one row each, and not the scene the group is in", async () => {
    render(<MoveStep groupId="g-desk" copy={false} use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByTestId("move-to-evening")).toBeTruthy());
    expect(screen.queryByTestId("move-to-gaming")).toBeNull();
    expect(screen.getByTestId("move-to-evening").textContent).toContain("Move to Evening");
  });

  it("asks the daemon about every scene once, and writes nothing", async () => {
    const use = fakeUse();
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    await waitFor(() => expect(use.dryRunMove).toHaveBeenCalledTimes(2));
    expect(use.dryRunMove).toHaveBeenCalledWith("g-desk", "evening", false);
    expect(use.dryRunMove).toHaveBeenCalledWith("g-desk", "focus", false);
    // Nothing was written: the daemon's answer is a dry run.
    expect(use.moveGroup).not.toHaveBeenCalled();
    expect(api.scenes.write).not.toHaveBeenCalled();
    expect(api.scenes.put).not.toHaveBeenCalled();
  });

  it("asks again when the step switches between moving and copying", async () => {
    const use = fakeUse();
    const view = render(
      <MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />,
    );
    await waitFor(() => expect(use.dryRunMove).toHaveBeenCalledTimes(2));
    view.rerender(<MoveStep groupId="g-desk" copy use={use} lights={lights()} onClose={vi.fn()} />);
    await waitFor(() => expect(use.dryRunMove).toHaveBeenCalledTimes(4));
    expect(use.dryRunMove).toHaveBeenLastCalledWith("g-desk", "focus", true);
  });

  it("a row is not enabled with the other mode's answer in the first render of the other mode", async () => {
    // The answer of the move lands.
    const use = fakeUse();
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    await act(async () => {
      root.render(
        <MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />,
      );
    });
    const row = await screen.findByTestId("move-to-evening");
    await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));
    expect(document.getElementById("move-outcome-evening")?.textContent).toContain("Adds 1 device.");

    // The step switches to the copy. The first render of the copy is committed on its own, and
    // the row shows what that render holds: the copy's own question has no answer yet, so the
    // row is held.
    try {
      act(() => {
        flushSync(() => {
          root.render(
            <MoveStep groupId="g-desk" copy use={use} lights={lights()} onClose={vi.fn()} />,
          );
        });
        // The row is not the move's answer: it is held until the daemon answers the copy.
        expect((row as HTMLButtonElement).disabled).toBe(true);
        expect(document.getElementById("move-outcome-evening")?.textContent).not.toContain("Adds 1 device.");
      });
    } finally {
      // The step goes away, even when the reads above failed: a step left on the page would be
      // found by the tests that follow.
      await act(async () => {
        root.unmount();
      });
      container.remove();
    }
  });

  it("a row is not enabled with the other mode's answer when the step switches", async () => {
    // The first answers land; the ones for the copy are still on their way.
    let round = 0;
    const use = fakeUse({
      dryRunMove: vi.fn((): Promise<MoveCheck> => {
        round += 1;
        if (round <= 2) return Promise.resolve({ state: "answered", answer: dryAnswer("g-desk") });
        return new Promise<MoveCheck>(() => undefined);
      }),
    });
    const view = render(
      <MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />,
    );
    const row = await screen.findByTestId("move-to-evening");
    await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));
    // The move's answer is on the row.
    expect(document.getElementById("move-outcome-evening")?.textContent).toContain("Adds 1 device.");

    // The step switches to the copy: the row is held, and it does not keep the move's answer.
    view.rerender(<MoveStep groupId="g-desk" copy use={use} lights={lights()} onClose={vi.fn()} />);
    expect((row as HTMLButtonElement).disabled).toBe(true);
    expect(document.getElementById("move-outcome-evening")?.textContent).not.toContain("Adds 1 device.");
    // The daemon is asked again, about the copy.
    await waitFor(() => expect(use.dryRunMove).toHaveBeenCalledTimes(4));
    expect(use.dryRunMove).toHaveBeenLastCalledWith("g-desk", "focus", true);
  });

  it("the rows ask again when a scene's revision changes", async () => {
    const use = fakeUse();
    const view = render(
      <MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />,
    );
    await waitFor(() => expect(use.dryRunMove).toHaveBeenCalledTimes(2));

    // Another window saved the scene the group is in: it is one rev on, so the answers on the
    // rows are older than the daemon now is.
    const { from, to, third } = scenes();
    const moved = { ...from, rev: from.rev + 1 };
    view.rerender(
      <MoveStep
        groupId="g-desk"
        copy={false}
        use={{ ...use, scene: moved, scenes: [moved, to, third] }}
        lights={lights()}
        onClose={vi.fn()}
      />,
    );
    await waitFor(() => expect(use.dryRunMove).toHaveBeenCalledTimes(4));
  });

  it("the effect that asks the daemon writes nothing: a new question draws the step once", async () => {
    // The daemon never answers: only the effect's own write can draw the step again.
    const use = fakeUse({
      dryRunMove: vi.fn(() => new Promise<MoveCheck>(() => undefined)),
    });
    // Every draw of the step is counted: a write of state inside it is a draw of its own.
    let draws = 0;
    const step = (scene: Scene, scenes: Scene[]) => (
      <Profiler
        id="move"
        onRender={() => {
          draws += 1;
        }}
      >
        <MoveStep
          groupId="g-desk"
          copy={false}
          use={{ ...use, scene, scenes }}
          lights={lights()}
          onClose={vi.fn()}
        />
      </Profiler>
    );
    const { from, to, third } = scenes();
    const view = render(step(from, [from, to, third]));
    await waitFor(() => expect(use.dryRunMove).toHaveBeenCalledTimes(2));
    const before = draws;

    // Another window saved the scene the group is in: the question is a new one, and the rows ask
    // the daemon again.
    const moved = { ...from, rev: from.rev + 1 };
    view.rerender(step(moved, [moved, to, third]));
    await waitFor(() => expect(use.dryRunMove).toHaveBeenCalledTimes(4));
    // The work the effect left is done.
    await act(async () => {});

    // The effect wrote nothing: the step drew itself once for the new question.
    expect(draws - before).toBe(1);
  });

  it("a row is held until the daemon answers, and it says so in the same box", async () => {
    const releases: Record<string, (answer: MoveCheck) => void> = {};
    const use = fakeUse({
      dryRunMove: vi.fn(
        (_group: string, to: string) =>
          new Promise<MoveCheck>((resolve) => {
            releases[to] = resolve;
          }),
      ),
    });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    expect((row as HTMLButtonElement).disabled).toBe(true);
    // The line is there, in the same box, and it is marked as waiting: the loading kit draws
    // the wait, so the line holds no sentence yet.
    const waiting = document.getElementById("move-outcome-evening");
    expect(waiting?.getAttribute("data-state")).toBe("pending");
    expect(waiting?.textContent).not.toContain("Could not check");
    await act(async () => {
      releases["evening"]?.({ state: "answered", answer: dryAnswer("g-desk") });
    });
    await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));
  });

  it("says what the daemon answered: what lands in the scene and what is taken out of it", async () => {
    const use = fakeUse({
      dryRunMove: vi.fn(async (): Promise<MoveCheck> => ({
        state: "answered",
        answer: dryAnswer("g-case"),
      })),
    });
    render(<MoveStep groupId="g-case" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    await waitFor(() =>
      expect(document.getElementById("move-outcome-evening")?.textContent).toContain(
        "Takes 1 device from Warm Wash",
      ),
    );
    const line = document.getElementById("move-outcome-evening");
    expect(line?.textContent).toContain("Adds 1 device.");
    expect(line?.textContent).toContain("Warm Wash has no devices left and is removed.");
  });

  it("says how many of the group's devices are away", async () => {
    render(<MoveStep groupId="g-case" copy={false} use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    await waitFor(() =>
      expect(document.getElementById("move-outcome-evening")?.textContent).toContain("not connected"),
    );
    expect(document.getElementById("move-outcome-evening")?.textContent).toContain("1 not connected.");
  });

  it("the outcome line is the row's own description, so it is read on focus", async () => {
    render(<MoveStep groupId="g-desk" copy={false} use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    const described = row.getAttribute("aria-describedby");
    expect(described).toBe("move-outcome-evening");
    await waitFor(() => expect(document.getElementById(described ?? "")?.textContent).toContain("Adds 1 device."));
  });

  it("the step opens with the focus on the first scene row", async () => {
    render(<MoveStep groupId="g-desk" copy={false} use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    await waitFor(() =>
      expect(document.activeElement?.getAttribute("data-testid")).toBe("move-to-evening"),
    );
  });

  it("a move the daemon refuses is disabled, with the daemon's own sentence", async () => {
    const sentence = htmlLimitSentence(3);
    const use = fakeUse({
      dryRunMove: vi.fn(async (): Promise<MoveCheck> => ({ state: "refused", error: sentence })),
    });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    await waitFor(() => expect(document.getElementById("move-outcome-evening")?.textContent).toBe(sentence));
    expect((row as HTMLButtonElement).disabled).toBe(true);
  });

  it("a dry run the daemon could not answer leaves the row disabled, with a way to ask again", async () => {
    const dryRunMove = vi.fn(async (): Promise<MoveCheck> => ({ state: "failed" }));
    const use = fakeUse({ dryRunMove });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByTestId("move-retry-evening")).toBeTruthy());
    expect(document.getElementById("move-outcome-evening")?.textContent).toBe("Could not check this move.");
    expect((screen.getByTestId("move-to-evening") as HTMLButtonElement).disabled).toBe(true);
    // The retry asks the daemon again, once, about that scene (the step had asked once for
    // every scene it lists).
    const asked = dryRunMove.mock.calls.length;
    fireEvent.click(screen.getByTestId("move-retry-evening"));
    await waitFor(() => expect(dryRunMove).toHaveBeenCalledTimes(asked + 1));
  });

  it("an answer that comes back after the step closed changes nothing", async () => {
    let release: (answer: MoveCheck) => void = () => {};
    const use = fakeUse({
      dryRunMove: vi.fn(() => new Promise<MoveCheck>((resolve) => { release = resolve; })),
    });
    const view = render(
      <MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />,
    );
    await screen.findByTestId("move-to-evening");
    view.unmount();
    await act(async () => {
      release({ state: "answered", answer: dryAnswer("g-desk") });
    });
    expect(use.moveGroup).not.toHaveBeenCalled();
    expect(notifySpy).not.toHaveBeenCalled();
  });

  it("a copy keeps the group in its scene and says so in the row", async () => {
    render(<MoveStep groupId="g-case" copy use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    expect(row.textContent).toContain("Copy to Evening");
  });

  it("confirming sends the real move with the revisions the dry run answered with", async () => {
    const use = fakeUse({
      dryRunMove: vi.fn(async (): Promise<MoveCheck> => ({
        state: "answered",
        answer: dryAnswer("g-desk"),
      })),
    });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(row);
    await waitFor(() => expect(use.moveGroup).toHaveBeenCalled());
    // The revisions are the ones the dry run answered with: the daemon has them.
    expect(use.moveGroup).toHaveBeenCalledWith("g-desk", "evening", false, { from: 7, to: 2 });
  });

  it("two clicks make one move: every row is held while a move is on its way", async () => {
    let release: (answer: GroupMoved | null) => void = () => {};
    const use = fakeUse({
      dryRunMove: vi.fn(async (): Promise<MoveCheck> => ({
        state: "answered",
        answer: dryAnswer("g-desk"),
      })),
      moveGroup: vi.fn(() => new Promise<GroupMoved | null>((resolve) => { release = resolve; })),
    });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    const evening = await screen.findByTestId("move-to-evening");
    const focus = await screen.findByTestId("move-to-focus");
    await waitFor(() => expect((evening as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(evening);
    await waitFor(() => expect((focus as HTMLButtonElement).disabled).toBe(true));
    fireEvent.click(focus);
    expect(use.moveGroup).toHaveBeenCalledTimes(1);
    await act(async () => {
      release(null);
    });
  });

  it("a move the daemon refuses leaves the sheet open", async () => {
    const use = fakeUse({
      dryRunMove: vi.fn(async (): Promise<MoveCheck> => ({
        state: "answered",
        answer: dryAnswer("g-desk"),
      })),
      moveGroup: vi.fn(async () => null),
    });
    const onClose = vi.fn();
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={onClose} />);
    const row = await screen.findByTestId("move-to-evening");
    await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(row);
    await waitFor(() => expect(use.moveGroup).toHaveBeenCalled());
    expect(onClose).not.toHaveBeenCalled();
  });

  it("a confirm the daemon refuses asks every row again, and the next confirm carries the new revisions", async () => {
    let round = 0;
    const use = fakeUse({
      dryRunMove: vi.fn(async (): Promise<MoveCheck> => {
        round += 1;
        // The first answer is the old one; the answers after the refusal carry the revisions the
        // daemon has now.
        return {
          state: "answered",
          answer: round <= 2 ? dryAnswer("g-desk") : dryAnswer("g-desk", { from: 8, to: 3 }),
        };
      }),
      // The daemon refused the confirm (a 409): nothing moved.
      moveGroup: vi.fn(async () => null),
    });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));
    expect(use.dryRunMove).toHaveBeenCalledTimes(2);

    // The confirm is refused: it carried the revisions of the answer that was on the row.
    fireEvent.click(row);
    await waitFor(() => expect(use.moveGroup).toHaveBeenCalledTimes(1));
    expect(use.moveGroup).toHaveBeenLastCalledWith("g-desk", "evening", false, { from: 7, to: 2 });

    // Every row asks again: the answers on the rows are older than the daemon now is.
    await waitFor(() => expect(use.dryRunMove).toHaveBeenCalledTimes(4));
    await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));

    // The next confirm carries the revisions the new answer was made with.
    fireEvent.click(row);
    await waitFor(() => expect(use.moveGroup).toHaveBeenCalledTimes(2));
    expect(use.moveGroup).toHaveBeenLastCalledWith("g-desk", "evening", false, { from: 8, to: 3 });
  });

  it("the revisions are the ones of the scenes named, whatever order the answer gives them in", async () => {
    const answer = dryAnswer("g-desk");
    // The daemon's answer with the scenes the other way round: the scene the group is in is
    // still the one at revision 7, and Evening the one at 2.
    const swapped: GroupMoved = { ...answer, before: [answer.before[1]!, answer.before[0]!] };
    const use = fakeUse({
      dryRunMove: vi.fn(async (): Promise<MoveCheck> => ({ state: "answered", answer: swapped })),
    });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(row);
    await waitFor(() => expect(use.moveGroup).toHaveBeenCalled());
    expect(use.moveGroup).toHaveBeenCalledWith("g-desk", "evening", false, { from: 7, to: 2 });
  });

  it("the step closes when its group is gone", async () => {
    const { from } = scenes();
    const use = fakeUse({ scene: { ...from, groups: from.groups.filter((g) => g.id !== "g-desk") } });
    const onClose = vi.fn();
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={onClose} />);
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});
