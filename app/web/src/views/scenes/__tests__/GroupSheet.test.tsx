import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useEffect } from "react";
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
    },
    lights: { list: vi.fn(), patch: vi.fn(), flash: vi.fn() },
    effect: vi.fn(),
    createPreset: vi.fn(),
  },
  notifySpy: vi.fn(),
}));

vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("@/components/Notify", () => ({ notify: notifySpy }));
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
    group_states: s.groups.map((g) => ({ id: g.id, failed: null })),
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
  { id: "aurora", name: "Aurora", description: "", needs: { screen: false, audio: false }, favorite: false },
  { id: "fire", name: "Fire", description: "", needs: { screen: false, audio: false }, favorite: false },
];

/** The rev the fake daemon is at. */
let serverRev = 7;

beforeEach(() => {
  for (const group of Object.values(api.scenes)) group.mockReset();
  for (const fn of Object.values(api.lights)) fn.mockReset();
  api.effect.mockReset();
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
  api.lights.list.mockResolvedValue({ lights: clone(DEMO_LIGHTS) });
  api.lights.flash.mockResolvedValue({ ok: true });
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

/** The hook the sheet is given, and the last of it, so a test can undo. */
let latest: UseScene | null = null;
const take = (use: UseScene) => {
  latest = use;
};

function Harness({ groupId }: { groupId: string }) {
  const use = useScene();
  useEffect(() => {
    take(use);
  }, [use]);
  return <GroupSheet groupId={groupId} use={use} lights={clone(DEMO_LIGHTS)} onClose={() => {}} />;
}

function open(groupId = "g-one") {
  render(<Harness groupId={groupId} />);
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
    const action = notifySpy.mock.calls.at(-1)?.[2] as { label: string; onClick: () => void };
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
});
