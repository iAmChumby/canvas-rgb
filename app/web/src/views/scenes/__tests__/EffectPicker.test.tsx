import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The picker a group opens to choose what it plays.
//
// Highlighting an entry previews it on the group's lights. The preview is not a change the owner
// made: closing the picker puts the old effect back, and the undo stack never held it.

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
    lights: { list: vi.fn(), patch: vi.fn(), flash: vi.fn() },
    effect: vi.fn(),
    stacks: vi.fn(),
    createPreset: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("@/components/Notify", () => ({ notify: vi.fn() }));

import { patchSnap } from "@/lib/store";
import { DEMO_LIGHTS } from "@/lib/fixtures";
import { htmlLimitSentence, SCENE_HTML_MAX } from "@/lib/htmlLimit";
import { EffectPicker } from "@/views/scenes/EffectPicker";
import { useScene } from "@/views/scenes/useScene";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
import type { ScenesResponse } from "@/proto/ScenesResponse";

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

/** The scene the fake daemon holds. `html` is how many OTHER groups play an HTML effect. */
function scene(html: number): Scene {
  const groups: Scene["groups"] = [
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
      members: [],
      plays: html > 0 ? { type: "effect", effect: "html:sample", props: {} } : { type: "off" },
      mode: "flow_across",
      brightness: 100,
    },
  ];
  // The limit is two: a second other group is what makes the picker refuse a third runtime.
  if (html > 1) {
    groups.push({
      id: "g-three",
      name: "Third",
      members: [],
      plays: { type: "effect", effect: "html:sample", props: {} },
      mode: "flow_across",
      brightness: 100,
    });
  }
  return { id: "gaming", name: "Gaming", rev: 7, groups };
}

function answer(html = 0): ScenesResponse {
  const s = scene(html);
  return {
    active: s.id,
    scenes: [s],
    group_states: s.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
  };
}

/** A scene whose other group plays a stack: the scene names it by id, so its layers come from
 *  `GET /stacks`. */
function stacked(): ScenesResponse {
  const s = scene(0);
  s.groups[1]!.plays = { type: "effect", effect: "stack:Twice", props: {} };
  return {
    active: s.id,
    scenes: [s],
    group_states: s.groups.map((g) => ({ id: g.id, failed: null, fits: [] })),
  };
}

const DETAIL: EffectDetail = {
  schema: {
    id: "fire",
    name: "Fire",
    description: "",
    needs: { screen: false, audio: false },
    properties: [{ key: "heat", label: "Heat", kind: { type: "number", min: 0, max: 10, step: 1, default: 5 } }],
    presets: [],
  },
  favorite: false,
  user_presets: [],
  props: { heat: { type: "number", value: 5 } },
};

const EFFECTS: EffectSummary[] = [
  { id: "aurora", name: "Aurora", description: "", needs: { screen: false, audio: false }, favorite: false, kind: "canvas" },
  { id: "fire", name: "Fire", description: "", needs: { screen: false, audio: false }, favorite: false, kind: "canvas" },
  { id: "html:sample", name: "Sample", description: "", needs: { screen: false, audio: false }, favorite: false, kind: "canvas" },
  { id: "late-effect", name: "Late", description: "", needs: { screen: false, audio: false }, favorite: false, kind: "canvas" },
  // Two patterns (0.16.0): one built on the fans the group holds, one built on a smaller ring.
  {
    id: "pattern:Front fans",
    name: "Front fans",
    description: "Pattern, ring of 40",
    needs: { screen: false, audio: false },
    favorite: false,
    kind: "pattern",
    built_on: { led_count: 40, arrangement: "ring" },
  },
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

/** A light as the daemon lists it: `leds` LEDs of a device, in `arrangement`. */
function light(id: string, label: string, leds: number, arrangement: LightView["light"]["arrangement"]): LightView {
  return {
    light: { id, device_id: id.split("#")[0] ?? id, zone_index: 0, arrangement, reversed: false },
    led_count: leds,
    connected: true,
    label,
  };
}

const FAN_A = light("demo:fan-a#z0", "Front fan 1", 40, "ring");
const FAN_B = light("demo:fan-b#z0", "Front fan 2", 40, "ring");

/** A scene whose one group holds two rings of 40: a ring of 40 fits it, a ring of 12 stretches both. */
function fanScene(): ScenesResponse {
  const s: Scene = {
    id: "gaming",
    name: "Gaming",
    rev: 7,
    groups: [
      {
        id: "g-fans",
        name: "Front fans",
        members: [FAN_A.light.id, FAN_B.light.id],
        plays: { type: "effect", effect: "aurora", props: {} },
        mode: "flow_across",
        brightness: 100,
      },
    ],
  };
  return { active: s.id, scenes: [s], group_states: [{ id: "g-fans", failed: null, fits: [] }] };
}

/** Serves the two-fan scene to the page and to the store, as the daemon would. */
function serveFans(): void {
  const answer = fanScene();
  api.scenes.list.mockResolvedValue(clone(answer));
  api.lights.list.mockResolvedValue({ lights: clone([FAN_A, FAN_B]) });
  patchSnap({
    scenes: clone(answer),
    scenesError: null,
    lights: clone([FAN_A, FAN_B]),
    lightsError: null,
    effects: EFFECTS,
  });
}

let serverRev = 7;

beforeEach(() => {
  for (const group of Object.values(api.scenes)) group.mockReset();
  for (const fn of Object.values(api.lights)) fn.mockReset();
  api.effect.mockReset();
  api.stacks.mockReset();
  serverRev = 7;
  api.scenes.list.mockResolvedValue(answer());
  api.stacks.mockResolvedValue({ stacks: [] });
  api.scenes.put.mockImplementation(
    async (_id: string, body: { scene: Scene; expected_rev: number }) => {
      serverRev = body.expected_rev + 1;
      return { ...body.scene, rev: serverRev };
    },
  );
  api.lights.list.mockResolvedValue({ lights: clone(DEMO_LIGHTS) });
  api.effect.mockResolvedValue(clone(DETAIL));
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

/** The hook the picker is given, and the last of it, so a test can look at the undo stack. */
let latest: ReturnType<typeof useScene> | null = null;
const take = (use: ReturnType<typeof useScene>) => {
  latest = use;
};

function Harness({ groupId = "g-one" }: { groupId?: string }) {
  const use = useScene();
  useEffect(() => {
    take(use);
  }, [use]);
  return <EffectPicker groupId={groupId} use={use} onClose={() => {}} />;
}

/** What the last whole-scene write carried for the group. */
function lastPlays(groupId = "g-one"): { type: string; effect?: string } | null {
  const call = api.scenes.put.mock.calls.at(-1);
  if (!call) return null;
  const body = call[1] as { scene: Scene };
  const group = body.scene.groups.find((g) => g.id === groupId);
  return group?.plays.type === "effect"
    ? { type: "effect", effect: group.plays.effect }
    : { type: "off" };
}

describe("the effect picker", () => {
  it("highlighting an effect previews it and closing restores the old one", async () => {
    const view = render(<Harness />);
    const tile = await screen.findByRole("button", { name: /Fire/ });
    fireEvent.pointerEnter(tile);
    await waitFor(() => expect(lastPlays()).toEqual({ type: "effect", effect: "fire" }));
    // The picker goes: the way out is the sheet's own (Escape, the scrim, the back key) or the
    // popover's (Escape, a click outside). Every one of them unmounts the picker, and the picker
    // puts the old effect back when it goes.
    view.unmount();
    await waitFor(() => expect(lastPlays()).toEqual({ type: "effect", effect: "aurora" }));
  });

  it("a preview is not on the undo stack: the owner's own choice is", async () => {
    render(<Harness />);
    const tile = await screen.findByRole("button", { name: /Fire/ });
    fireEvent.pointerEnter(tile);
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalled());
    // The preview went to the daemon, but it is not a change the owner made.
    expect(latest?.canUndo).toBe(false);
    fireEvent.click(tile);
    await waitFor(() => expect(latest?.canUndo).toBe(true));
    expect(lastPlays()).toEqual({ type: "effect", effect: "fire" });
  });

  it("a preview that lands after the picker closed writes nothing", async () => {
    // The effect's detail is on its way; the picker closes before it comes back.
    let answerDetail: (detail: EffectDetail) => void = () => undefined;
    api.effect.mockImplementation(
      () =>
        new Promise<EffectDetail>((resolve) => {
          answerDetail = resolve;
        }),
    );
    const view = render(<Harness />);
    const tile = await screen.findByRole("button", { name: /Late/ });
    fireEvent.pointerEnter(tile);
    view.unmount();
    await act(async () => {
      answerDetail(clone(DETAIL));
      await Promise.resolve();
    });
    // The late preview belongs to a picture nobody wants: nothing was written.
    expect(api.scenes.put).not.toHaveBeenCalled();
  });

  it("an HTML effect cannot be chosen when the scene's two runtimes are taken, and says why", async () => {
    patchSnap({ scenes: answer(2) });
    api.scenes.list.mockResolvedValue(answer(2));
    render(<Harness />);
    const html = await screen.findByRole("button", { name: /Sample/ });
    expect(html.getAttribute("aria-disabled")).toBe("true");
    // The row says it in a few words that fit, and carries the daemon's sentence as its tooltip:
    // two others plus this one.
    const note = within(html).getByText("2 HTML already");
    expect(note.getAttribute("title")).toBe(htmlLimitSentence(3));
    // Pointing at the row puts the whole sentence on the line under the list, and leaving it
    // gives the line back.
    expect(screen.queryByText(htmlLimitSentence(3))).toBeNull();
    fireEvent.pointerEnter(html);
    expect(screen.getByText(htmlLimitSentence(3)).getAttribute("data-why")).toBe("true");
    fireEvent.pointerLeave(html);
    expect(screen.queryByText(htmlLimitSentence(3))).toBeNull();
    // The keyboard gets the same line.
    fireEvent.focus(html);
    expect(screen.getByText(htmlLimitSentence(3))).toBeTruthy();
    fireEvent.blur(html);
    expect(screen.queryByText(htmlLimitSentence(3))).toBeNull();
    // Clicking it does nothing at all: no preview and no choice.
    fireEvent.click(html);
    fireEvent.pointerEnter(html);
    await new Promise((r) => setTimeout(r, 30));
    expect(api.scenes.put).not.toHaveBeenCalled();
  });

  it("a second HTML effect can be chosen beside one, and a third cannot", async () => {
    patchSnap({ scenes: answer(1) });
    api.scenes.list.mockResolvedValue(answer(1));
    render(<Harness />);
    // One other group plays an HTML effect: the scene's second slot is this one's.
    const html = await screen.findByRole("button", { name: /Sample/ });
    expect(html.getAttribute("aria-disabled")).toBeNull();
    expect(within(html).queryByText(/HTML already/)).toBeNull();
    fireEvent.click(html);
    await waitFor(() => expect(lastPlays()).toEqual({ type: "effect", effect: "html:sample" }));
  });

  it("a stack with two HTML layers takes both runtimes, and blocks an HTML effect", async () => {
    patchSnap({ scenes: stacked() });
    api.scenes.list.mockResolvedValue(stacked());
    // The scene names the stack by id only: the picker reads its layers from the stacks route.
    api.stacks.mockResolvedValue({
      stacks: [
        { id: "stack:Twice", name: "Twice", layers: 2, html_layers: SCENE_HTML_MAX, missing: [] },
      ],
    });
    render(<Harness groupId="g-one" />);
    const html = await screen.findByRole("button", { name: /Sample/ });
    await waitFor(() => expect(html.getAttribute("aria-disabled")).toBe("true"));
    expect(within(html).getByText("2 HTML already").getAttribute("title")).toBe(
      htmlLimitSentence(SCENE_HTML_MAX + 1),
    );
  });

  // Plan Task 11: patterns are in the picker with the effects, under a heading of their own, and
  // each says what it was built on and how it would land on this group.
  it("a Patterns heading lists the patterns with their kind", async () => {
    serveFans();
    render(<Harness groupId="g-fans" />);
    const heading = await screen.findByRole("heading", { name: "Patterns" });
    // After Stacks, as the plan says.
    expect(screen.getAllByRole("heading").map((h) => h.textContent)).toEqual([
      "Off",
      "Built-in",
      "Your effects",
      "Patterns",
    ]);
    const section = heading.closest("section") as HTMLElement;
    const rows = within(section).getAllByRole("button").map((b) => b.textContent ?? "");
    expect(rows.some((r) => r.includes("Front fans") && r.includes("ring of 40"))).toBe(true);
    expect(rows.some((r) => r.includes("Intake") && r.includes("ring of 12"))).toBe(true);
  });

  it("a pattern that fits every device is marked Fits", async () => {
    serveFans();
    render(<Harness groupId="g-fans" />);
    const fits = await screen.findByRole("button", { name: /Front fans/ });
    expect(within(fits).getByText("Fits")).toBeTruthy();
    // The one that would not fit carries the count instead, and no Fits mark.
    const stretches = screen.getByRole("button", { name: /Intake/ });
    expect(within(stretches).queryByText("Fits")).toBeNull();
  });

  it("a pattern that would stretch two devices says so and can still be chosen", async () => {
    serveFans();
    render(<Harness groupId="g-fans" />);
    const intake = await screen.findByRole("button", { name: /Intake/ });
    expect(within(intake).getByText("2 devices would be stretched")).toBeTruthy();
    // Nothing is refused: it is chosen like any effect.
    fireEvent.click(intake);
    await waitFor(() => expect(lastPlays("g-fans")).toEqual({ type: "effect", effect: "pattern:Intake" }));
  });
});
