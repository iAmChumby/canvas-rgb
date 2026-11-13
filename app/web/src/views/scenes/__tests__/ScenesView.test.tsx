import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The Scenes page: one row per group of the active scene, the scene strip, the Unassigned row and
// the notice for a light that was just detected. `useScene` and the light feed are replaced, so
// the tests see what the page asks of them and not the daemon.

const { pathRef } = vi.hoisted(() => ({ pathRef: { current: "/" } }));
vi.mock("next/navigation", () => ({
  usePathname: () => pathRef.current,
  useRouter: () => ({ push: () => {}, replace: () => {}, prefetch: () => {} }),
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
}));

const scene = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/views/scenes/useScene", () => ({ useScene: () => scene.current }));
vi.mock("@/views/scenes/useLightFeed", () => ({ useLightFeed: () => () => null }));

import { patchSnap } from "@/lib/store";
import { DEMO_LIGHTS, DEMO_SCENES } from "@/lib/fixtures";
import { Rail } from "@/shell/Rail";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { SEEN_LIGHTS_KEY } from "@/views/scenes/newLights";
import { ScenesView } from "@/views/scenes/ScenesView";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { Scene } from "@/proto/Scene";
import type { LightView } from "@/proto/LightView";
import type { GroupState } from "@/proto/GroupState";
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
});

const clone = <T,>(value: T): T => structuredClone(value);

const DEMO_EFFECTS = ["aurora", "rainbow_wave", "breathing", "spiral", "white", "fire"].map((id) => ({
  id,
  name: id.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase()),
  description: "",
  needs: { audio: false },
  favorite: false,
})) as unknown as EffectSummary[];

const FRESH: LightView = {
  light: {
    id: "openrgb:desk-lamp#z0",
    device_id: "openrgb:desk-lamp",
    zone_index: 0,
    arrangement: "single",
    reversed: false,
  },
  led_count: 1,
  connected: true,
  label: "Desk lamp",
};

const gaming = (): Scene => clone(DEMO_SCENES.scenes[0] as Scene);

function fake(over: Partial<UseScene> = {}): UseScene & { calls: Record<string, unknown[][]> } {
  const calls: Record<string, unknown[][]> = {};
  const spy =
    (name: string) =>
    (...args: unknown[]) => {
      (calls[name] ??= []).push(args);
    };
  const active = gaming();
  const held = new Set(active.groups.flatMap((g) => g.members));
  const base: UseScene = {
    scene: active,
    scenes: clone(DEMO_SCENES.scenes),
    unassigned: DEMO_LIGHTS.filter((l) => !held.has(l.light.id)),
    states: active.groups.map((g): GroupState => ({ id: g.id, failed: null })),
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
    undo: spy("undo"),
    redo: spy("redo"),
    canUndo: false,
    canRedo: false,
    error: null,
  };
  return Object.assign({ ...base, ...over }, { calls });
}

function mount(use: UseScene) {
  scene.current = use;
  return render(
    <StripSlotsProvider>
      <StripSlot kind="title" />
      <StripSlot kind="actions" />
      <Rail pathname="/" />
      <ScenesView />
    </StripSlotsProvider>,
  );
}

beforeEach(() => {
  window.localStorage.clear();
  // Every demo light has been seen, so only a light that appears later is "new".
  window.localStorage.setItem(SEEN_LIGHTS_KEY, JSON.stringify(DEMO_LIGHTS.map((l) => l.light.id)));
  patchSnap({
    status: null,
    scenes: { active: "gaming", scenes: clone(DEMO_SCENES.scenes), group_states: [] },
    scenesError: null,
    lights: clone(DEMO_LIGHTS),
    lightsError: null,
    effects: DEMO_EFFECTS,
  });
});

afterEach(() => {
  cleanup();
});

const rows = () => screen.getAllByTestId("group-row");

describe("the Scenes page", () => {
  it("renders one row per group in scene order", () => {
    const use = fake();
    mount(use);
    const names = rows().map((r) => within(r).getByTestId("group-name").textContent);
    expect(names).toEqual((use.scene as Scene).groups.map((g) => g.name));
  });

  it("Unassigned is absent when every light is in a group", () => {
    const one = clone(DEMO_SCENES.scenes[1] as Scene);
    mount(fake({ scene: one, unassigned: [], states: one.groups.map((g) => ({ id: g.id, failed: null })) }));
    expect(screen.queryByTestId("unassigned-row")).toBeNull();
  });

  it("shows the Unassigned row when a light has no group", () => {
    mount(fake());
    expect(screen.getByTestId("unassigned-row")).toBeTruthy();
  });

  it("pressing a scene key calls setActive with its id", () => {
    const use = fake();
    mount(use);
    fireEvent.click(screen.getByRole("button", { name: "Evening" }));
    expect(use.calls.setActive).toEqual([["evening"]]);
  });

  it("a failed group shows the daemon's own words when its effect is installed", () => {
    const base = fake();
    const first = (base.scene as Scene).groups[0];
    mount(fake({ states: [{ id: first?.id ?? "", failed: "the effect could not start" }] }));
    expect(screen.getByText("the effect could not start")).toBeTruthy();
  });

  it("a missing effect is said in words: 'Missing effect', the raw id only in a tooltip", () => {
    const evening = clone(DEMO_SCENES.scenes[2] as Scene);
    const lost = evening.groups[1];
    mount(
      fake({
        scene: evening,
        states: evening.groups.map((g) => ({
          id: g.id,
          failed: g.id === lost?.id ? "the effect html:glow-removed is not installed" : null,
        })),
      }),
    );
    const row = rows()[1] as HTMLElement;
    expect(within(row).getByRole("button", { name: /plays Missing effect/ })).toBeTruthy();
    expect(within(row).getByText("This effect is no longer installed.")).toBeTruthy();
    // The id never shows as text: only a tooltip holds it.
    expect(row.innerHTML).toContain("html:glow-removed");
    expect(document.body.textContent).not.toContain("html:glow-removed");
  });

  const withFresh = () => {
    const use = fake();
    use.unassigned = [...use.unassigned, FRESH];
    patchSnap({ lights: [...clone(DEMO_LIGHTS), FRESH] });
    mount(use);
    return use;
  };

  it("the notice's key for a non-first group adds the light to that group, and only that group", () => {
    const use = withFresh();
    const notice = screen.getByRole("region", { name: /Desk lamp/ });
    const groups = (use.scene as Scene).groups;
    const third = groups[2];
    fireEvent.click(within(notice).getByRole("button", { name: `Add to ${third?.name}` }));
    const edits = use.calls.edit ?? [];
    expect(edits).toHaveLength(1);
    const [draft] = edits[0] as [Scene];
    for (const g of draft.groups) {
      const holds = g.members.includes(FRESH.light.id);
      expect(holds, `group ${g.name}`).toBe(g.id === third?.id);
    }
  });

  it("shows three keys and a menu for the rest, and the menu adds to the group it names", () => {
    const use = withFresh();
    const notice = screen.getByRole("region", { name: /Desk lamp/ });
    const groups = (use.scene as Scene).groups;
    expect(groups.length).toBeGreaterThan(3);
    const keys = within(notice).getAllByRole("button", { name: /^Add to / });
    expect(keys).toHaveLength(3);
    fireEvent.click(within(notice).getByRole("button", { name: `${groups.length - 3} more` }));
    const last = groups[groups.length - 1];
    fireEvent.click(screen.getByRole("button", { name: `Add to ${last?.name}` }));
    const [draft] = (use.calls.edit ?? [])[0] as [Scene];
    expect(draft.groups[groups.length - 1]?.members).toContain(FRESH.light.id);
    expect(draft.groups[0]?.members).not.toContain(FRESH.light.id);
  });

  it("the notice comes before the rows, so a keyboard reaches it early and it covers none", () => {
    withFresh();
    const notice = screen.getByRole("region", { name: /Desk lamp/ });
    const firstRow = rows()[0] as HTMLElement;
    expect(notice.compareDocumentPosition(firstRow) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(notice).getByRole("button", { name: "Dismiss" })).toBeTruthy();
  });

  it("the notice goes when it is dismissed", () => {
    withFresh();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("region", { name: /Desk lamp/ })).toBeNull();
  });

  it("a row's open key claims no disclosure it does not have", () => {
    mount(fake());
    for (const key of screen.getAllByRole("button", { name: /^Open / })) {
      expect(key.getAttribute("aria-expanded")).toBeNull();
    }
  });

  it("never says '0 devices' while the lights are not read: it says it is reading, or that it could not", () => {
    patchSnap({ lights: null, lightsError: null });
    const { unmount } = mount(fake({ unassigned: [] }));
    for (const row of rows()) {
      expect(row.textContent).not.toMatch(/0 devices|0 LEDs/);
      expect(row.querySelector('[aria-busy="true"]')).not.toBeNull();
    }
    expect(document.body.textContent).not.toMatch(/LEDs/);
    unmount();
    patchSnap({ lights: null, lightsError: "refused" });
    mount(fake({ unassigned: [] }));
    for (const row of rows()) {
      expect(row.textContent).toContain("Devices not read");
      expect(row.textContent).not.toMatch(/0 devices|0 LEDs/);
    }
  });

  it("a scene with no groups says so, and does not say '0 LEDs'", () => {
    const empty = clone(DEMO_SCENES.scenes[0] as Scene);
    empty.groups = [];
    mount(fake({ scene: empty, states: [] }));
    expect(screen.getByText("This scene has no groups yet")).toBeTruthy();
    expect(screen.getByTestId("unassigned-row")).toBeTruthy();
    expect(document.body.textContent).not.toMatch(/ 0 LEDs/);
    expect(document.body.textContent).toContain("no groups yet");
  });

  it("the brightness control calls setBrightness", () => {
    const use = fake();
    mount(use);
    const first = rows()[0] as HTMLElement;
    const slider = within(first).getByRole("slider");
    fireEvent.keyDown(slider, { key: "ArrowLeft" });
    const group = (use.scene as Scene).groups[0];
    expect(use.calls.setBrightness).toEqual([[group?.id, (group?.brightness ?? 0) - 1]]);
  });

  it("the rail's first item is Scenes and the word Layout is not in the document", () => {
    patchSnap({ status: null });
    mount(fake());
    const first = screen.getAllByRole("link")[0] as HTMLElement;
    expect(first.textContent).toContain("Scenes");
    expect(document.body.textContent).not.toMatch(/layout/i);
  });
});
