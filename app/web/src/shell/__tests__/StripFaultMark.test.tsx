import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { patchSnap } from "@/lib/store";
import { StatusStrip } from "@/shell/StatusStrip";
import { StripSlotsProvider } from "@/shell/PageHeader";
import type { Fault } from "@/proto/Fault";
import type { Status } from "@/proto/Status";

// #580 fix pass C1, C2: the count is not a summary that hides the first problem, and a dismissed
// fault says so in the mark and offers a way back.

const showAgain = vi.fn(async (_key?: string) => undefined);
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/",
}));
vi.mock("@/lib/endpoints", () => ({
  api: { patchOutput: vi.fn(async () => undefined), showFaultAgain: (key: string) => showAgain(key) },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const phoneRef = { current: false };
beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: phoneRef.current && query.includes("max-width: 720px"),
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
  showAgain.mockClear();
  phoneRef.current = false;
});

const fault = (key: string, dismissed = false): Fault => ({
  kind: "fan_chip",
  key,
  affected: `Fan note ${key}`,
  fix: "Install PawnIO.",
  dismissed,
});

function status(over: Partial<Status> = {}): Status {
  return {
    version: "0.20.0",
    power: "running",
    brightness: 100,
    fps_target: 30,
    fps_measured: 30,
    frame_time_ms: 33.3,
    frames_rendered: 0,
    parked: false,
    devices_connected: 0,
    devices_total: 0,
    preview_subscribers: 0,
    active_effects: [],
    active_scene: null,
    openrgb: "connected",
    service: "connected",
    service_down_ms: null,
    conflicts: [],
    show_zone_panel: false,
    onboarding_complete: true,
    faults: [],
    ...over,
  } as unknown as Status;
}

function openStrip() {
  patchSnap({ statusError: null, statusFault: null, effects: [], scenes: null, scenesError: null });
  return render(
    <StripSlotsProvider>
      <StatusStrip detailsOpen onDetailsOpen={() => undefined} />
    </StripSlotsProvider>,
  );
}

function mark(): HTMLElement {
  return document.querySelector(".strip-mark") as HTMLElement;
}

function problemText(): string {
  return document.querySelector(".strip-details-problems")?.textContent ?? "";
}

describe("the count lists every problem (C1)", () => {
  it("shows the one fault in the popover as well as the mark", () => {
    patchSnap({ status: status({ faults: [fault("1111111111111111")] }) });
    openStrip();
    expect(mark().textContent).toContain("1 warning");
    expect(problemText()).toContain("Fan note 1111111111111111");
  });

  it("lists both faults and still names the mark's count", () => {
    patchSnap({ status: status({ faults: [fault("1111111111111111"), fault("2222222222222222")] }) });
    openStrip();
    expect(mark().textContent).toContain("2 warnings");
    expect(problemText()).toContain("Fan note 1111111111111111");
    expect(problemText()).toContain("Fan note 2222222222222222");
  });

  it("keeps Safe start visible in the popover beside a fault", () => {
    patchSnap({
      status: status({ safe_start: { active: true, unclean_streak: 1 }, faults: [fault("1111111111111111")] }),
    });
    openStrip();
    expect(mark().textContent).toContain("1 warning");
    expect(problemText()).toContain("Safe start");
    expect(problemText()).toContain("Fan note 1111111111111111");
  });

  it("makes the accessible name match the visible count", () => {
    patchSnap({ status: status({ faults: [fault("1111111111111111")] }) });
    openStrip();
    expect(mark().getAttribute("aria-label")).toBe(mark().textContent);
    expect(mark().getAttribute("title")).toBe(mark().textContent);
  });
});

describe("a dismissed fault says so and can be shown again (C2)", () => {
  it("marks the count as dismissed and offers Show again", () => {
    patchSnap({ status: status({ faults: [fault("1111111111111111", true)] }) });
    openStrip();
    expect(mark().textContent).toContain("1 warning, dismissed");
    expect(mark().getAttribute("aria-label")).toBe(mark().textContent);
    expect(screen.getByRole("button", { name: "Show again" })).toBeTruthy();
    screen.getByRole("button", { name: "Show again" }).click();
    expect(showAgain).toHaveBeenCalledWith("1111111111111111");
  });

  it("never reads All connected over an active dismissed fault", () => {
    patchSnap({ status: status({ faults: [fault("1111111111111111", true)] }) });
    openStrip();
    expect(mark().textContent).not.toContain("All connected");
  });

  it("keeps the full words and the dismissed mark on a phone", () => {
    phoneRef.current = true;
    patchSnap({ status: status({ faults: [fault("1111111111111111", true)] }) });
    openStrip();
    const m = mark();
    expect(m.textContent).toBe("1");
    expect(m.getAttribute("aria-label")).toBe("1 warning, dismissed");
    expect(m.getAttribute("title")).toBe("1 warning, dismissed");
    expect(m.getAttribute("data-dismissed")).toBe("true");
  });
});
