import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

// A later launch with the onboarding unfinished (#579): the page is loaded with the hello already
// seen, so it opens on Scenes and the checklist with "Welcome back", never on the hello. This is
// the issue's "resumes after a restart".

vi.hoisted(() => {
  window.localStorage.setItem("canvas-rgb.hello-seen", "1");
});

const { nav } = vi.hoisted(() => ({ nav: { replace: vi.fn() } }));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => {}, replace: nav.replace }),
  usePathname: () => "/fans/",
}));

import { patchSnap } from "@/lib/store";
import { shouldOpen } from "@/views/editor/flow";
import { OnboardingGate } from "@/views/editor/OnboardingGate";
import { SetupLayout } from "@/views/setup/SetupLayout";
import type { Status } from "@/proto/Status";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
});

const status = (over: Partial<Status> = {}): Status =>
  ({
    devices_connected: 3,
    devices_total: 3,
    conflicts: [],
    openrgb: "connected",
    service: "down",
    active_scene: { id: "main", name: "Main" },
    active_effects: ["aurora"],
    onboarding_complete: false,
    ...over,
  }) as Status;

const mount = () =>
  render(
    <SetupLayout onTroubleshoot={() => {}}>
      <p>the page</p>
    </SetupLayout>,
  );

afterEach(() => {
  cleanup();
  patchSnap({ status: null });
});

describe("a later launch with the onboarding unfinished", () => {
  it("opens on Scenes and not on the hello", async () => {
    expect(shouldOpen(status())).toBe(false);
    patchSnap({ status: status() });
    await act(async () => {
      render(<OnboardingGate />);
    });
    expect(nav.replace).toHaveBeenCalledWith("/");
  });

  it("says Welcome back while something is done", () => {
    patchSnap({ status: status() });
    mount();
    expect(screen.getByRole("heading", { name: "Welcome back" })).toBeTruthy();
  });

  it("does not say Welcome back with nothing done", () => {
    patchSnap({
      status: status({ devices_connected: 0, active_scene: null, active_effects: [] }),
    });
    mount();
    expect(screen.getByRole("heading", { name: "Setup checklist" })).toBeTruthy();
  });
});
