// Task 11: the Effects page's Play action is "Play on …", and its controls set the effect's
// starting values. The page reads the active scene's groups from the snapshot, so each test
// puts the scene it needs there.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { getSnap, patchSnap, refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import type { Group } from "@/proto/Group";
import type { Scene } from "@/proto/Scene";
import type { ScenesResponse } from "@/proto/ScenesResponse";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/effects/",
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const URL = "/effects/?state=html,stack";

beforeAll(() => {
  // The demo backend reads its scenario once, on its first answer: every test uses this URL.
  window.history.replaceState({}, "", URL);
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
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function group(id: string, name: string, plays?: string): Group {
  return {
    id,
    name,
    members: ["openrgb:strip-1#z0"],
    plays: plays ? { type: "effect", effect: plays, props: {} } : { type: "off" },
    mode: "flow_across",
    brightness: 100,
  };
}

function scenesWith(groups: Group[]): ScenesResponse {
  const scene: Scene = { id: "s", name: "Scene", rev: 1, groups };
  return { active: "s", scenes: [scene], group_states: groups.map((g) => ({ id: g.id, failed: null })) };
}

async function mount(active: string) {
  await act(async () => {
    await refreshAll();
  });
  const selected = getSnap().effects?.find((e) => e.id === active);
  patchSnap({
    status: {
      ...getSnap().status!,
      active_scene: { id: "s", name: "Scene" },
      // Another effect is playing: the one under test is idle, so its action is Play.
      active_effect: "white",
      active_effects: ["white"],
    },
  });
  render(
    <>
      <NotifyHost />
      <StripSlotsProvider>
        <div className="strip-slots">
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </div>
        <EffectsView />
      </StripSlotsProvider>
    </>,
  );
  // Pick the effect under test from the library, and wait for its detail.
  fireEvent.click(await screen.findByRole("button", { name: `Open ${selected?.name ?? active}` }));
  await screen.findByRole("heading", { name: selected?.name ?? active });
}

describe("Play on …", () => {
  it("opens the effect and group a scene asked for, and offers the way back", async () => {
    window.history.replaceState({}, "", "/effects/?effect=aurora&group=g-case&state=html,stack");
    await act(async () => {
      await refreshAll();
    });
    patchSnap({
      status: {
        ...getSnap().status!,
        active_scene: { id: "s", name: "Scene" },
        active_effect: "white",
        active_effects: ["white"],
      },
      scenes: scenesWith([group("g-case", "Case fans"), group("g-ram", "Memory")]),
    });
    render(
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>,
    );
    // The effect the group asked for is the one on screen.
    expect(await screen.findByRole("heading", { name: "Aurora" })).toBeTruthy();
    // And the way back names the group it came from.
    const back = screen.getByRole("link", { name: "Back to Case fans" });
    expect(back.getAttribute("href")).toBe("/?group=g-case");
  });

  it("with one group Play plays there without a popover", async () => {
    window.history.replaceState({}, "", URL);
    await mount("aurora");
    patchSnap({ scenes: scenesWith([group("g-all", "Everything")]) });
    const play = await screen.findByRole("button", { name: "Play" });
    const spy = vi.spyOn(api, "activateEffect").mockResolvedValue({ ok: true });
    await act(async () => {
      fireEvent.click(play);
    });
    await waitFor(() =>
      expect(spy).toHaveBeenCalledWith("aurora", { props: {}, preset: null, groups: ["Everything"] }),
    );
    expect(screen.queryByRole("dialog", { name: "Play on" })).toBeNull();
  });

  it("with three groups Play offers them and sends the chosen names", async () => {
    window.history.replaceState({}, "", URL);
    await mount("aurora");
    const groups = [group("g-case", "Case fans"), group("g-ram", "Memory"), group("g-gpu", "Graphics card")];
    patchSnap({ scenes: scenesWith(groups) });
    const play = await screen.findByRole("button", { name: "Play on …" });
    const spy = vi.spyOn(api, "activateEffect").mockResolvedValue({ ok: true });
    fireEvent.click(play);
    const dialog = await screen.findByRole("dialog", { name: "Play on" });
    // Every group is ticked to begin with; picking one group makes it the only one.
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "Memory" }));
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Play" }));
    });
    await waitFor(() => expect(spy).toHaveBeenCalledWith("aurora", { props: {}, preset: null, groups: ["Memory"] }));
  });

  it("plays an effect that is already on one group on another group too", async () => {
    window.history.replaceState({}, "", URL);
    await mount("aurora");
    patchSnap({
      scenes: scenesWith([
        group("g-case", "Case fans", "aurora"),
        group("g-ram", "Memory"),
        group("g-gpu", "Graphics card"),
        group("g-desk", "Cooler and desk"),
      ]),
    });
    const play = await screen.findByRole("button", { name: "Playing on 1 of 4 · Play on …" });
    expect(play.getAttribute("aria-disabled")).toBeNull();
    fireEvent.click(play);
    const dialog = await screen.findByRole("dialog", { name: "Play on" });
    // The group already playing it is ticked, marked, and cannot be unticked.
    const caseFans = within(dialog).getByRole("checkbox", { name: "Case fans" });
    expect(caseFans.getAttribute("aria-checked")).toBe("true");
    expect(caseFans.getAttribute("aria-disabled")).toBe("true");
    fireEvent.click(caseFans);
    expect(caseFans.getAttribute("aria-checked")).toBe("true");
    expect(within(dialog).getByText("Playing")).toBeTruthy();
    const spy = vi.spyOn(api, "activateEffect").mockResolvedValue({ ok: true });
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "Memory" }));
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Play" }));
    });
    // Only the group that was picked: the playing group keeps its own values.
    await waitFor(() => expect(spy).toHaveBeenCalledWith("aurora", { props: {}, preset: null, groups: ["Memory"] }));
  });

  it("opens on every group and sends every name when nothing is changed", async () => {
    window.history.replaceState({}, "", URL);
    await mount("aurora");
    patchSnap({
      scenes: scenesWith([group("g-case", "Case fans"), group("g-ram", "Memory"), group("g-gpu", "Graphics card")]),
    });
    const play = await screen.findByRole("button", { name: "Play on …" });
    const spy = vi.spyOn(api, "activateEffect").mockResolvedValue({ ok: true });
    fireEvent.click(play);
    const dialog = await screen.findByRole("dialog", { name: "Play on" });
    expect(within(dialog).getByRole("checkbox", { name: "Every group" }).getAttribute("aria-checked")).toBe("true");
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Play" }));
    });
    await waitFor(() =>
      expect(spy).toHaveBeenCalledWith("aurora", {
        props: {},
        preset: null,
        groups: ["Case fans", "Memory", "Graphics card"],
      }),
    );
  });

  it("shows the daemon's message when it refuses a second HTML group", async () => {
    window.history.replaceState({}, "", URL);
    await mount("aurora");
    patchSnap({
      scenes: scenesWith([group("g-case", "Case fans"), group("g-ram", "Memory")]),
    });
    const spy = vi
      .spyOn(api, "activateEffect")
      .mockRejectedValue(
        new ApiFailure(422, "validation", "at most 1 group of a scene may play an HTML effect (a stack with an HTML layer counts); 2 do"),
      );
    const play = await screen.findByRole("button", { name: "Play on …" });
    fireEvent.click(play);
    const dialog = await screen.findByRole("dialog", { name: "Play on" });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Play" }));
    });
    await waitFor(() => expect(spy).toHaveBeenCalled());
    expect(
      (await screen.findByText(/at most 1 group of a scene may play an HTML effect/)).textContent,
    ).toContain("2 do");
  });

  it("marks a stack with an HTML layer as one HTML effect", async () => {
    window.history.replaceState({}, "", "/effects/?state=complete,stack");
    await act(async () => {
      await refreshAll();
    });
    patchSnap({
      status: {
        ...getSnap().status!,
        active_scene: { id: "s", name: "Scene" },
        active_effect: "stack:Sunset glow",
        active_effects: ["stack:Sunset glow"],
      },
      scenes: scenesWith([group("g-case", "Case fans"), group("g-ram", "Memory")]),
    });
    vi.spyOn(api, "stack").mockResolvedValue({
      stack: { id: "stack:Sunset glow", layers: [{ effect: "html:fixture-sample" }] },
      layers: [],
    } as never);
    render(
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>,
    );
    expect(await screen.findByRole("heading", { name: "Sunset glow" })).toBeTruthy();
    const play = (await screen.findAllByRole("button", { name: "Play on …" }))[0]!;
    fireEvent.click(play);
    const dialog = await screen.findByRole("dialog", { name: "Play on" });
    expect(await within(dialog).findByText("One HTML effect at a time")).toBeTruthy();
    expect(within(dialog).queryByRole("checkbox", { name: "Every group" })).toBeNull();
  });

  it("still plays on everything when the daemon has no scenes", async () => {
    window.history.replaceState({}, "", URL);
    await act(async () => {
      await refreshAll();
    });
    patchSnap({
      scenes: null,
      scenesError: "404 no scene route",
      status: {
        ...getSnap().status!,
        active_scene: null,
        active_effect: "aurora",
        active_effects: ["aurora"],
      },
    });
    render(
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>,
    );
    const spy = vi.spyOn(api, "activateEffect").mockResolvedValue({ ok: true });
    const play = await screen.findByRole("button", { name: "Play" });
    expect(play.getAttribute("aria-disabled")).toBeNull();
    await act(async () => {
      fireEvent.click(play);
    });
    await waitFor(() => expect(spy).toHaveBeenCalledWith("aurora", { props: {}, preset: null }));
  });

  it("an HTML effect in a two-group scene offers single groups only", async () => {
    window.history.replaceState({}, "", URL);
    await mount("html:fixture-sample");
    patchSnap({ scenes: scenesWith([group("g-case", "Case fans"), group("g-ram", "Memory")]) });
    const play = await screen.findByRole("button", { name: "Play on …" });
    fireEvent.click(play);
    const dialog = await screen.findByRole("dialog", { name: "Play on" });
    expect(within(dialog).queryByRole("checkbox", { name: "Every group" })).toBeNull();
    expect(within(dialog).getByText("One HTML effect at a time")).toBeTruthy();
    const spy = vi.spyOn(api, "activateEffect").mockResolvedValue({ ok: true });
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "Memory" }));
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Play" }));
    });
    await waitFor(() => expect(spy).toHaveBeenCalledWith("html:fixture-sample", {
      props: {},
      preset: null,
      groups: ["Memory"],
    }));
  });

  it("Effects controls call the defaults route", async () => {
    window.history.replaceState({}, "", URL);
    await mount("aurora");
    patchSnap({ scenes: scenesWith([group("g-case", "Case fans"), group("g-ram", "Memory")]) });
    const speed = await screen.findByLabelText("Speed value");
    const defaults = vi.spyOn(api, "patchEffectDefaults").mockResolvedValue({ ok: true });
    const active = vi.spyOn(api, "patchActiveProps").mockResolvedValue({ ok: true });
    const activate = vi.spyOn(api, "activateEffect").mockResolvedValue({ ok: true });
    fireEvent.focus(speed);
    fireEvent.change(speed, { target: { value: "55" } });
    fireEvent.blur(speed);
    await waitFor(
      () => expect(defaults).toHaveBeenCalledWith("aurora", { speed: { type: "number", value: 55 } }),
      { timeout: 3_000 },
    );
    expect(active).not.toHaveBeenCalled();
    expect(activate).not.toHaveBeenCalled();
    expect(screen.getByText("Starting values. A group keeps its own.")).toBeTruthy();
  });
});

// The helper below is unused on purpose: it documents the snapshot shape the page reads.
void (() => {
  const groups = [group("g", "G")];
  return scenesWith(groups);
});
