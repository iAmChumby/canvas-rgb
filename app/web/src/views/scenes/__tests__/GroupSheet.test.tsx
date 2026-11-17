import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useEffect, useState } from "react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The open group: the effect's own controls, its presets and the devices in it.
//
// The scene data layer is the real one, against a fake daemon: what the sheet sends is what the
// daemon would receive, and the undo that brings a deleted group back is the real undo.

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
    lights: { list: vi.fn(), patch: vi.fn(), flash: vi.fn() },
    stacks: vi.fn(),
    effect: vi.fn(),
    createPreset: vi.fn(),
  },
  notifySpy: vi.fn(),
}));

vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("@/components/Notify", () => ({ notify: notifySpy, dismissNotification: vi.fn() }));
vi.mock("next/navigation", () => ({
  usePathname: () => "/",
  useRouter: () => ({ push: () => {}, replace: () => {}, prefetch: () => {} }),
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
}));

import { patchSnap } from "@/lib/store";
import { DEMO_LIGHTS } from "@/lib/fixtures";
import { GroupSheet } from "@/views/scenes/GroupSheet";
import { useScene } from "@/views/scenes/useScene";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { GroupMove } from "@/proto/GroupMove";
import type { LightFit } from "@/proto/LightFit";
import type { Scene } from "@/proto/Scene";
import type { ScenesResponse } from "@/proto/ScenesResponse";
import type { Props } from "@/proto/props";
import type { UseScene } from "@/views/scenes/useScene";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  globalThis.IntersectionObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  } as unknown as typeof IntersectionObserver;
});

const clone = <T,>(value: T): T => structuredClone(value);

const FIRST = DEMO_LIGHTS[0];
const SECOND = DEMO_LIGHTS[1];
const THIRD = DEMO_LIGHTS[2];

/** The scene the fake daemon holds: two groups, each with one device, each playing Aurora. */
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
        plays: { type: "effect", effect: "aurora", props: {} },
        mode: "flow_across",
        brightness: 100,
      },
      {
        id: "g-two",
        name: "Other",
        members: [SECOND?.light.id ?? ""],
        plays: { type: "effect", effect: "aurora", props: {} },
        mode: "flow_across",
        brightness: 100,
      },
    ],
  };
}

function answer(): ScenesResponse {
  const s = scene();
  return {
    active: s.id,
    scenes: [s],
    group_states: s.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
  };
}

/** Aurora: one number, one built-in preset. */
const DETAIL: EffectDetail = {
  schema: {
    id: "aurora",
    name: "Aurora",
    description: "",
    needs: { screen: false, audio: false },
    properties: [
      { key: "speed", label: "Speed", kind: { type: "number", min: 0, max: 40, step: 1, default: 8 } },
    ],
    presets: [{ name: "Slow dusk", props: { speed: { type: "number", value: 3 } } }],
  },
  favorite: false,
  user_presets: [],
  props: { speed: { type: "number", value: 8 } },
};

const EFFECTS: EffectSummary[] = [
  { id: "aurora", name: "Aurora", description: "", needs: { screen: false, audio: false }, favorite: false, kind: "canvas" },
  { id: "fire", name: "Fire", description: "", needs: { screen: false, audio: false }, favorite: false, kind: "canvas" },
  // The pattern the first group plays (0.16.0): built on a ring of 12.
  {
    id: "pattern:Intake",
    name: "Intake",
    description: "Pattern, ring of 12",
    needs: { screen: false, audio: false },
    favorite: false,
    kind: "pattern",
    built_on: { led_count: 12, arrangement: "ring" },
  },
];

/** The label of the control that shares a group's effect across its lights. */
const MODE_LABEL = "How this group's lights share the effect";

/** The scene the fake daemon answers with from now on, and the store's copy of it. */
function serve(next: ScenesResponse): void {
  api.scenes.list.mockResolvedValue(clone(next));
  patchSnap({ scenes: clone(next), scenesError: null });
}

/**
 * The demo scene with three devices in its first group, playing `pattern:Intake` with the mode it
 * stored before it did, and the flags the daemon sends for it.
 */
function patternAnswer(fits: LightFit[]): ScenesResponse {
  const s = scene();
  const first = s.groups[0];
  if (!first) return answer();
  first.members = [FIRST?.light.id ?? "", SECOND?.light.id ?? "", THIRD?.light.id ?? ""];
  first.mode = "each_its_own";
  first.plays = { type: "effect", effect: "pattern:Intake", props: {} };
  return {
    active: s.id,
    scenes: [s],
    group_states: [
      { id: first.id, failed: null, fits },
      { id: "g-two", failed: null, fits: [] },
    ],
  };
}

/** The rev the fake daemon is at. */
let serverRev = 7;

/** The scene the group is in, and another scene it may go to. */
function twoScenes(): ScenesResponse {
  const s = scene();
  const other: Scene = {
    id: "evening",
    name: "Evening",
    rev: 2,
    groups: [
      {
        id: "g-warm",
        name: "Warm Wash",
        members: [SECOND?.light.id ?? ""],
        plays: { type: "effect", effect: "aurora", props: {} },
        mode: "flow_across",
        brightness: 100,
      },
    ],
  };
  return {
    active: s.id,
    scenes: [s, other],
    group_states: s.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
  };
}

beforeEach(() => {
  for (const group of Object.values(api.scenes)) group.mockReset();
  for (const fn of Object.values(api.lights)) fn.mockReset();
  api.effect.mockReset();
  api.stacks.mockReset();
  api.stacks.mockResolvedValue({ stacks: [] });
  notifySpy.mockReset();
  serverRev = scene().rev;
  api.scenes.list.mockResolvedValue(answer());
  api.scenes.put.mockImplementation(
    async (_id: string, body: { scene: Scene; expected_rev: number }) => {
      serverRev = body.expected_rev + 1;
      return { ...body.scene, rev: serverRev };
    },
  );
  api.scenes.patchGroupProps.mockImplementation(
    async (_sceneId: string, groupId: string, props: Props) => {
      serverRev += 1;
      const next = clone(scene());
      next.rev = serverRev;
      const g = next.groups.find((x) => x.id === groupId);
      if (g && g.plays.type === "effect") g.plays = { ...g.plays, props: { ...g.plays.props, ...props } };
      return next;
    },
  );
  api.scenes.putGroupBrightness.mockImplementation(async () => {
    serverRev += 1;
    const next = clone(scene());
    next.rev = serverRev;
    return next;
  });
  // The daemon's answer to the move route: the step asks it with a dry run, and the answer is
  // what the row says.
  api.scenes.moveGroup.mockImplementation(
    async (_sceneId: string, groupId: string, body: GroupMove) => {
      const from = scene();
      const to = twoScenes().scenes.find((s) => s.id === body.to) ?? from;
      const moving = from.groups.find((g) => g.id === groupId) ?? from.groups[0]!;
      return {
        group_id: groupId,
        saved: !body.dry_run,
        scenes: [
          { ...from, rev: from.rev + 1, groups: from.groups.filter((g) => g.id !== groupId) },
          { ...to, rev: to.rev + 1, groups: [...to.groups, moving] },
        ],
        before: [from, to],
        taken: [],
        emptied: [],
      };
    },
  );
  api.lights.list.mockResolvedValue({ lights: clone(DEMO_LIGHTS) });
  api.lights.flash.mockResolvedValue({ ok: true });
  api.effect.mockResolvedValue(clone(DETAIL));
  api.stacks.mockResolvedValue({ stacks: [] });
  patchSnap({
    scenes: answer(),
    scenesError: null,
    lights: clone(DEMO_LIGHTS),
    lightsError: null,
    effects: EFFECTS,
  });
});

afterEach(() => {
  cleanup();
});

/** The hook the sheet is given, and the last of it, so a test can undo. */
let latest: UseScene | null = null;
const take = (use: UseScene) => {
  latest = use;
};

function Harness({ groupId }: { groupId: string }) {
  const use = useScene();
  const [closed, setClosed] = useState(false);
  useEffect(() => {
    take(use);
  }, [use]);
  if (closed) return null;
  return (
    <GroupSheet
      groupId={groupId}
      use={use}
      lights={clone(DEMO_LIGHTS)}
      onClose={() => setClosed(true)}
    />
  );
}

function open(groupId = "g-one") {
  render(<Harness groupId={groupId} />);
}

/** What the last whole-scene write carried for the group. */
function lastPlays(): { type: string; effect?: string } | null {
  const call = api.scenes.put.mock.calls.at(-1);
  if (!call) return null;
  const body = call[1] as { scene: Scene };
  const group = body.scene.groups.find((g) => g.id === "g-one");
  return group?.plays.type === "effect"
    ? { type: "effect", effect: group.plays.effect }
    : { type: "off" };
}

describe("the open group", () => {
  it("changing a control calls setProps for this group only", async () => {
    open("g-one");
    const slider = await screen.findByRole("slider", { name: "Speed" });
    fireEvent.keyDown(slider, { key: "ArrowRight" });
    await waitFor(() => expect(api.scenes.patchGroupProps).toHaveBeenCalled());
    const [sceneId, groupId, props] = api.scenes.patchGroupProps.mock.calls[0] as [
      string,
      string,
      Props,
    ];
    expect(sceneId).toBe("gaming");
    expect(groupId).toBe("g-one");
    expect(props).toEqual({ speed: { type: "number", value: 9 } });
    // The other group was not touched.
    const groups = api.scenes.patchGroupProps.mock.calls.map((c) => c[1]);
    expect(groups.every((g) => g === "g-one")).toBe(true);
  });

  it("choosing a preset copies its values into the group", async () => {
    open("g-one");
    const preset = await screen.findByRole("button", { name: "Use the preset Slow dusk" });
    fireEvent.click(preset);
    await waitFor(() => expect(api.scenes.patchGroupProps).toHaveBeenCalled());
    const [, groupId, props] = api.scenes.patchGroupProps.mock.calls[0] as [string, string, Props];
    expect(groupId).toBe("g-one");
    expect(props).toEqual({ speed: { type: "number", value: 3 } });
  });

  it("removing the last member deletes the group and undo brings it back", async () => {
    open("g-one");
    const name = FIRST?.label ?? "";
    const take = await screen.findByRole("button", { name: `Take ${name} out of this group` });
    fireEvent.click(take);
    // The group is gone: the sheet has nothing to show.
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalled());
    const sent = api.scenes.put.mock.calls.at(-1)?.[1] as { scene: Scene };
    expect(sent.scene.groups.map((g) => g.id)).toEqual(["g-two"]);
    // The undo toast offers the undo.
    expect(notifySpy).toHaveBeenCalled();
    const [message, , action] = notifySpy.mock.calls.at(-1) as [
      string,
      string,
      { label: string; onClick: () => void },
    ];
    expect(message).toBe("One device had no devices left and was removed");
    expect(action.label).toBe("Undo");
    // The change is on the undo stack: the owner can take it back.
    expect(latest?.canUndo).toBe(true);
    act(() => {
      action.onClick();
    });
    await waitFor(() => expect(screen.getByRole("dialog")).toBeTruthy());
    const back = api.scenes.put.mock.calls.at(-1)?.[1] as { scene: Scene };
    expect(back.scene.groups.map((g) => g.id)).toEqual(["g-one", "g-two"]);
  });

  it("the menu offers a move and a copy, and both are disabled with one scene", async () => {
    open("g-one");
    fireEvent.click(await screen.findByRole("button", { name: "More about this group" }));
    const move = await screen.findByRole("button", { name: /Move to scene/ });
    const copy = await screen.findByRole("button", { name: /Copy to scene/ });
    expect((move as HTMLButtonElement).disabled).toBe(true);
    expect(move.getAttribute("title")).toBe("There is no other scene.");
    expect((copy as HTMLButtonElement).disabled).toBe(true);
    expect(copy.getAttribute("title")).toBe("There is no other scene.");
  });

  it("the menu has Move up and Move down, and they are disabled at an end", async () => {
    open("g-one");
    fireEvent.click(await screen.findByRole("button", { name: "More about this group" }));
    const up = await screen.findByRole("button", { name: "Move up" });
    const down = await screen.findByRole("button", { name: "Move down" });
    // The group is the first one: it cannot go up.
    expect((up as HTMLButtonElement).disabled).toBe(true);
    expect(up.getAttribute("title")).toBe("It is already first.");
    expect((down as HTMLButtonElement).disabled).toBe(false);
  });

  it("the last group cannot go down, and the first one cannot go up", async () => {
    open("g-two");
    fireEvent.click(await screen.findByRole("button", { name: "More about this group" }));
    const up = await screen.findByRole("button", { name: "Move up" });
    const down = await screen.findByRole("button", { name: "Move down" });
    // The group is the last one: it cannot go down, but it can go up.
    expect((down as HTMLButtonElement).disabled).toBe(true);
    expect(down.getAttribute("title")).toBe("It is already last.");
    expect((up as HTMLButtonElement).disabled).toBe(false);
  });

  it("Move down puts the group one place down, in one save with the revision the page read", async () => {
    open("g-one");
    fireEvent.click(await screen.findByRole("button", { name: "More about this group" }));
    fireEvent.click(await screen.findByRole("button", { name: "Move down" }));
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalledTimes(1));
    const [id, body] = api.scenes.put.mock.calls[0] as [
      string,
      { scene: Scene; expected_rev: number },
    ];
    expect(id).toBe("gaming");
    // One save: the whole scene, with the new order and the revision the page read.
    expect(body.expected_rev).toBe(7);
    expect(body.scene.groups.map((g) => g.id)).toEqual(["g-two", "g-one"]);
  });

  it("choosing Move to scene opens the step, and it lists the other scene", async () => {
    api.scenes.list.mockResolvedValue(twoScenes());
    patchSnap({ scenes: twoScenes(), scenesError: null });
    open("g-one");
    fireEvent.click(await screen.findByRole("button", { name: "More about this group" }));
    const move = await screen.findByRole("button", { name: /Move to scene/ });
    expect((move as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(move);
    // The step is open: it names the scene the group may go to, and what the move does. The
    // line is the daemon's own answer, so it comes when the daemon has answered.
    expect(await screen.findByText("Move to Evening")).toBeTruthy();
    expect(await screen.findByText(/Adds 1 device\./)).toBeTruthy();
  });

  it("the step goes back to the group, and the way out of it is the sheet's own", async () => {
    api.scenes.list.mockResolvedValue(twoScenes());
    patchSnap({ scenes: twoScenes(), scenesError: null });
    open("g-one");
    fireEvent.click(await screen.findByRole("button", { name: "More about this group" }));
    fireEvent.click(await screen.findByRole("button", { name: /Move to scene/ }));
    await screen.findByText("Move to Evening");
    // The header's key is the way back: the step closes into the group.
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    await waitFor(() => expect(screen.queryByText("Move to Evening")).toBeNull());
    expect(screen.getByText("Presets")).toBeTruthy();
  });

  it("says no Save and no Revert: every change is undoable", async () => {
    open("g-one");
    await screen.findByRole("dialog");
    const names = [...document.querySelectorAll("button")].map((b) =>
      (b.textContent ?? "").trim(),
    );
    expect(names).not.toContain("Save");
    expect(names).not.toContain("Revert");
    // The one key that writes is the plan's own: a preset of the effect, not the group.
    expect(names.some((n) => n === "Save as preset")).toBe(true);
    const labels = [...document.querySelectorAll("[aria-label]")].map((e) =>
      e.getAttribute("aria-label"),
    );
    expect(labels).not.toContain("Save");
    expect(labels).not.toContain("Revert");
    expect(within(screen.getByRole("dialog")).getByText("Presets")).toBeTruthy();
  });

  it("every way out of the group puts the preview back", async () => {
    // The Escape key, the scrim and the back key all close the group, and the picker that was
    // showing an effect puts the old one back: the lights never keep a picture the owner did
    // not choose, and the undo stack never held it.
    for (const way of ["escape", "scrim", "back"]) {
      api.scenes.put.mockClear();
      open("g-one");
      await screen.findByRole("dialog");
      fireEvent.click(screen.getByTestId("sheet-effect"));
      await screen.findByTestId("effect-picker");
      const tile = screen.getByRole("button", { name: /Fire/ });
      fireEvent.pointerEnter(tile);
      await waitFor(() => expect(lastPlays()).toEqual({ type: "effect", effect: "fire" }));
      if (way === "escape") fireEvent.keyDown(document, { key: "Escape" });
      if (way === "scrim") fireEvent.click(screen.getByTestId("sheet-scrim"));
      if (way === "back") fireEvent.click(screen.getByRole("button", { name: "Back" }));
      await waitFor(() => expect(lastPlays()).toEqual({ type: "effect", effect: "aurora" }));
      cleanup();
    }
  });

  // Plan Task 11: the open group of a pattern says how the pattern lands on each device, hides a
  // mode that does not apply, and keeps the mode the group stored for when it plays a canvas
  // effect again.
  it("the open group shows a flag on each stretched device", async () => {
    const fits: LightFit[] = [
      { light: FIRST?.light.id ?? "", fit: "stretched" },
      { light: SECOND?.light.id ?? "", fit: "other_shape" },
      { light: THIRD?.light.id ?? "", fit: "one_led" },
    ];
    serve(patternAnswer(fits));
    open("g-one");
    await screen.findByRole("dialog");
    const rows = [...document.querySelectorAll("[data-member]")];
    expect(rows).toHaveLength(3);
    // Nothing is hidden: every device of the group carries what the pattern does to it.
    expect(rows[0]?.textContent).toContain("Stretched");
    expect(rows[1]?.textContent).toContain("Built for a different shape");
    expect(rows[2]?.textContent).toContain("One LED shows the first colour");
  });

  it("the mode control is hidden for a group playing a pattern and returns for a canvas effect", async () => {
    serve(patternAnswer([]));
    open("g-one");
    await screen.findByRole("dialog");
    // Every device plays the whole pattern, so how the lights share the effect has no meaning.
    expect(screen.queryByRole("radiogroup", { name: MODE_LABEL })).toBeNull();
    cleanup();
    serve(answer());
    open("g-two");
    expect(await screen.findByRole("radiogroup", { name: MODE_LABEL })).toBeTruthy();
  });

  it("the stored mode survives playing a pattern and back", async () => {
    serve(patternAnswer([]));
    open("g-one");
    await screen.findByRole("dialog");
    expect(screen.queryByRole("radiogroup", { name: MODE_LABEL })).toBeNull();
    // Back to a canvas effect: the mode the group kept is the one it plays.
    fireEvent.click(screen.getByTestId("sheet-effect"));
    fireEvent.click(await screen.findByRole("button", { name: /Fire/ }));
    await waitFor(() => expect(lastPlays()).toEqual({ type: "effect", effect: "fire" }));
    const sent = api.scenes.put.mock.calls.at(-1)?.[1] as { scene: Scene };
    expect(sent.scene.groups.find((g) => g.id === "g-one")?.mode).toBe("each_its_own");
    const group = await screen.findByRole("radiogroup", { name: MODE_LABEL });
    expect(within(group).getByRole("radio", { name: "each its own" }).getAttribute("aria-checked")).toBe(
      "true",
    );
  });

  // #468 phase 4: how a pattern runs through the group's lights.
  describe("the shape of a pattern's run", () => {
    const SHAPE_LABEL = "How this pattern runs through the group's lights";
    /** What the last whole-scene write carried for `g-one`'s sequence. */
    const lastSequence = () => {
      const body = api.scenes.put.mock.calls.at(-1)?.[1] as { scene: Scene };
      return body.scene.groups.find((g) => g.id === "g-one")?.sequence;
    };

    it("a pattern group offers Each light, One ring and One line, and starts on Each light", async () => {
      serve(patternAnswer([]));
      open("g-one");
      const shape = await screen.findByRole("radiogroup", { name: SHAPE_LABEL });
      const names = within(shape).getAllByRole("radio").map((r) => r.textContent);
      expect(names).toEqual(["Each light", "One ring", "One line"]);
      expect(within(shape).getByRole("radio", { name: "Each light" }).getAttribute("aria-checked")).toBe("true");
    });

    it("picking One ring then One line is saved as the group's sequence, and Each light takes it away", async () => {
      serve(patternAnswer([]));
      open("g-one");
      const shape = await screen.findByRole("radiogroup", { name: SHAPE_LABEL });
      fireEvent.click(within(shape).getByRole("radio", { name: "One ring" }));
      await waitFor(() => expect(lastSequence()).toBe("ring"));
      fireEvent.click(within(shape).getByRole("radio", { name: "One line" }));
      await waitFor(() => expect(lastSequence()).toBe("line"));
      expect(within(shape).getByRole("radio", { name: "One line" }).getAttribute("aria-checked")).toBe("true");
      fireEvent.click(within(shape).getByRole("radio", { name: "Each light" }));
      await waitFor(() => expect(api.scenes.put.mock.calls.length).toBeGreaterThan(2));
      // `None` is the absence of the field, as the daemon writes it.
      expect(lastSequence()).toBeUndefined();
      const body = api.scenes.put.mock.calls.at(-1)?.[1] as { scene: Scene };
      expect("sequence" in (body.scene.groups.find((g) => g.id === "g-one") ?? {})).toBe(false);
    });

    it("shows the sequence the group already has, and a canvas effect has no shape control", async () => {
      const answered = patternAnswer([]);
      const first = answered.scenes[0]?.groups[0];
      if (first) first.sequence = "ring";
      serve(answered);
      open("g-one");
      const shape = await screen.findByRole("radiogroup", { name: SHAPE_LABEL });
      expect(within(shape).getByRole("radio", { name: "One ring" }).getAttribute("aria-checked")).toBe("true");
      cleanup();
      serve(answer());
      open("g-two");
      await screen.findByRole("radiogroup", { name: MODE_LABEL });
      expect(screen.queryByRole("radiogroup", { name: SHAPE_LABEL })).toBeNull();
    });

    it("the members reorder from the keyboard, and the run follows that order", async () => {
      serve(patternAnswer([]));
      open("g-one");
      await screen.findByRole("radiogroup", { name: SHAPE_LABEL });
      const grips = () => [...document.querySelectorAll<HTMLElement>("[data-member] button")].filter((b) =>
        (b.getAttribute("aria-label") ?? "").startsWith("Move "),
      );
      fireEvent.keyDown(grips()[0] as HTMLElement, { key: "ArrowDown" });
      await waitFor(() => expect(api.scenes.put).toHaveBeenCalled());
      const body = api.scenes.put.mock.calls.at(-1)?.[1] as { scene: Scene };
      expect(body.scene.groups.find((g) => g.id === "g-one")?.members).toEqual([
        SECOND?.light.id,
        FIRST?.light.id,
        THIRD?.light.id,
      ]);
    });
  });
});

// #469: a group's list and its "flow across / each its own" switch were drawn in one scroll area,
// so a row could sit in the switch's pixels and be cut by the band's edge. jsdom has no layout, so
// this checks the structure that makes the overlap impossible: the band (the picture and the
// switch) is the sheet's own header, and everything that can scroll - the effect's presets and
// settings, the devices - is in the pane below it.
describe("the band and the sheet's pane are two boxes (#469)", () => {
  it("no list shares a box with the mode switch", async () => {
    open();
    await screen.findByRole("dialog");
    const sheet = screen.getByTestId("group-sheet");
    const pane = sheet.querySelector('[data-testid="sheet-scroll"]');
    expect(pane, "the sheet's own scrolling pane").not.toBeNull();
    const stage = screen.getByTestId("group-stage");
    const mode = screen.getByRole("radiogroup", { name: MODE_LABEL });
    const presets = screen.getByRole("group", { name: "The presets of this effect" });
    const devices = screen.getByRole("heading", { name: "The devices" });
    // The band is above the pane, never inside it: a row in the pane cannot reach the switch.
    expect(pane?.contains(stage)).toBe(false);
    expect(pane?.contains(mode)).toBe(false);
    // The list is in the pane, so it scrolls there and is cut by nothing but its own pane.
    expect(pane?.contains(presets)).toBe(true);
    expect(pane?.contains(devices)).toBe(true);
  });

  it("the effect step keeps the list in the same pane", async () => {
    open();
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByTestId("sheet-effect"));
    const picker = await screen.findByTestId("effect-picker");
    const pane = screen.getByTestId("group-sheet").querySelector('[data-testid="sheet-scroll"]');
    expect(pane?.contains(picker)).toBe(true);
  });
});
