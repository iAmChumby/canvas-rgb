import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The move step: one row per other scene, the action, and the line that says what it does.
//
// The daemon is the truth: the page's own copy of the rules says what will happen, and the
// daemon's answer says what did.

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
import { MoveStep } from "@/views/scenes/MoveStep";
import type { GroupMoved } from "@/proto/GroupMoved";
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

/** The scene the group is in, and the scene it may go to. */
function scenes(): { from: Scene; to: Scene } {
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
  return { from, to };
}

function fakeUse(over: Partial<UseScene> = {}): UseScene {
  const { from, to } = scenes();
  return {
    scene: from,
    scenes: [from, to],
    unassigned: [],
    states: [],
    edit: vi.fn(),
    setProps: vi.fn(),
    setBrightness: vi.fn(),
    setActive: vi.fn(),
    create: vi.fn(),
    remove: vi.fn(),
    renameScene: vi.fn(),
    moveGroup: vi.fn(async () => null),
    reorderScenes: vi.fn(),
    undo: vi.fn(),
    redo: vi.fn(),
    canUndo: false,
    canRedo: false,
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

  it("says what lands in the scene and what is taken out of it", async () => {
    render(<MoveStep groupId="g-case" copy={false} use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    // The group holds the light the target's group holds: it is taken out of it, and the target's
    // group is left with nothing.
    const line = await screen.findByText(/Takes 1 device from Warm Wash/);
    expect(line.textContent).toContain("Adds 1 device.");
    expect(line.textContent).toContain("Warm Wash has no devices left and is removed.");
  });

  it("says how many of the group's devices are away", async () => {
    render(<MoveStep groupId="g-case" copy={false} use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    const line = await screen.findByText(/not connected/);
    expect(line.textContent).toContain("1 not connected.");
  });

  it("the outcome line is the row's own description, so it is read on focus", async () => {
    render(<MoveStep groupId="g-desk" copy={false} use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    const described = row.getAttribute("aria-describedby");
    expect(described).toBe("move-outcome-evening");
    expect(document.getElementById(described ?? "")?.textContent).toContain("Adds 1 device.");
  });

  it("the step opens with the focus on the first scene row", async () => {
    render(<MoveStep groupId="g-desk" copy={false} use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    await waitFor(() => expect(document.activeElement?.getAttribute("data-testid")).toBe("move-to-evening"));
  });

  it("a move the daemon would refuse is disabled, with the daemon's own sentence", async () => {
    // The target runs two HTML effects and the group that would go plays one too: three. The
    // group holds a light of its own, so the move takes nothing out of the target.
    const { from, to } = scenes();
    const htmlTo: Scene = {
      ...to,
      groups: [
        group("h1", "H1", [FIRST], "html:one"),
        group("h2", "H2", [SECOND], "html:two"),
      ],
    };
    const htmlFrom: Scene = {
      ...from,
      groups: [group("g-desk", "Desk", [THIRD], "html:three"), group("g-case", "Case fans", [SECOND])],
    };
    const use = fakeUse({ scene: htmlFrom, scenes: [htmlFrom, htmlTo] });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    expect((row as HTMLButtonElement).disabled).toBe(true);
    const line = document.getElementById("move-outcome-evening");
    expect(line?.textContent).toBe(
      "at most 2 groups of a scene may play an HTML effect (a stack with an HTML layer counts); 3 do",
    );
  });

  it("a stack with an HTML layer counts toward the limit", async () => {
    api.stacks.mockResolvedValue({ stacks: [{ id: "stack:Glow", name: "Glow", layers: 2, html_layers: 1, missing: [] }] });
    const { from, to } = scenes();
    const stackTo: Scene = {
      ...to,
      groups: [
        group("h1", "H1", [FIRST], "html:one"),
        group("s1", "S1", [SECOND], "stack:Glow"),
      ],
    };
    const htmlFrom: Scene = {
      ...from,
      groups: [group("g-desk", "Desk", [THIRD], "html:three"), group("g-case", "Case fans", [SECOND])],
    };
    const use = fakeUse({ scene: htmlFrom, scenes: [htmlFrom, stackTo] });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    // The stack holds an HTML layer, so the scene would run three HTML groups: refused.
    expect((row as HTMLButtonElement).disabled).toBe(true);
    expect(document.getElementById("move-outcome-evening")?.textContent).toContain("3 do");
  });

  it("a stack without an HTML layer is not one", async () => {
    api.stacks.mockResolvedValue({ stacks: [{ id: "stack:Plain", name: "Plain", layers: 2, html_layers: 0, missing: [] }] });
    const { from, to } = scenes();
    const stackTo: Scene = {
      ...to,
      groups: [
        group("h1", "H1", [FIRST], "html:one"),
        group("s1", "S1", [SECOND], "stack:Plain"),
      ],
    };
    const htmlFrom: Scene = {
      ...from,
      groups: [group("g-desk", "Desk", [THIRD], "html:three"), group("g-case", "Case fans", [SECOND])],
    };
    const use = fakeUse({ scene: htmlFrom, scenes: [htmlFrom, stackTo] });
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    // Two HTML groups are the limit: the move is allowed.
    expect((row as HTMLButtonElement).disabled).toBe(false);
  });

  it("a copy keeps the group in its scene and says so in the row", async () => {
    render(<MoveStep groupId="g-case" copy use={fakeUse()} lights={lights()} onClose={vi.fn()} />);
    const row = await screen.findByTestId("move-to-evening");
    expect(row.textContent).toContain("Copy to Evening");
  });

  it("the toast says what happened and names the groups the move emptied", async () => {
    const answer: GroupMoved = {
      group_id: "g-case",
      saved: true,
      scenes: [],
      before: [],
      taken: [],
      emptied: ["Warm Wash"],
    };
    const undo = vi.fn();
    const use = fakeUse({ moveGroup: vi.fn(async () => answer), undo });
    const onClose = vi.fn();
    render(<MoveStep groupId="g-case" copy={false} use={use} lights={lights()} onClose={onClose} />);
    const row = await screen.findByTestId("move-to-evening");
    fireEvent.click(row);
    await waitFor(() => expect(notifySpy).toHaveBeenCalled());
    const [text, tone, action] = notifySpy.mock.calls[0] as [string, string, { label: string; onClick: () => void }];
    expect(text).toBe("Moved Case fans to Evening. Warm Wash had no lights left and was removed.");
    expect(tone).toBe("warn");
    expect(action.label).toBe("Undo");
    expect(onClose).toHaveBeenCalled();
    action.onClick();
    expect(undo).toHaveBeenCalled();
  });

  it("a move the daemon refuses leaves the sheet open and says nothing of its own", async () => {
    const use = fakeUse({ moveGroup: vi.fn(async () => null) });
    const onClose = vi.fn();
    render(<MoveStep groupId="g-desk" copy={false} use={use} lights={lights()} onClose={onClose} />);
    const row = await screen.findByTestId("move-to-evening");
    fireEvent.click(row);
    await waitFor(() => expect(use.moveGroup).toHaveBeenCalled());
    expect(onClose).not.toHaveBeenCalled();
    expect(notifySpy).not.toHaveBeenCalled();
  });
});
