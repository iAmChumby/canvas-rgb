import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
    createPreset: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("@/components/Notify", () => ({ notify: vi.fn() }));

import { patchSnap } from "@/lib/store";
import { DEMO_LIGHTS } from "@/lib/fixtures";
import { EffectPicker, HTML_ONE_AT_A_TIME } from "@/views/scenes/EffectPicker";
import { useScene } from "@/views/scenes/useScene";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { EffectSummary } from "@/proto/EffectSummary";
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

/** The scene the fake daemon holds. `html` says the other group plays an HTML effect. */
function scene(html: boolean): Scene {
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
        members: [],
        plays: html
          ? { type: "effect", effect: "html:sample", props: {} }
          : { type: "off" },
        mode: "flow_across",
        brightness: 100,
      },
    ],
  };
}

function answer(html = false): ScenesResponse {
  const s = scene(html);
  return {
    active: s.id,
    scenes: [s],
    group_states: s.groups.map((g) => ({ id: g.id, failed: null })),
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
  { id: "aurora", name: "Aurora", description: "", needs: { screen: false, audio: false }, favorite: false },
  { id: "fire", name: "Fire", description: "", needs: { screen: false, audio: false }, favorite: false },
  { id: "html:sample", name: "Sample", description: "", needs: { screen: false, audio: false }, favorite: false },
];

let serverRev = 7;

beforeEach(() => {
  for (const group of Object.values(api.scenes)) group.mockReset();
  for (const fn of Object.values(api.lights)) fn.mockReset();
  api.effect.mockReset();
  serverRev = 7;
  api.scenes.list.mockResolvedValue(answer());
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

function Harness({ onUse }: { onUse: (use: ReturnType<typeof useScene>) => void }) {
  const use = useScene();
  useEffect(() => {
    onUse(use);
  }, [use, onUse]);
  return <EffectPicker groupId="g-one" use={use} onClose={() => {}} />;
}

/** The hook the picker is given, and the last of it, so a test can look at the undo stack. */
let latest: ReturnType<typeof useScene> | null = null;
const take = (use: ReturnType<typeof useScene>) => {
  latest = use;
};

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

describe("the effect picker", () => {
  it("highlighting an effect previews it and closing restores the old one", async () => {
    render(<Harness onUse={take} />);
    const tile = await screen.findByRole("button", { name: /Fire/ });
    fireEvent.pointerEnter(tile);
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalled());
    await waitFor(() => expect(lastPlays()).toEqual({ type: "effect", effect: "fire" }));
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(lastPlays()).toEqual({ type: "effect", effect: "aurora" }));
  });

  it("a preview is not on the undo stack: the owner's own choice is", async () => {
    render(<Harness onUse={take} />);
    const tile = await screen.findByRole("button", { name: /Fire/ });
    fireEvent.pointerEnter(tile);
    await waitFor(() => expect(api.scenes.put).toHaveBeenCalled());
    // The preview went to the daemon, but it is not a change the owner made.
    expect(latest?.canUndo).toBe(false);
    fireEvent.click(tile);
    await waitFor(() => expect(latest?.canUndo).toBe(true));
    expect(lastPlays()).toEqual({ type: "effect", effect: "fire" });
  });

  it("an HTML effect another group plays cannot be chosen, and says why", async () => {
    patchSnap({ scenes: answer(true) });
    api.scenes.list.mockResolvedValue(answer(true));
    render(<Harness onUse={take} />);
    const html = await screen.findByRole("button", { name: /Sample/ });
    expect(html.getAttribute("aria-disabled")).toBe("true");
    expect(screen.getByText(new RegExp(HTML_ONE_AT_A_TIME))).toBeTruthy();
    // Pointing at it does not preview it either.
    fireEvent.pointerEnter(html);
    await new Promise((r) => setTimeout(r, 30));
    expect(lastPlays()).toBeNull();
  });
});
