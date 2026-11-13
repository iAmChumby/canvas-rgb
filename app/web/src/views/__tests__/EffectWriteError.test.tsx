import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { EffectsView } from "@/views/EffectsView";
import { patchSnap } from "@/lib/store";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { Status } from "@/proto/Status";

// Issue #156 (Effects): a successful read of the effect detail clears the shared `error`
// slot, which also holds write errors (EffectsView.tsx flush/activate/toggleFavorite/
// deleteEffect). A read that lands after a failed write hides the write's error, so the
// change looks as if it went through. A read may clear only the read error.

const { api } = vi.hoisted(() => ({
  api: {
    effect: vi.fn(),
    patchEffectDefaults: vi.fn(),
    activateEffect: vi.fn(),
    setFavorite: vi.fn(),
    deleteEffect: vi.fn(),
    effectSource: vi.fn(),
    importEffect: vi.fn(),
    createPreset: vi.fn(),
    replacePreset: vi.fn(),
    deletePreset: vi.fn(),
    effectThumbnail: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));
// The store's refresh asks the daemon for every list; here it only has to succeed.
vi.mock("@/lib/store", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/store")>()),
  refreshAll: () => Promise.resolve(),
}));

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
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
});

afterEach(cleanup);

function detail(id: string): EffectDetail {
  return {
    schema: {
      id,
      name: id === "aurora" ? "Aurora" : "Other",
      description: "",
      needs: { screen: false, audio: false },
      properties: [{ key: "speed", label: "Speed", kind: { type: "number", min: 0, max: 100, step: 1, default: 30 } }],
      presets: [{ name: "Fast", props: { speed: { type: "number", value: 80 } } }],
    },
    favorite: false,
    user_presets: [],
    props: { speed: { type: "number", value: 30 } },
  };
}

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.effect.mockImplementation((id: string) => Promise.resolve(detail(id)));
  api.effectThumbnail.mockResolvedValue(new ArrayBuffer(0));
  patchSnap({
    effects: [
      { id: "aurora", name: "Aurora", description: "", needs: { screen: false, audio: false }, favorite: false },
      { id: "other", name: "Other", description: "", needs: { screen: false, audio: false }, favorite: false },
    ],
    effectsError: null,
    effectsFailed: [],
    status: {
      active_effects: ["aurora"],
      active_scene: { id: "s", name: "Scene" },
    } as unknown as Status,
    scenes: {
      active: "s",
      scenes: [
        {
          id: "s",
          name: "Scene",
          rev: 1,
          groups: [
            { id: "g", name: "Everything", members: [], plays: { type: "off" }, mode: "flow_across", brightness: 100 },
          ],
        },
      ],
      group_states: [],
    },
    pending: [],
  });
});

function page() {
  return render(
    <StripSlotsProvider>
      <StripSlot kind="title" />
      <EffectsView />
    </StripSlotsProvider>,
  );
}

describe("a read that lands after a failed write", () => {
  it("keeps the failed write's message on screen", async () => {
    api.patchEffectDefaults.mockRejectedValue(new Error("the effect did not take the change"));
    page();
    // The detail read has landed: the slider is on screen.
    const speed = await screen.findByLabelText("Speed value");

    // The user edits the value; the debounced write fails.
    fireEvent.focus(speed);
    fireEvent.change(speed, { target: { value: "55" } });
    fireEvent.blur(speed);
    await waitFor(() => expect(api.patchEffectDefaults).toHaveBeenCalledTimes(1));
    await screen.findByText("the effect did not take the change");

    // Another effect is opened: its detail read succeeds. Aurora's failure is not said on
    // Other: it is not Other's change that failed.
    fireEvent.click(screen.getByRole("button", { name: "Open Other" }));
    await screen.findByRole("heading", { name: "Other" });
    await screen.findByLabelText("Speed value");
    expect(screen.queryByText("the effect did not take the change")).toBeNull();

    // And back to the first: its detail read succeeds too.
    fireEvent.click(screen.getByRole("button", { name: "Open Aurora" }));
    await screen.findByRole("heading", { name: "Aurora" });
    await screen.findByLabelText("Speed value");

    // The write still did not go through: its message is still on screen.
    expect(screen.queryByText("the effect did not take the change")).toBeTruthy();
  });
});

describe("a failed favourite", () => {
  it("is cleared by the next favourite that goes through, not left on screen for good", async () => {
    api.setFavorite.mockRejectedValueOnce(new Error("the favourite was not saved"));
    api.setFavorite.mockResolvedValue(undefined);
    page();
    await screen.findByLabelText("Speed value");

    fireEvent.click(screen.getByRole("button", { name: "Favorite Other" }));
    await screen.findByText("the favourite was not saved");

    // The user tries again, and it goes through: the old failure is gone.
    fireEvent.click(screen.getByRole("button", { name: "Favorite Other" }));
    await waitFor(() => expect(api.setFavorite).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByText("the favourite was not saved")).toBeNull());
  });
});

describe("an edit after a failed write", () => {
  it("clears that effect's failure: the edit replaces what the failed write asked for", async () => {
    api.patchEffectDefaults.mockRejectedValueOnce(new Error("the effect did not take the change"));
    // The retry stays out, so only the edit itself can have cleared the message.
    api.patchEffectDefaults.mockReturnValue(new Promise(() => {}));
    page();
    const speed = await screen.findByLabelText("Speed value");
    fireEvent.focus(speed);
    fireEvent.change(speed, { target: { value: "55" } });
    fireEvent.blur(speed);
    await screen.findByText("the effect did not take the change");

    fireEvent.focus(speed);
    fireEvent.change(speed, { target: { value: "60" } });
    fireEvent.blur(speed);
    await waitFor(() => expect(screen.queryByText("the effect did not take the change")).toBeNull());
  });
});

describe("other ways a failure is left behind", () => {
  it("a preset that goes on clears the effect's failed change", async () => {
    api.patchEffectDefaults.mockRejectedValueOnce(new Error("the effect did not take the change"));
    api.activateEffect.mockResolvedValue(undefined);
    page();
    const speed = await screen.findByLabelText("Speed value");
    fireEvent.focus(speed);
    fireEvent.change(speed, { target: { value: "55" } });
    fireEvent.blur(speed);
    await screen.findByText("the effect did not take the change");

    fireEvent.click(screen.getByRole("button", { name: /^Apply preset Fast/ }));
    await waitFor(() => expect(api.activateEffect).toHaveBeenCalled());
    await waitFor(() => expect(screen.queryByText("the effect did not take the change")).toBeNull());
  });

  it("a failed favourite goes when the user opens another effect", async () => {
    api.setFavorite.mockRejectedValue(new Error("the favourite was not saved"));
    page();
    await screen.findByLabelText("Speed value");
    fireEvent.click(screen.getByRole("button", { name: "Favorite Other" }));
    await screen.findByText("the favourite was not saved");

    fireEvent.click(screen.getByRole("button", { name: "Open Other" }));
    await screen.findByRole("heading", { name: "Other" });
    expect(screen.queryByText("the favourite was not saved")).toBeNull();
  });
});


