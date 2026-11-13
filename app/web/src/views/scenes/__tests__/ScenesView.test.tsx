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
    effects: [],
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

  it("a failed group shows its failure text", () => {
    const base = fake();
    const first = (base.scene as Scene).groups[0];
    mount(fake({ states: [{ id: first?.id ?? "", failed: "the effect glow is not installed" }] }));
    expect(screen.getByText("the effect glow is not installed")).toBeTruthy();
  });

  it("a new unassigned light shows a notice whose action adds it to the chosen group", () => {
    const fresh: LightView = {
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
    const use = fake();
    use.unassigned = [...use.unassigned, fresh];
    patchSnap({ lights: [...clone(DEMO_LIGHTS), fresh] });
    mount(use);
    const notice = screen.getByRole("region", { name: /Desk lamp/ });
    const group = (use.scene as Scene).groups[0];
    fireEvent.click(within(notice).getByRole("button", { name: `Add to ${group?.name}` }));
    const edits = use.calls.edit ?? [];
    expect(edits).toHaveLength(1);
    const [draft] = edits[0] as [Scene];
    expect(draft.groups[0]?.members).toContain("openrgb:desk-lamp#z0");
  });

  it("the notice goes when it is dismissed", () => {
    const fresh: LightView = {
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
    const use = fake();
    use.unassigned = [...use.unassigned, fresh];
    patchSnap({ lights: [...clone(DEMO_LIGHTS), fresh] });
    mount(use);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("region", { name: /Desk lamp/ })).toBeNull();
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
