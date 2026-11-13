import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { effectMissing, effectNameOf, playing, readableEffectId } from "@/lib/status";
import { patchSnap } from "@/lib/store";
import { Rail } from "@/shell/Rail";
import { StatusStrip } from "@/shell/StatusStrip";
import { StripSlotsProvider } from "@/shell/PageHeader";
import { DEMO_SCENES } from "@/lib/fixtures";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { Status } from "@/proto/Status";

// With scenes every group plays its own effect, so the places that used to name "the" effect name
// the scene: the strip, the rail deck, the rail's Effects row. And a missing effect is never shown
// as its raw id.

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/",
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
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
});

afterEach(() => {
  cleanup();
});

const effect = (id: string, name: string): EffectSummary => ({ id, name }) as unknown as EffectSummary;
const EFFECTS = [effect("aurora", "Aurora"), effect("rainbow_wave", "Rainbow Wave")];

const status = (over: Partial<Status> = {}): Status => ({
  version: "0.14.0",
  power: "running",
  brightness: 100,
  fps_target: 30,
  fps_measured: 30,
  frame_time_ms: 33.3,
  frames_rendered: 0,
  parked: false,
  devices_connected: 12,
  devices_total: 12,
  preview_subscribers: 0,
  active_layout: null,
  active_effect: "aurora",
  active_effects: ["aurora", "rainbow_wave"],
  active_scene: { id: "gaming", name: "Gaming" },
  openrgb: "connected",
  service: "connected",
  service_down_ms: null,
  conflicts: [],
  show_zone_panel: false,
  onboarding_complete: true,
  ...over,
});

describe("an effect's name for a reader", () => {
  it("reads the effects list, and never the raw id", () => {
    expect(effectNameOf(EFFECTS, "aurora")).toBe("Aurora");
    expect(effectNameOf(EFFECTS, "html:glow-removed")).toBe("Missing effect");
    expect(effectMissing(EFFECTS, "html:glow-removed")).toBe(true);
    expect(effectMissing(EFFECTS, "aurora")).toBe(false);
  });

  it("gives a readable form while the list has not arrived, and is not 'missing' then", () => {
    expect(effectNameOf(null, "html:glow-removed")).toBe("Glow Removed");
    expect(effectMissing(null, "html:glow-removed")).toBe(false);
    expect(readableEffectId("html:glow-remo")).toBe("Glow Remo");
    expect(readableEffectId("rainbow_wave")).toBe("Rainbow Wave");
    expect(readableEffectId("html:")).toBe("Missing effect");
  });
});

describe("what is playing", () => {
  it("is the scene's name, with how many devices are lit", () => {
    expect(playing(status(), EFFECTS)).toEqual({ name: "Gaming", detail: "on 12 devices", count: true });
  });

  it("is the scene's name even when every group is off", () => {
    expect(playing(status({ active_effects: [] }), EFFECTS)).toMatchObject({ name: "Gaming", detail: "all off" });
  });

  it("says 'No scene yet', not 'No layout yet', when there is neither a scene nor a layout", () => {
    const none = playing(status({ active_scene: null, active_layout: null }), EFFECTS);
    expect(none.name).toBe("No scene yet");
    expect(none.name).not.toMatch(/layout/i);
  });

  it("names the effect for a daemon without scenes", () => {
    const old = status({ active_scene: null, active_layout: "desk" });
    expect(playing(old, EFFECTS).name).toBe("Aurora");
  });
});

describe("the rail with a scene", () => {
  const mountRail = () =>
    render(
      <StripSlotsProvider>
        <Rail pathname="/" />
      </StripSlotsProvider>,
    );

  it("titles the deck with the scene's name and its group count, not an effect", () => {
    patchSnap({
      status: status(),
      effects: EFFECTS,
      scenes: { active: "gaming", scenes: structuredClone(DEMO_SCENES.scenes), group_states: [] },
    });
    mountRail();
    const deck = screen.getByRole("region", { name: "The deck" });
    const row = deck.querySelector(".rail-deck-row") as HTMLElement;
    expect(row.textContent).toContain("Gaming");
    expect(row.textContent).toContain("4 groups");
    expect(row.textContent).not.toContain("Aurora");
  });

  it("does not show the playing effect on the Effects row", () => {
    patchSnap({ status: status(), effects: EFFECTS });
    mountRail();
    const link = screen.getByRole("link", { name: /Effects/ });
    expect(within(link).queryByText("Aurora")).toBeNull();
  });

  it("still shows the playing effect for a daemon without scenes", () => {
    patchSnap({ status: status({ active_scene: null, active_layout: "desk" }), effects: EFFECTS, scenes: null });
    mountRail();
    const link = screen.getByRole("link", { name: /Effects/ });
    expect(within(link).getByText("Aurora")).toBeTruthy();
  });
});

describe("the status strip", () => {
  it("names the scene, and says offline in words when the daemon is away", () => {
    patchSnap({ status: status(), effects: EFFECTS, statusError: null });
    const { unmount } = render(
      <StripSlotsProvider>
        <StatusStrip detailsOpen={false} onDetailsOpen={() => {}} />
      </StripSlotsProvider>,
    );
    expect(document.querySelector(".strip-effect-name")?.textContent).toBe("Gaming");
    unmount();
    patchSnap({ status: null, statusError: "refused" });
    render(
      <StripSlotsProvider>
        <StatusStrip detailsOpen={false} onDetailsOpen={() => {}} />
      </StripSlotsProvider>,
    );
    expect(document.querySelector(".strip-effect-name")?.textContent).toBe("Offline");
  });
});
